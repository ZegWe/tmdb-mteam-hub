use sha1::{Digest, Sha1};

const TORRENT_HASH_HTTP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) tmdb-mteam-hub/0.1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentDownloadInfo {
    pub info_hashes: Vec<String>,
    pub torrent_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentHashError {
    message: String,
}

impl TorrentHashError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TorrentHashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TorrentHashError {}

pub fn magnet_info_hash_hex(url: &str) -> Option<String> {
    let raw = url.trim();
    if !raw.to_ascii_lowercase().starts_with("magnet:?") {
        return None;
    }
    let query = raw.split_once('?')?.1;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key != "xt" {
            continue;
        }
        let value = percent_decode_component(value);
        let prefix = "urn:btih:";
        if value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix) {
            let hash = value[prefix.len()..].trim();
            if hash.len() == 40 && hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Some(hash.to_ascii_lowercase());
            }
        }
    }
    None
}

pub fn torrent_info_hash_hex_from_bytes(bytes: &[u8]) -> Result<String, TorrentHashError> {
    let (start, end) = torrent_info_span(bytes)?;
    let digest = Sha1::digest(&bytes[start..end]);
    Ok(format!("{digest:x}"))
}

pub async fn torrent_download_info_from_url(
    url: &str,
) -> Result<TorrentDownloadInfo, TorrentHashError> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(TorrentDownloadInfo {
            info_hashes: Vec::new(),
            torrent_bytes: None,
        });
    }
    if let Some(hash) = magnet_info_hash_hex(url) {
        return Ok(TorrentDownloadInfo {
            info_hashes: vec![hash],
            torrent_bytes: None,
        });
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Ok(TorrentDownloadInfo {
            info_hashes: Vec::new(),
            torrent_bytes: None,
        });
    }

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .user_agent(TORRENT_HASH_HTTP_UA)
        .redirect(reqwest::redirect::Policy::limited(15))
        .build()
        .map_err(|e| TorrentHashError::new(format!("构建种子下载客户端失败: {e}")))?;
    let response = client
        .get(url)
        .header("Accept", "application/x-bittorrent,*/*")
        .header("Referer", "https://kp.m-team.cc/")
        .send()
        .await
        .map_err(|e| TorrentHashError::new(format!("下载种子以计算 hash 失败: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(TorrentHashError::new(format!(
            "下载种子以计算 hash 返回 HTTP {status}"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| TorrentHashError::new(format!("读取种子内容失败: {e}")))?;
    let bytes = bytes.to_vec();
    Ok(TorrentDownloadInfo {
        info_hashes: vec![torrent_info_hash_hex_from_bytes(&bytes)?],
        torrent_bytes: Some(bytes),
    })
}

fn torrent_info_span(bytes: &[u8]) -> Result<(usize, usize), TorrentHashError> {
    if bytes.first() != Some(&b'd') {
        return Err(TorrentHashError::new("种子文件根节点不是 bencode 字典"));
    }
    let mut pos = 1;
    while pos < bytes.len() {
        if bytes[pos] == b'e' {
            return Err(TorrentHashError::new("种子文件缺少 info 字典"));
        }
        let key_start = pos;
        let key_end = bencode_value_end(bytes, key_start)?;
        let key = bencode_bytes_value(bytes, key_start, key_end)?;
        let value_start = key_end;
        let value_end = bencode_value_end(bytes, value_start)?;
        if key == b"info" {
            return Ok((value_start, value_end));
        }
        pos = value_end;
    }
    Err(TorrentHashError::new("种子文件 bencode 字典未正常结束"))
}

fn bencode_value_end(bytes: &[u8], pos: usize) -> Result<usize, TorrentHashError> {
    let Some(first) = bytes.get(pos).copied() else {
        return Err(TorrentHashError::new("bencode 数据提前结束"));
    };
    match first {
        b'i' => bytes[pos + 1..]
            .iter()
            .position(|b| *b == b'e')
            .map(|offset| pos + 1 + offset + 1)
            .ok_or_else(|| TorrentHashError::new("bencode 整数未结束")),
        b'l' => {
            let mut next = pos + 1;
            loop {
                match bytes.get(next).copied() {
                    Some(b'e') => return Ok(next + 1),
                    Some(_) => next = bencode_value_end(bytes, next)?,
                    None => return Err(TorrentHashError::new("bencode 列表未结束")),
                }
            }
        }
        b'd' => {
            let mut next = pos + 1;
            loop {
                match bytes.get(next).copied() {
                    Some(b'e') => return Ok(next + 1),
                    Some(_) => {
                        next = bencode_value_end(bytes, next)?;
                        next = bencode_value_end(bytes, next)?;
                    }
                    None => return Err(TorrentHashError::new("bencode 字典未结束")),
                }
            }
        }
        b'0'..=b'9' => {
            let colon = bytes[pos..]
                .iter()
                .position(|b| *b == b':')
                .map(|offset| pos + offset)
                .ok_or_else(|| TorrentHashError::new("bencode 字节串缺少长度分隔符"))?;
            let len = std::str::from_utf8(&bytes[pos..colon])
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or_else(|| TorrentHashError::new("bencode 字节串长度无效"))?;
            colon
                .checked_add(1)
                .and_then(|start| start.checked_add(len))
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| TorrentHashError::new("bencode 字节串超过数据长度"))
        }
        _ => Err(TorrentHashError::new("无法解析 bencode 值")),
    }
}

fn bencode_bytes_value(bytes: &[u8], start: usize, end: usize) -> Result<&[u8], TorrentHashError> {
    let colon = bytes[start..end]
        .iter()
        .position(|b| *b == b':')
        .map(|offset| start + offset)
        .ok_or_else(|| TorrentHashError::new("bencode 字典键不是字节串"))?;
    let len = std::str::from_utf8(&bytes[start..colon])
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .ok_or_else(|| TorrentHashError::new("bencode 字典键长度无效"))?;
    let value_start = colon + 1;
    let value_end = value_start + len;
    if value_end != end {
        return Err(TorrentHashError::new("bencode 字典键长度不匹配"));
    }
    Ok(&bytes[value_start..value_end])
}

fn percent_decode_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrent_info_hash_uses_raw_info_dictionary_bytes() {
        let info =
            b"d4:name4:test12:piece lengthi16384e6:lengthi12345e6:pieces20:abcdefghijklmnopqrste";
        let torrent = [
            b"d8:announce15:https://tracker".as_slice(),
            b"4:info".as_slice(),
            info.as_slice(),
            b"e".as_slice(),
        ]
        .concat();

        let hash = torrent_info_hash_hex_from_bytes(&torrent).unwrap();

        assert_eq!(hash, "15a4d0049ad4708d1504650b6c70b90320aaf3e1");
    }

    #[test]
    fn magnet_info_hash_accepts_btih_xt() {
        let hash = magnet_info_hash_hex(
            "magnet:?dn=test&xt=urn:btih:AAEBC806FDBF63111B1F7DDE3A89BC17D2988686",
        )
        .unwrap();

        assert_eq!(hash, "aaebc806fdbf63111b1f7dde3a89bc17d2988686");
    }

    #[tokio::test]
    async fn info_hash_candidates_from_url_uses_magnet_without_http_fetch() {
        let info = torrent_download_info_from_url(
            "magnet:?xt=urn:btih:AAEBC806FDBF63111B1F7DDE3A89BC17D2988686",
        )
        .await
        .unwrap();

        assert_eq!(
            info.info_hashes,
            vec!["aaebc806fdbf63111b1f7dde3a89bc17d2988686".to_string()]
        );
        assert!(info.torrent_bytes.is_none());
    }
}
