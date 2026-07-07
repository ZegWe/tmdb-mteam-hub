use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::cookie::{CookieStore, Jar};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DESKTOP_CHROME_UA: &str = concat!(
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 ",
    "(KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36"
);
const AUTH_COOKIE_NAME: &str = "dbcl2";
const SUBJECT_URL_PREFIX: &str = "https://movie.douban.com/subject/";
const SUBJECT_INTEREST_URL_PREFIX: &str = "https://movie.douban.com/j/subject/";
const REXXAR_SEARCH_URL: &str = "https://m.douban.com/rexxar/api/v2/search/subjects";
const REXXAR_MOVIE_URL_PREFIX: &str = "https://m.douban.com/rexxar/api/v2/movie/";
const REXXAR_MOVIE_REFERER_PREFIX: &str = "https://m.douban.com/movie/subject/";
const MOVIE_BASE_URL: &str = "https://movie.douban.com/";
const MINE_URL: &str = "https://movie.douban.com/mine";
const QR_CODE_URL: &str = "https://accounts.douban.com/j/mobile/login/qrlogin_code";
const QR_STATUS_URL: &str = "https://accounts.douban.com/j/mobile/login/qrlogin_status";
const LOGIN_REFERER: &str = "https://accounts.douban.com/passport/login";
const IMAGE_REFERER: &str = "https://movie.douban.com/";
const LIBRARY_PAGE_SIZE: usize = 15;
const LIBRARY_MAX_PAGES: usize = 80;

static QR_SESSION_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct DoubanError {
    pub status: StatusCode,
    pub message: String,
}

impl DoubanError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn upstream(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DoubanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}

impl std::error::Error for DoubanError {}

#[derive(Debug, Clone)]
pub struct QrSession {
    pub code: String,
    pub client: Client,
    pub jar: Arc<Jar>,
    pub image: Arc<Vec<u8>>,
}

#[derive(Debug, Serialize)]
pub struct QrStartResult {
    pub session_id: String,
    pub image_url: String,
}

#[derive(Debug, Serialize)]
pub struct QrPollResult {
    pub done: bool,
    pub login_status: String,
    pub message: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookie_header: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DoubanRating {
    pub value: Option<f64>,
    pub count: Option<u64>,
    pub info: String,
    pub star_count: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DoubanSearchResult {
    pub source: &'static str,
    pub media_type: &'static str,
    pub id: String,
    pub subject_id: String,
    pub title: String,
    pub url: String,
    pub abstract_text: String,
    pub abstract_2: String,
    pub cover_url: String,
    pub poster_url: String,
    pub rating: DoubanRating,
    pub vote_average: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct DoubanSubjectDetail {
    pub source: &'static str,
    pub media_type: &'static str,
    pub id: String,
    pub subject_id: String,
    pub url: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub original_title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aka: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub countries: Vec<String>,
    pub image: String,
    pub poster_url: String,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    pub actors: Vec<String>,
    pub genres: Vec<String>,
    pub date_published: String,
    pub duration: String,
    pub summary: String,
    pub rating: DoubanRating,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_interest: Option<DoubanInterest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_rating: Option<u8>,
}

#[derive(Debug, Copy, Clone)]
pub enum DoubanLibraryStatus {
    Wish,
    Collect,
}

impl DoubanLibraryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wish => "wish",
            Self::Collect => "collect",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Wish => "想看",
            Self::Collect => "看过",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DoubanLibraryList {
    pub status: &'static str,
    pub label: &'static str,
    pub items: Vec<DoubanLibraryItem>,
}

#[derive(Debug, Serialize)]
pub struct DoubanLibraryItem {
    pub source: &'static str,
    pub media_type: &'static str,
    pub id: String,
    pub subject_id: String,
    pub title: String,
    pub url: String,
    pub abstract_text: String,
    pub abstract_2: String,
    pub cover_url: String,
    pub poster_url: String,
    pub status: &'static str,
    pub status_label: &'static str,
    pub date: String,
    pub comment: String,
    pub tags: Vec<String>,
    pub user_rating: Option<u8>,
}

#[derive(Debug, Copy, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoubanInterest {
    Wish,
    Collect,
}

impl DoubanInterest {
    fn as_str(self) -> &'static str {
        match self {
            Self::Wish => "wish",
            Self::Collect => "collect",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DoubanInterestResult {
    pub ok: bool,
    pub interest: &'static str,
    pub rating: Option<u8>,
    pub tags: String,
}

#[derive(Deserialize)]
struct DoubanInterestResponse {
    r: Option<i64>,
    #[serde(default)]
    msg: String,
}

#[derive(Deserialize)]
struct QrCodeResponse {
    payload: Option<QrCodePayload>,
}

#[derive(Deserialize)]
struct QrCodePayload {
    code: Option<String>,
    img: Option<String>,
}

#[derive(Deserialize)]
struct QrStatusResponse {
    #[serde(default)]
    message: String,
    #[serde(default)]
    description: String,
    payload: Option<QrStatusPayload>,
}

#[derive(Deserialize)]
struct QrStatusPayload {
    #[serde(default)]
    login_status: String,
}

pub fn normalize_cookie_header(raw: &str) -> String {
    let mut value = raw.trim();
    if let Some(rest) = value
        .strip_prefix("Cookie:")
        .or_else(|| value.strip_prefix("cookie:"))
    {
        value = rest.trim();
    }
    value
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn has_auth_cookie(cookie_header: &str) -> bool {
    cookie_header
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .any(|(name, value)| name.trim() == AUTH_COOKIE_NAME && !value.trim().is_empty())
}

pub fn require_auth_cookie(cookie_header: &str) -> Result<String, DoubanError> {
    let normalized = normalize_cookie_header(cookie_header);
    if normalized.is_empty() {
        return Err(DoubanError::bad_request(
            "请先在设置中填写豆瓣 Cookie，或用 QR 登录自动获取",
        ));
    }
    if !has_auth_cookie(&normalized) {
        return Err(DoubanError::bad_request(format!(
            "豆瓣 Cookie 中缺少 {AUTH_COOKIE_NAME}，请重新填写或 QR 登录"
        )));
    }
    Ok(normalized)
}

pub fn auth_cache_key_fragment(cookie_header: &str) -> Result<String, DoubanError> {
    let normalized = require_auth_cookie(cookie_header)?;
    let Some(value) = cookie_value(&normalized, AUTH_COOKIE_NAME) else {
        return Ok("current".to_string());
    };
    let account_id = value
        .trim_matches('"')
        .split(':')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();
    if account_id.is_empty() {
        Ok("current".to_string())
    } else {
        Ok(account_id)
    }
}

fn cookie_value(cookie_header: &str, name: &str) -> Option<String> {
    cookie_header
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(cookie_name, value)| {
            (cookie_name.trim() == name).then(|| value.trim().to_string())
        })
}

fn extract_ck_from_html(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut offset = 0usize;
    while let Some(rel) = lower[offset..].find("<input") {
        let start = offset + rel;
        let Some(tag_end_rel) = html[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end_rel + 1;
        let tag = &html[start..tag_end];
        if attr_value(tag, "name").as_deref() == Some("ck") {
            if let Some(value) = attr_value(tag, "value").filter(|s| !s.is_empty()) {
                return Some(value);
            }
        }
        offset = tag_end;
    }

    for marker in ["ck=", "ck%3D"] {
        if let Some(idx) = html.find(marker) {
            let start = idx + marker.len();
            let value = html[start..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
                .collect::<String>();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

pub async fn qr_start() -> Result<(String, QrSession, QrStartResult), DoubanError> {
    let jar = Arc::new(Jar::default());
    let client = Client::builder()
        .use_rustls_tls()
        .cookie_provider(jar.clone())
        .build()
        .map_err(|e| DoubanError::upstream(format!("创建豆瓣登录客户端失败: {e}")))?;

    let code_json: QrCodeResponse = request_json(&client, QR_CODE_URL).await?;
    let payload = code_json
        .payload
        .ok_or_else(|| DoubanError::upstream("豆瓣 QR code 响应缺少 payload"))?;
    let code = payload
        .code
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| DoubanError::upstream("豆瓣 QR code 响应缺少 code"))?;
    let image_url = payload
        .img
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| DoubanError::upstream("豆瓣 QR code 响应缺少 img"))?
        .replace("\\/", "/");

    let image = client
        .get(&image_url)
        .header("User-Agent", DESKTOP_CHROME_UA)
        .header("Accept", "image/png,image/*,*/*;q=0.8")
        .header("Referer", LOGIN_REFERER)
        .send()
        .await
        .map_err(|e| DoubanError::upstream(format!("下载豆瓣 QR 图片失败: {e}")))?;
    let status = image.status();
    let bytes = image
        .bytes()
        .await
        .map_err(|e| DoubanError::upstream(format!("读取豆瓣 QR 图片失败: {e}")))?;
    if !status.is_success() || bytes.is_empty() {
        return Err(DoubanError::upstream(format!(
            "豆瓣 QR 图片下载失败: HTTP {status}"
        )));
    }

    let session_id = new_session_id();
    let result = QrStartResult {
        session_id: session_id.clone(),
        image_url: format!("/api/douban/qr/image?session_id={session_id}"),
    };
    let session = QrSession {
        code,
        client,
        jar,
        image: Arc::new(bytes.to_vec()),
    };
    Ok((session_id, session, result))
}

pub async fn qr_poll(session: &QrSession) -> Result<QrPollResult, DoubanError> {
    let status_url = Url::parse_with_params(QR_STATUS_URL, &[("code", session.code.as_str())])
        .map_err(|e| DoubanError::upstream(format!("构造豆瓣 QR 状态 URL 失败: {e}")))?;
    let status_json: QrStatusResponse = request_json(&session.client, status_url.as_str()).await?;
    let login_status = status_json
        .payload
        .map(|p| p.login_status)
        .unwrap_or_default();
    let cookie_header = jar_cookie_header(&session.jar)?;
    let done = has_auth_cookie(&cookie_header);
    Ok(QrPollResult {
        done,
        login_status,
        message: status_json.message,
        description: status_json.description,
        cookie_header: done.then_some(cookie_header),
    })
}

pub async fn search(
    cookie_header: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<DoubanSearchResult>, DoubanError> {
    if query.trim().is_empty() {
        return Err(DoubanError::bad_request("搜索关键字不能为空"));
    }
    if limit < 1 {
        return Err(DoubanError::bad_request("limit 必须大于 0"));
    }
    let cookie = normalize_cookie_header(cookie_header);
    let count = limit.to_string();
    let url = rexxar_search_url(query.trim(), &count)?;
    let referer = format!(
        "https://m.douban.com/search/?query={}",
        percent_encode_query_component(query.trim())
    );
    let data = fetch_rexxar_json(url, &cookie, &referer).await?;
    search_results_from_rexxar_json(&data, limit)
}

fn rexxar_search_url(query: &str, count: &str) -> Result<Url, DoubanError> {
    let mut url = Url::parse(REXXAR_SEARCH_URL)
        .map_err(|e| DoubanError::upstream(format!("构造豆瓣 rexxar 搜索 URL 失败: {e}")))?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("start", "0")
        .append_pair("count", count);
    Ok(url)
}

pub async fn subject_detail(
    cookie_header: &str,
    subject: &str,
) -> Result<DoubanSubjectDetail, DoubanError> {
    let subject_id = subject_id(subject)?;
    let cookie = normalize_cookie_header(cookie_header);
    let data = fetch_rexxar_subject(&subject_id, &cookie).await?;
    let mut detail = subject_detail_from_rexxar_json(&subject_id, &data)?;
    if has_auth_cookie(&cookie) {
        if let Ok(html) = fetch_html(&subject_url(&subject_id), &cookie, MOVIE_BASE_URL).await {
            let (user_interest, user_rating) = subject_user_interest_from_html(&html);
            if user_interest.is_some() {
                detail.user_interest = user_interest;
                detail.user_rating = user_rating.or(detail.user_rating);
            }
        }
    }
    Ok(detail)
}

pub async fn subject_detail_rexxar(
    cookie_header: &str,
    subject: &str,
) -> Result<DoubanSubjectDetail, DoubanError> {
    let subject_id = subject_id(subject)?;
    let cookie = normalize_cookie_header(cookie_header);
    let data = fetch_rexxar_subject(&subject_id, &cookie).await?;
    subject_detail_from_rexxar_json(&subject_id, &data)
}

async fn fetch_rexxar_subject(subject_id: &str, cookie_header: &str) -> Result<Value, DoubanError> {
    let mut url = Url::parse(&format!("{REXXAR_MOVIE_URL_PREFIX}{subject_id}"))
        .map_err(|e| DoubanError::upstream(format!("构造豆瓣 rexxar 详情 URL 失败: {e}")))?;
    url.query_pairs_mut()
        .append_pair("ck", "")
        .append_pair("for_mobile", "1");
    let referer = format!("{REXXAR_MOVIE_REFERER_PREFIX}{subject_id}/");
    fetch_rexxar_json(url, cookie_header, &referer).await
}

async fn fetch_rexxar_json(
    url: Url,
    cookie_header: &str,
    referer: &str,
) -> Result<Value, DoubanError> {
    let client = Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| DoubanError::upstream(format!("创建豆瓣 rexxar 客户端失败: {e}")))?;
    let mut req = client
        .get(url)
        .header("User-Agent", DESKTOP_CHROME_UA)
        .header("Accept", "application/json")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Origin", "https://m.douban.com")
        .header("Referer", referer);
    if !cookie_header.trim().is_empty() {
        req = req.header("Cookie", cookie_header);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| DoubanError::upstream(format!("豆瓣 rexxar 请求失败: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| DoubanError::upstream(format!("读取豆瓣 rexxar 响应失败: {e}")))?;
    if !status.is_success() {
        return Err(DoubanError::upstream(format!(
            "豆瓣 rexxar 接口 HTTP {status}: {}",
            rexxar_error_message(&text).unwrap_or_else(|| text.chars().take(120).collect())
        )));
    }
    serde_json::from_str::<Value>(&text).map_err(|e| {
        DoubanError::upstream(format!(
            "解析豆瓣 rexxar JSON 失败: {e}: {}",
            text.chars().take(120).collect::<String>()
        ))
    })
}

fn rexxar_error_message(text: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    value_to_string(value.get("localized_message"))
        .filter(|s| !s.is_empty())
        .or_else(|| value_to_string(value.get("msg")))
}

fn search_results_from_rexxar_json(
    data: &Value,
    limit: usize,
) -> Result<Vec<DoubanSearchResult>, DoubanError> {
    let items = data
        .get("subjects")
        .and_then(|v| v.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            DoubanError::upstream("豆瓣 rexxar 搜索结果结构异常: 缺少 subjects.items")
        })?;
    let mut out = Vec::new();
    for item in items {
        if value_to_string(item.get("layout")).as_deref() != Some("subject") {
            continue;
        }
        let Some(target) = item.get("target") else {
            continue;
        };
        let subject_id = value_to_string(target.get("id"))
            .or_else(|| value_to_string(item.get("target_id")))
            .unwrap_or_default();
        if subject_id.is_empty() {
            continue;
        }
        let title = value_to_string(target.get("title")).unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let cover_url = best_image_url_from_value(target)
            .as_deref()
            .and_then(proxied_image_url)
            .unwrap_or_default();
        let rating = rating_from_value(target.get("rating"));
        let vote_average = rating.value;
        out.push(DoubanSearchResult {
            source: "douban",
            media_type: "douban",
            id: subject_id.clone(),
            subject_id: subject_id.clone(),
            title,
            url: value_to_string(target.get("url"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| subject_url(&subject_id)),
            abstract_text: value_to_string(target.get("card_subtitle"))
                .or_else(|| value_to_string(target.get("abstract")))
                .unwrap_or_default(),
            abstract_2: value_to_string(target.get("year")).unwrap_or_default(),
            poster_url: cover_url.clone(),
            cover_url,
            rating,
            vote_average,
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn subject_detail_from_rexxar_json(
    subject_id: &str,
    data: &Value,
) -> Result<DoubanSubjectDetail, DoubanError> {
    let title = value_to_string(data.get("title"))
        .filter(|s| !s.is_empty())
        .or_else(|| value_to_string(data.get("original_title")).filter(|s| !s.is_empty()))
        .unwrap_or_default();
    if title.is_empty() {
        return Err(DoubanError::upstream("无法从豆瓣 rexxar 详情解析标题"));
    }
    let image = best_image_url_from_value(data)
        .as_deref()
        .and_then(proxied_image_url)
        .unwrap_or_default();
    let rating = rating_from_value(data.get("rating"));
    let (user_interest, user_rating) = rexxar_user_interest(data.get("interest"));
    Ok(DoubanSubjectDetail {
        source: "douban",
        media_type: "douban",
        id: subject_id.to_string(),
        subject_id: subject_id.to_string(),
        url: value_to_string(data.get("url"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| subject_url(subject_id)),
        title,
        original_title: value_to_string(data.get("original_title")).unwrap_or_default(),
        aka: string_list(data.get("aka")),
        languages: string_list(data.get("languages")),
        countries: string_list(data.get("countries")),
        poster_url: image.clone(),
        image,
        directors: people_names(data.get("directors")),
        writers: people_names(data.get("writers")),
        actors: people_names(data.get("actors")),
        genres: string_list(data.get("genres")),
        date_published: value_to_string(data.get("release_date"))
            .filter(|s| !s.is_empty())
            .or_else(|| first_string_from_value(data.get("pubdate")))
            .unwrap_or_default(),
        duration: first_string_from_value(data.get("durations")).unwrap_or_default(),
        summary: value_to_string(data.get("intro")).unwrap_or_default(),
        rating,
        user_interest,
        user_rating,
    })
}

pub async fn library(
    cookie_header: &str,
    status: DoubanLibraryStatus,
    limit: usize,
) -> Result<DoubanLibraryList, DoubanError> {
    let cookie = require_auth_cookie(cookie_header)?;
    let limit = limit.clamp(1, LIBRARY_PAGE_SIZE * LIBRARY_MAX_PAGES);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut start = 0usize;

    for _ in 0..LIBRARY_MAX_PAGES {
        let url = library_page_url(status, start)?;
        let html = fetch_html(url.as_str(), &cookie, "https://movie.douban.com/").await?;
        let page_items = extract_library_items(&html, status);
        if page_items.is_empty() {
            break;
        }

        let before = out.len();
        for item in page_items {
            if seen.insert(item.subject_id.clone()) {
                out.push(item);
                if out.len() >= limit {
                    break;
                }
            }
        }

        if out.len() >= limit || out.len() == before {
            break;
        }
        start += LIBRARY_PAGE_SIZE;
    }

    Ok(DoubanLibraryList {
        status: status.as_str(),
        label: status.label(),
        items: out,
    })
}

pub async fn mark_interest(
    cookie_header: &str,
    subject: &str,
    interest: DoubanInterest,
    rating: Option<u8>,
    tags: &str,
) -> Result<DoubanInterestResult, DoubanError> {
    let cookie = require_auth_cookie(cookie_header)?;
    let subject_id = subject_id(subject)?;
    if matches!(interest, DoubanInterest::Wish) && rating.is_some() {
        return Err(DoubanError::bad_request("想看状态不能设置评分"));
    }
    if let Some(rating) = rating {
        if !(1..=5).contains(&rating) {
            return Err(DoubanError::bad_request("评分必须是 1 到 5 星"));
        }
    }
    let tags = normalize_interest_tags(tags)?;

    let detail_url = subject_url(&subject_id);
    let ck = if let Some(ck) = cookie_value(&cookie, "ck") {
        ck
    } else {
        let html = fetch_html(&detail_url, &cookie, MOVIE_BASE_URL).await?;
        extract_ck_from_html(&html)
            .ok_or_else(|| DoubanError::bad_request("豆瓣页面缺少 ck，无法提交看过/想看标记"))?
    };

    let url = format!("{SUBJECT_INTEREST_URL_PREFIX}{subject_id}/interest");
    let rating_value = rating.map(|n| n.to_string()).unwrap_or_default();
    let form = [
        ("ck", ck.as_str()),
        ("interest", interest.as_str()),
        ("rating", rating_value.as_str()),
        ("foldcollect", "F"),
        ("tags", tags.as_str()),
        ("comment", ""),
        ("private", "on"),
    ];

    let client = Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| DoubanError::upstream(format!("创建豆瓣标记客户端失败: {e}")))?;
    let resp = client
        .post(&url)
        .header("User-Agent", DESKTOP_CHROME_UA)
        .header("Accept", "application/json, text/javascript, */*; q=0.01")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Referer", detail_url)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Cookie", cookie)
        .form(&form)
        .send()
        .await
        .map_err(|e| DoubanError::upstream(format!("豆瓣标记请求失败: {e}")))?;
    let status = resp.status();
    let final_url = resp.url().to_string();
    let text = resp
        .text()
        .await
        .map_err(|e| DoubanError::upstream(format!("读取豆瓣标记响应失败: {e}")))?;
    ensure_html_success(status, &final_url, &text)?;
    let data: DoubanInterestResponse = serde_json::from_str(&text).map_err(|e| {
        DoubanError::upstream(format!(
            "解析豆瓣标记响应失败: {e}: {}",
            text.chars().take(300).collect::<String>()
        ))
    })?;
    if data.r.unwrap_or(0) != 0 {
        let msg = if data.msg.trim().is_empty() {
            format!("豆瓣标记失败: r={}", data.r.unwrap_or_default())
        } else {
            data.msg
        };
        return Err(DoubanError::bad_request(msg));
    }

    Ok(DoubanInterestResult {
        ok: true,
        interest: interest.as_str(),
        rating,
        tags,
    })
}

fn normalize_interest_tags(raw: &str) -> Result<String, DoubanError> {
    let tags = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if tags.chars().count() > 120 {
        return Err(DoubanError::bad_request("标签最多 120 个字符"));
    }
    Ok(tags)
}

pub async fn fetch_image(raw_url: &str) -> Result<(String, Vec<u8>), DoubanError> {
    let candidates = image_fetch_candidates(raw_url);
    if candidates.is_empty() {
        return Err(DoubanError::bad_request("无效的豆瓣封面图片 URL"));
    }

    let client = Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|e| DoubanError::upstream(format!("创建豆瓣图片客户端失败: {e}")))?;
    let mut last_error = String::new();

    for url in candidates {
        let Ok(parsed) = Url::parse(&url) else {
            last_error = format!("URL 无效: {url}");
            continue;
        };
        if !is_allowed_douban_image_url(&parsed) {
            last_error = format!("拒绝非豆瓣图片 URL: {url}");
            continue;
        }

        let resp = match client
            .get(parsed)
            .header("User-Agent", DESKTOP_CHROME_UA)
            .header(
                "Accept",
                "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
            )
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .header("Referer", IMAGE_REFERER)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                last_error = format!("{url}: {e}");
                continue;
            }
        };

        let status = resp.status();
        let final_url = resp.url().clone();
        if !is_allowed_douban_image_url(&final_url) {
            last_error = format!("豆瓣图片跳转到不可信地址: {final_url}");
            continue;
        }
        if !status.is_success() {
            last_error = format!("{url}: HTTP {status}");
            continue;
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "image/jpeg".to_string());
        if !content_type.starts_with("image/") {
            last_error = format!("{url}: 非图片响应 {content_type}");
            continue;
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| DoubanError::upstream(format!("读取豆瓣图片失败: {e}")))?;
        if bytes.is_empty() {
            last_error = format!("{url}: 图片为空");
            continue;
        }

        return Ok((content_type, bytes.to_vec()));
    }

    Err(DoubanError::upstream(format!(
        "豆瓣封面图片下载失败: {last_error}"
    )))
}

async fn request_json<T: for<'de> Deserialize<'de>>(
    client: &Client,
    url: &str,
) -> Result<T, DoubanError> {
    let resp = client
        .get(url)
        .header("User-Agent", DESKTOP_CHROME_UA)
        .header("Accept", "application/json, text/javascript, */*; q=0.01")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Referer", LOGIN_REFERER)
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await
        .map_err(|e| DoubanError::upstream(format!("豆瓣请求失败: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| DoubanError::upstream(format!("读取豆瓣响应失败: {e}")))?;
    if !status.is_success() {
        return Err(DoubanError::upstream(format!(
            "豆瓣接口 HTTP {status}: {}",
            text.chars().take(500).collect::<String>()
        )));
    }
    serde_json::from_str(&text).map_err(|e| {
        DoubanError::upstream(format!(
            "解析豆瓣 JSON 失败: {e}: {}",
            text.chars().take(500).collect::<String>()
        ))
    })
}

async fn fetch_html(url: &str, cookie_header: &str, referer: &str) -> Result<String, DoubanError> {
    let client = Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| DoubanError::upstream(format!("创建豆瓣客户端失败: {e}")))?;
    let resp = client
        .get(url)
        .header("User-Agent", DESKTOP_CHROME_UA)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Referer", referer)
        .header("Cookie", cookie_header)
        .send()
        .await
        .map_err(|e| DoubanError::upstream(format!("豆瓣影视请求失败: {e}")))?;
    let status = resp.status();
    let final_url = resp.url().to_string();
    let text = resp
        .text()
        .await
        .map_err(|e| DoubanError::upstream(format!("读取豆瓣页面失败: {e}")))?;
    ensure_html_success(status, &final_url, &text)?;
    Ok(text)
}

fn ensure_html_success(status: StatusCode, final_url: &str, body: &str) -> Result<(), DoubanError> {
    if final_url.contains("accounts.douban.com") || final_url.contains("/passport/login") {
        return Err(DoubanError::bad_request(
            "豆瓣请求被重定向到登录页，请更新 Cookie",
        ));
    }
    if final_url.contains("sec.douban.com") {
        return Err(DoubanError::bad_request(
            "豆瓣请求被重定向到安全验证页，请稍后重试或重新登录",
        ));
    }
    if !status.is_success() {
        return Err(DoubanError::upstream(format!(
            "豆瓣页面 HTTP {status}: {}",
            extract_title(body).unwrap_or_default()
        )));
    }
    Ok(())
}

fn jar_cookie_header(jar: &Jar) -> Result<String, DoubanError> {
    let url = Url::parse(MOVIE_BASE_URL)
        .map_err(|e| DoubanError::upstream(format!("构造豆瓣 Cookie URL 失败: {e}")))?;
    let Some(value) = jar.cookies(&url) else {
        return Ok(String::new());
    };
    value
        .to_str()
        .map(normalize_cookie_header)
        .map_err(|e| DoubanError::upstream(format!("读取豆瓣 Cookie 失败: {e}")))
}

fn new_session_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = QR_SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{now:x}{seq:x}")
}

fn subject_url(subject_id: &str) -> String {
    format!("{SUBJECT_URL_PREFIX}{subject_id}/")
}

fn subject_id(value: &str) -> Result<String, DoubanError> {
    let raw = value.trim();
    if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
        return Ok(raw.to_string());
    }
    if let Some(idx) = raw.find("/subject/") {
        let rest = &raw[idx + "/subject/".len()..];
        let id = rest
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        if !id.is_empty() {
            return Ok(id);
        }
    }
    Err(DoubanError::bad_request("无法解析豆瓣 subject id"))
}

fn library_page_url(status: DoubanLibraryStatus, start: usize) -> Result<Url, DoubanError> {
    let mut url = Url::parse(MINE_URL)
        .map_err(|e| DoubanError::upstream(format!("构造豆瓣列表 URL 失败: {e}")))?;
    url.query_pairs_mut()
        .append_pair("status", status.as_str())
        .append_pair("sort", "time")
        .append_pair("start", &start.to_string())
        .append_pair("filter", "all")
        .append_pair("mode", "grid");
    Ok(url)
}

fn extract_library_items(html: &str, status: DoubanLibraryStatus) -> Vec<DoubanLibraryItem> {
    let mut out = Vec::new();
    for block in div_blocks_with_class(html, "item") {
        let Some(subject_id) = first_subject_id_in_text(block) else {
            continue;
        };
        let title = extract_subject_anchor_text(block, &subject_id)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| subject_id.clone());
        let url = subject_url(&subject_id);
        let intro = first_element_text_by_class(block, "intro").unwrap_or_default();
        let date = first_element_text_by_class(block, "date").unwrap_or_default();
        let comment = first_element_text_by_class(block, "comment").unwrap_or_default();
        let tags = first_element_text_by_class(block, "tags")
            .map(|s| {
                s.trim_start_matches("标签:")
                    .trim_start_matches("标签：")
                    .split_whitespace()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cover_url = best_subject_image_url(&Value::Null, block)
            .as_deref()
            .and_then(proxied_image_url)
            .unwrap_or_default();
        let user_rating = extract_user_rating(block);

        out.push(DoubanLibraryItem {
            source: "douban",
            media_type: "douban",
            id: subject_id.clone(),
            subject_id,
            title,
            url,
            abstract_text: intro,
            abstract_2: [date.as_str(), comment.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
            cover_url: cover_url.clone(),
            poster_url: cover_url,
            status: status.as_str(),
            status_label: status.label(),
            date,
            comment,
            tags,
            user_rating,
        });
    }
    out
}

fn div_blocks_with_class<'a>(html: &'a str, class_name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while let Some(rel) = html[offset..].find("<div") {
        let start = offset + rel;
        let Some(tag_end_rel) = html[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end_rel + 1;
        let tag = &html[start..tag_end];
        if tag_has_class(tag, class_name) {
            let Some(end) = matching_div_end(html, tag_end) else {
                break;
            };
            out.push(&html[start..end]);
            offset = end;
        } else {
            offset = tag_end;
        }
    }
    out
}

fn matching_div_end(html: &str, content_start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut offset = content_start;
    loop {
        let next_open = html[offset..].find("<div").map(|i| offset + i);
        let next_close = html[offset..].find("</div").map(|i| offset + i);
        match (next_open, next_close) {
            (None, None) => return None,
            (Some(open), Some(close)) if open < close => {
                let tag_end = html[open..].find('>').map(|i| open + i + 1)?;
                depth += 1;
                offset = tag_end;
            }
            (_, Some(close)) => {
                let tag_end = html[close..].find('>').map(|i| close + i + 1)?;
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(tag_end);
                }
                offset = tag_end;
            }
            (Some(open), None) => {
                let tag_end = html[open..].find('>').map(|i| open + i + 1)?;
                depth += 1;
                offset = tag_end;
            }
        }
    }
}

fn tag_has_class(tag: &str, class_name: &str) -> bool {
    attr_value(tag, "class")
        .map(|value| value.split_whitespace().any(|c| c == class_name))
        .unwrap_or(false)
}

fn first_subject_id_in_text(text: &str) -> Option<String> {
    let idx = text.find("/subject/")?;
    let rest = &text[idx + "/subject/".len()..];
    let id = rest
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

fn extract_subject_anchor_text(block: &str, subject_id: &str) -> Option<String> {
    let needle = format!("/subject/{subject_id}");
    let mut offset = 0usize;
    while let Some(rel) = block[offset..].find(&needle) {
        let idx = offset + rel;
        let Some(anchor_start) = block[..idx].rfind("<a") else {
            offset = idx + needle.len();
            continue;
        };
        let Some(tag_end_rel) = block[anchor_start..].find('>') else {
            offset = idx + needle.len();
            continue;
        };
        let inner_start = anchor_start + tag_end_rel + 1;
        let Some(anchor_end_rel) = block[inner_start..].find("</a>") else {
            offset = idx + needle.len();
            continue;
        };
        let text = strip_html(&block[inner_start..inner_start + anchor_end_rel]);
        if !text.is_empty() {
            return Some(text);
        }
        offset = idx + needle.len();
    }
    None
}

fn first_element_text_by_class(html: &str, class_name: &str) -> Option<String> {
    let mut offset = 0usize;
    while let Some(rel) = html[offset..].find('<') {
        let start = offset + rel;
        if html[start..].starts_with("</") || html[start..].starts_with("<!--") {
            offset = start + 1;
            continue;
        }
        let Some(tag_end_rel) = html[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end_rel + 1;
        let tag = &html[start..tag_end];
        let Some(name) = tag_name(tag) else {
            offset = tag_end;
            continue;
        };
        if tag_has_class(tag, class_name) {
            let close = format!("</{name}>");
            if let Some(end_rel) = html[tag_end..].find(&close) {
                let text = strip_html(&html[tag_end..tag_end + end_rel]);
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
        offset = tag_end;
    }
    None
}

fn tag_name(tag: &str) -> Option<&str> {
    let rest = tag.strip_prefix('<')?.trim_start();
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .unwrap_or(rest.len());
    (end > 0).then_some(&rest[..end])
}

fn extract_user_rating(block: &str) -> Option<u8> {
    if let Some(value) = first_tag_attr_by_marker(block, "id=\"n_rating\"", "value")
        .or_else(|| first_tag_attr_by_marker(block, "id='n_rating'", "value"))
        .and_then(|value| value.trim().parse::<u8>().ok())
        .filter(|value| (1..=5).contains(value))
    {
        return Some(value);
    }
    (1u8..=5)
        .rev()
        .find(|n| block.contains(&format!("rating{n}-t")))
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let tag_end = lower[start..].find('>')? + start + 1;
    let end = lower[tag_end..].find("</title>")? + tag_end;
    Some(normalize_ws(&strip_html(&html[tag_end..end])))
}

fn rating_from_value(value: Option<&Value>) -> DoubanRating {
    let value = value.unwrap_or(&Value::Null);
    DoubanRating {
        value: number_from_value(
            value
                .get("ratingValue")
                .or_else(|| value.get("value"))
                .unwrap_or(&Value::Null),
        ),
        count: value
            .get("ratingCount")
            .or_else(|| value.get("count"))
            .and_then(u64_from_value),
        info: value_to_string(value.get("rating_info")).unwrap_or_default(),
        star_count: number_from_value(value.get("star_count").unwrap_or(&Value::Null)),
    }
}

fn number_from_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn u64_from_value(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().replace(',', "").parse().ok(),
        _ => None,
    }
}

fn people_names(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items.iter().flat_map(|v| people_names(Some(v))).collect(),
        Some(Value::Object(obj)) => value_to_string(obj.get("name")).into_iter().collect(),
        Some(v) => value_to_string(Some(v)).into_iter().collect(),
        None => Vec::new(),
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| value_to_string(Some(v)))
            .collect(),
        Some(v) => value_to_string(Some(v)).into_iter().collect(),
        None => Vec::new(),
    }
}

fn first_string_from_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| value_to_string(Some(v)))
            .find(|s| !s.is_empty()),
        v => value_to_string(Some(v)).filter(|s| !s.is_empty()),
    }
}

fn first_tag_attr_by_marker(html: &str, marker: &str, attr: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let marker = marker.to_lowercase();
    let idx = lower.find(&marker)?;
    let tag_start = if lower[idx..].starts_with('<') {
        idx
    } else {
        html[..idx].rfind('<')?
    };
    let tag_end = html[tag_start..].find('>')? + tag_start + 1;
    attr_value(&html[tag_start..tag_end], attr).filter(|s| !s.is_empty())
}

fn rexxar_user_interest(value: Option<&Value>) -> (Option<DoubanInterest>, Option<u8>) {
    let Some(Value::Object(obj)) = value else {
        return (None, None);
    };
    let status = ["status", "interest", "type", "state"]
        .into_iter()
        .filter_map(|key| value_to_string(obj.get(key)))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let interest = if status.contains("wish") || status.contains("mark") || status.contains("想看")
    {
        Some(DoubanInterest::Wish)
    } else if status.contains("collect")
        || status.contains("done")
        || status.contains("watched")
        || status.contains("看过")
    {
        Some(DoubanInterest::Collect)
    } else {
        None
    };
    let rating = obj
        .get("rating")
        .and_then(rexxar_user_rating)
        .or_else(|| obj.get("rating_value").and_then(rexxar_user_rating))
        .or_else(|| obj.get("star_count").and_then(rexxar_user_rating));
    (interest, rating)
}

fn subject_user_interest_from_html(html: &str) -> (Option<DoubanInterest>, Option<u8>) {
    let Some(block) = element_block_by_id(html, "interest_sect_level") else {
        return (None, None);
    };
    let text = normalize_ws(&strip_html(block));
    let interest = if text.contains("我想看") {
        Some(DoubanInterest::Wish)
    } else if text.contains("我看过") {
        Some(DoubanInterest::Collect)
    } else {
        None
    };
    let rating = extract_user_rating(block);
    (interest, rating)
}

fn element_block_by_id<'a>(html: &'a str, id: &str) -> Option<&'a str> {
    let markers = [
        format!("id=\"{id}\""),
        format!("id='{id}'"),
        format!("id = \"{id}\""),
        format!("id = '{id}'"),
    ];
    for marker in markers {
        let Some(idx) = html.find(&marker) else {
            continue;
        };
        let Some(tag_start) = html[..idx].rfind('<') else {
            continue;
        };
        let tag_end = html[tag_start..].find('>').map(|i| tag_start + i + 1)?;
        let tag = &html[tag_start..tag_end];
        let Some(name) = tag_name(tag) else {
            continue;
        };
        if name.eq_ignore_ascii_case("div") {
            let end = matching_div_end(html, tag_end)?;
            return Some(&html[tag_start..end]);
        }
        let close = format!("</{name}>");
        let end = html[tag_end..]
            .find(&close)
            .map(|i| tag_end + i + close.len())?;
        return Some(&html[tag_start..end]);
    }
    None
}

fn rexxar_user_rating(value: &Value) -> Option<u8> {
    let raw = match value {
        Value::Object(obj) => obj
            .get("value")
            .or_else(|| obj.get("star_count"))
            .or_else(|| obj.get("rating"))
            .and_then(number_from_value)?,
        v => number_from_value(v)?,
    };
    let stars = if raw > 5.0 && raw <= 10.0 {
        (raw / 2.0).round()
    } else {
        raw.round()
    };
    ((1.0..=5.0).contains(&stars)).then_some(stars as u8)
}

fn best_image_url_from_value(value: &Value) -> Option<String> {
    let mut candidates = Vec::new();
    collect_priority_image_fields(value, &mut candidates);
    collect_image_urls_from_value(value, &mut candidates);
    choose_best_image_url(candidates)
}

fn best_subject_image_url(data: &Value, html: &str) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(image) = value_to_string(data.get("image")) {
        collect_image_urls_from_text(&image, &mut candidates);
        push_clean_image_url(&mut candidates, &image);
    }
    collect_tag_attr_images(html, "property=\"og:image\"", "content", &mut candidates);
    collect_tag_attr_images(html, "property='og:image'", "content", &mut candidates);
    collect_tag_attr_images(html, "rel=\"v:image\"", "src", &mut candidates);
    collect_tag_attr_images(html, "rel='v:image'", "src", &mut candidates);
    collect_tag_attr_images(html, "itemprop=\"image\"", "src", &mut candidates);
    collect_tag_attr_images(html, "itemprop=\"image\"", "content", &mut candidates);
    collect_tag_attr_images(html, "itemprop='image'", "src", &mut candidates);
    collect_tag_attr_images(html, "itemprop='image'", "content", &mut candidates);
    collect_image_urls_from_text(html, &mut candidates);
    choose_best_image_url(candidates)
}

fn collect_priority_image_fields(value: &Value, out: &mut Vec<String>) {
    let Value::Object(obj) = value else {
        return;
    };
    for key in [
        "cover_url",
        "poster_url",
        "image",
        "img",
        "pic",
        "cover",
        "thumbnail",
    ] {
        if let Some(v) = obj.get(key) {
            collect_image_urls_from_value(v, out);
        }
    }
}

fn collect_image_urls_from_value(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            push_clean_image_url(out, s);
            collect_image_urls_from_text(s, out);
        }
        Value::Array(items) => {
            for item in items {
                collect_image_urls_from_value(item, out);
            }
        }
        Value::Object(obj) => {
            for v in obj.values() {
                collect_image_urls_from_value(v, out);
            }
        }
        _ => {}
    }
}

fn collect_tag_attr_images(html: &str, marker: &str, attr: &str, out: &mut Vec<String>) {
    let lower = html.to_lowercase();
    let marker = marker.to_lowercase();
    let mut offset = 0usize;
    while let Some(rel) = lower[offset..].find(&marker) {
        let marker_idx = offset + rel;
        let tag_start = html[..marker_idx].rfind('<').unwrap_or(marker_idx);
        let tag_end = html[marker_idx..]
            .find('>')
            .map(|i| marker_idx + i + 1)
            .unwrap_or_else(|| html.len());
        let tag = &html[tag_start..tag_end];
        if let Some(value) = attr_value(tag, attr) {
            push_clean_image_url(out, &value);
            collect_image_urls_from_text(&value, out);
        }
        offset = tag_end;
    }
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let mut offset = 0usize;
    while let Some(rel) = lower[offset..].find(attr) {
        let attr_idx = offset + rel;
        let after_attr = attr_idx + attr.len();
        let rest = &tag[after_attr..];
        let rest_trimmed = rest.trim_start();
        let skipped = rest.len() - rest_trimmed.len();
        if !rest_trimmed.starts_with('=') {
            offset = after_attr;
            continue;
        }
        let value_start = after_attr + skipped + 1;
        let value = tag[value_start..].trim_start();
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            offset = value_start;
            continue;
        }
        let inner_start = value_start + quote.len_utf8();
        let inner = &tag[inner_start..];
        let inner_end = inner.find(quote)?;
        return Some(html_unescape(&inner[..inner_end]));
    }
    None
}

fn collect_image_urls_from_text(text: &str, out: &mut Vec<String>) {
    let decoded = html_unescape(&text.replace("\\/", "/").replace("\\u002F", "/"));
    for prefix in ["https://img", "http://img", "//img"] {
        let mut offset = 0usize;
        while let Some(rel) = decoded[offset..].find(prefix) {
            let start = offset + rel;
            let end = decoded[start..]
                .find(|ch: char| {
                    ch.is_whitespace()
                        || matches!(ch, '"' | '\'' | '<' | '>' | ')' | '(' | ',' | ']')
                })
                .map(|i| start + i)
                .unwrap_or_else(|| decoded.len());
            push_clean_image_url(out, &decoded[start..end]);
            if end >= decoded.len() {
                break;
            }
            offset = end + 1;
        }
    }
}

fn choose_best_image_url(candidates: Vec<String>) -> Option<String> {
    let mut seen = Vec::<String>::new();
    let mut best: Option<(i32, usize, String)> = None;
    for candidate in candidates {
        let Some(cleaned) = clean_douban_image_url(&candidate) else {
            continue;
        };
        if seen.iter().any(|s| s == &cleaned) {
            continue;
        }
        let idx = seen.len();
        seen.push(cleaned.clone());
        let score = image_score(&cleaned);
        if best
            .as_ref()
            .map(|(best_score, best_idx, _)| {
                score > *best_score || (score == *best_score && idx < *best_idx)
            })
            .unwrap_or(true)
        {
            best = Some((score, idx, cleaned));
        }
    }
    best.map(|(_, _, url)| url)
}

fn image_score(url: &str) -> i32 {
    let mut score = 0;
    let path = Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| url.to_string());
    if path.contains("/l_ratio_poster/") {
        score += 80;
    } else if path.contains("/m_ratio_poster/") {
        score += 60;
    } else if path.contains("/s_ratio_poster/") {
        score += 40;
    } else if path.contains("_ratio_poster") {
        score += 35;
    } else if path.contains("/l/public/") {
        score += 30;
    } else if path.contains("/m/public/") {
        score += 20;
    } else if path.contains("/s/public/") {
        score += 10;
    }
    if path.contains("/public/p") {
        score += 10;
    }
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        score += 2;
    }
    score
}

fn proxied_image_url(raw: &str) -> Option<String> {
    let clean = clean_douban_image_url(raw)?;
    Some(format!(
        "/api/douban/image?url={}",
        percent_encode_query_component(&clean)
    ))
}

fn image_fetch_candidates(raw: &str) -> Vec<String> {
    let Some(cleaned) = clean_douban_image_url(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if has_ratio_poster_size(&cleaned) {
        for size in ["l_ratio_poster", "m_ratio_poster", "s_ratio_poster"] {
            if let Some(variant) = with_ratio_poster_size(&cleaned, size) {
                push_image_variant(&mut out, &variant);
            }
        }
    } else if has_plain_photo_size(&cleaned) {
        for size in ["l", "m", "s"] {
            if let Some(variant) = with_plain_photo_size(&cleaned, size) {
                push_image_variant(&mut out, &variant);
            }
        }
    }
    push_image_variant(&mut out, &cleaned);
    out
}

fn with_ratio_poster_size(raw: &str, size: &str) -> Option<String> {
    for current in ["s_ratio_poster", "m_ratio_poster", "l_ratio_poster"] {
        let needle = format!("/{current}/");
        if raw.contains(&needle) {
            return Some(raw.replace(&needle, &format!("/{size}/")));
        }
    }
    None
}

fn with_plain_photo_size(raw: &str, size: &str) -> Option<String> {
    for current in ["s", "m", "l"] {
        let needle = format!("/view/photo/{current}/public/");
        if raw.contains(&needle) {
            return Some(raw.replace(&needle, &format!("/view/photo/{size}/public/")));
        }
    }
    None
}

fn has_ratio_poster_size(raw: &str) -> bool {
    ["s_ratio_poster", "m_ratio_poster", "l_ratio_poster"]
        .iter()
        .any(|size| raw.contains(&format!("/{size}/")))
}

fn has_plain_photo_size(raw: &str) -> bool {
    [
        "/view/photo/s/public/",
        "/view/photo/m/public/",
        "/view/photo/l/public/",
    ]
    .iter()
    .any(|segment| raw.contains(segment))
}

fn push_image_variant(out: &mut Vec<String>, raw: &str) {
    if raw.ends_with(".webp") {
        push_unique(out, &format!("{}.jpg", raw.trim_end_matches(".webp")));
    }
    push_unique(out, raw);
}

fn push_clean_image_url(out: &mut Vec<String>, raw: &str) {
    if let Some(url) = clean_douban_image_url(raw) {
        push_unique(out, &url);
    }
}

fn push_unique(out: &mut Vec<String>, value: &str) {
    if !out.iter().any(|s| s == value) {
        out.push(value.to_string());
    }
}

fn clean_douban_image_url(raw: &str) -> Option<String> {
    let mut s = html_unescape(raw.trim())
        .replace("\\/", "/")
        .replace("\\u002F", "/")
        .trim_matches(|ch| matches!(ch, '"' | '\'' | ' ' | '\n' | '\r' | '\t'))
        .to_string();
    if s.starts_with("//") {
        s = format!("https:{s}");
    } else if let Some(rest) = s.strip_prefix("http://") {
        s = format!("https://{rest}");
    }
    let mut url = Url::parse(&s).ok()?;
    let query = url
        .query()
        .filter(|query| is_allowed_douban_image_query(query))
        .map(ToOwned::to_owned);
    url.set_query(query.as_deref());
    url.set_fragment(None);
    if !is_allowed_douban_image_url(&url) {
        return None;
    }
    Some(url.to_string())
}

fn is_allowed_douban_image_query(query: &str) -> bool {
    query.starts_with("imageView2/")
        && query
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.'))
}

fn is_allowed_douban_image_url(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    (host == "doubanio.com" || host.ends_with(".doubanio.com"))
        && url.path().contains("/view/photo/")
}

fn percent_encode_query_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|v| value_to_string(Some(v)))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" / "),
        ),
        Value::Object(_) => None,
    }
    .filter(|s| !s.is_empty())
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    html_unescape(&normalize_ws(&out))
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn html_unescape(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn proxied_image_url_keeps_original_url_for_fallbacks() {
        let url = proxied_image_url(
            "http://img1.doubanio.com/view/photo/s_ratio_poster/public/p123.webp",
        )
        .expect("proxy url");
        assert!(url.starts_with("/api/douban/image?url="));
        assert!(url.contains(
            "https%3A%2F%2Fimg1.doubanio.com%2Fview%2Fphoto%2Fs_ratio_poster%2Fpublic%2Fp123.webp"
        ));
    }

    #[test]
    fn proxied_image_url_preserves_rexxar_image_view_query() {
        let url = proxied_image_url(
            "https://qnmob3.doubanio.com/view/photo/large/public/p480747492.jpg?imageView2/0/q/80/w/9999/h/120/format/jpg",
        )
        .expect("proxy url");

        assert!(url.contains("qnmob3.doubanio.com"));
        assert!(url.contains("imageView2%2F0%2Fq%2F80%2Fw%2F9999%2Fh%2F120%2Fformat%2Fjpg"));
    }

    #[test]
    fn image_fetch_candidates_prefer_large_and_keep_original() {
        let candidates = image_fetch_candidates(
            "https://img2.doubanio.com/view/photo/s_ratio_poster/public/p1.webp",
        );
        assert_eq!(
            candidates,
            vec![
                "https://img2.doubanio.com/view/photo/l_ratio_poster/public/p1.jpg",
                "https://img2.doubanio.com/view/photo/l_ratio_poster/public/p1.webp",
                "https://img2.doubanio.com/view/photo/m_ratio_poster/public/p1.jpg",
                "https://img2.doubanio.com/view/photo/m_ratio_poster/public/p1.webp",
                "https://img2.doubanio.com/view/photo/s_ratio_poster/public/p1.jpg",
                "https://img2.doubanio.com/view/photo/s_ratio_poster/public/p1.webp",
            ]
        );
    }

    #[test]
    fn best_image_url_from_value_finds_nested_cover() {
        let value = json!({
            "title": "subject",
            "nested": {
                "cover_url": "https:\\/\\/img9.doubanio.com\\/view\\/photo\\/s_ratio_poster\\/public\\/p999.jpg"
            }
        });
        assert_eq!(
            best_image_url_from_value(&value).as_deref(),
            Some("https://img9.doubanio.com/view/photo/s_ratio_poster/public/p999.jpg")
        );
    }

    #[test]
    fn extract_ck_from_hidden_input_or_logout_url() {
        assert_eq!(
            extract_ck_from_html(r#"<input type="hidden" name="ck" value="6z5p"/>"#).as_deref(),
            Some("6z5p")
        );
        assert_eq!(
            extract_ck_from_html(r#"<a href="/accounts/logout?source=movie&ck=abcd">退出</a>"#)
                .as_deref(),
            Some("abcd")
        );
    }

    #[test]
    fn extract_library_items_reads_collection_card() {
        let html = r#"
        <div class="grid-view">
          <div class="item">
            <div class="pic">
              <a href="https://movie.douban.com/subject/1234567/">
                <img src="https://img1.doubanio.com/view/photo/s_ratio_poster/public/p123.webp" />
              </a>
            </div>
            <div class="info">
              <ul>
                <li class="title">
                  <a href="https://movie.douban.com/subject/1234567/"><em>测试电影</em> / Test Movie</a>
                </li>
                <li class="intro">2024 / 中国大陆 / 剧情</li>
                <li><span class="rating4-t"></span><span class="date">2026-06-20</span></li>
                <li><span class="comment">短评内容</span></li>
                <li><span class="tags">标签: 华语 剧情</span></li>
              </ul>
            </div>
          </div>
        </div>
        "#;

        let items = extract_library_items(html, DoubanLibraryStatus::Collect);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.subject_id, "1234567");
        assert_eq!(item.title, "测试电影 / Test Movie");
        assert_eq!(item.abstract_text, "2024 / 中国大陆 / 剧情");
        assert_eq!(item.date, "2026-06-20");
        assert_eq!(item.comment, "短评内容");
        assert_eq!(item.tags, vec!["华语", "剧情"]);
        assert_eq!(item.user_rating, Some(4));
        assert!(item.poster_url.starts_with("/api/douban/image?url="));
    }

    #[test]
    fn subject_detail_from_rexxar_json_keeps_title_languages_separate() {
        let data = json!({
            "id": "1292052",
            "title": "肖申克的救赎",
            "original_title": "The Shawshank Redemption",
            "aka": ["月黑高飞(港)", "刺激1995(台)"],
            "url": "https://movie.douban.com/subject/1292052/",
            "cover_url": "https://img3.doubanio.com/view/photo/m_ratio_poster/public/p480747492.jpg",
            "directors": [{"name": "弗兰克·德拉邦特"}],
            "actors": [{"name": "蒂姆·罗宾斯"}, {"name": "摩根·弗里曼"}],
            "genres": ["剧情", "犯罪"],
            "pubdate": ["1994-09-10(多伦多电影节)"],
            "durations": ["142分钟"],
            "intro": "剧情简介",
            "rating": {
                "count": 3300949,
                "star_count": 5.0,
                "value": 9.7
            },
            "languages": ["英语"],
            "countries": ["美国"]
        });

        let detail = subject_detail_from_rexxar_json("1292052", &data).expect("rexxar detail");

        assert_eq!(detail.title, "肖申克的救赎");
        assert_eq!(detail.original_title, "The Shawshank Redemption");
        assert_eq!(detail.aka, vec!["月黑高飞(港)", "刺激1995(台)"]);
        assert_eq!(detail.languages, vec!["英语"]);
        assert_eq!(detail.countries, vec!["美国"]);
        assert_eq!(detail.date_published, "1994-09-10(多伦多电影节)");
        assert_eq!(detail.duration, "142分钟");
        assert_eq!(detail.summary, "剧情简介");
        assert_eq!(detail.directors, vec!["弗兰克·德拉邦特"]);
        assert!(detail.writers.is_empty());
        assert_eq!(detail.actors, vec!["蒂姆·罗宾斯", "摩根·弗里曼"]);
        assert_eq!(detail.genres, vec!["剧情", "犯罪"]);
        assert_eq!(detail.rating.value, Some(9.7));
        assert_eq!(detail.rating.count, Some(3300949));
        assert_eq!(detail.rating.star_count, Some(5.0));
        assert!(detail.poster_url.starts_with("/api/douban/image?url="));
    }

    #[test]
    fn subject_user_interest_from_html_reads_wish_state() {
        let html = r#"
        <div id="interest_sect_level" class="clearfix">
          <div class="j a_stars">
            <span class="mr10">
              我想看这部电影
              <span class="collection_date">2026-06-24</span>
            </span>
          </div>
        </div>
        "#;

        let (interest, rating) = subject_user_interest_from_html(html);

        assert!(matches!(interest, Some(DoubanInterest::Wish)));
        assert_eq!(rating, None);
    }

    #[test]
    fn subject_user_interest_from_html_reads_collect_state_and_rating() {
        let html = r#"
        <div id="interest_sect_level" class="clearfix">
          <div class="j a_stars">
            <span class="mr10">我看过这部电影</span>
            我的评价:
            <input id="n_rating" type="hidden" value="4" />
          </div>
        </div>
        "#;

        let (interest, rating) = subject_user_interest_from_html(html);

        assert!(matches!(interest, Some(DoubanInterest::Collect)));
        assert_eq!(rating, Some(4));
    }

    #[test]
    fn search_results_from_rexxar_json_read_subject_targets() {
        let data = json!({
            "subjects": {
                "items": [
                    {
                        "layout": "subject",
                        "target_id": "1292052",
                        "target_type": "movie",
                        "type_name": "电影",
                        "target": {
                            "id": "1292052",
                            "title": "肖申克的救赎",
                            "year": "1994",
                            "card_subtitle": "美国 / 剧情 犯罪 / 弗兰克·德拉邦特",
                            "cover_url": "https://qnmob3.doubanio.com/view/photo/large/public/p480747492.jpg?imageView2/0/q/80",
                            "rating": {
                                "count": 3300949,
                                "star_count": 5.0,
                                "value": 9.7
                            },
                            "uri": "douban://douban.com/movie/1292052"
                        }
                    },
                    {
                        "layout": "review",
                        "target_type": "review",
                        "target": { "id": "skip", "title": "跳过" }
                    },
                    {
                        "layout": "subject",
                        "target_id": "35467152",
                        "target_type": "tv",
                        "type_name": "电视剧",
                        "target": {
                            "id": "35467152",
                            "title": "测试剧集",
                            "year": "2024",
                            "card_subtitle": "中国大陆 / 剧情",
                            "cover_url": "https://qnmob3.doubanio.com/view/photo/large/public/p1.jpg",
                            "rating": {
                                "count": 1000,
                                "star_count": 4.0,
                                "value": 8.1
                            },
                            "uri": "douban://douban.com/movie/35467152"
                        }
                    }
                ]
            }
        });

        let items = search_results_from_rexxar_json(&data, 10).expect("rexxar search results");

        assert_eq!(items.len(), 2);
        let item = &items[0];
        assert_eq!(item.id, "1292052");
        assert_eq!(item.subject_id, "1292052");
        assert_eq!(item.title, "肖申克的救赎");
        assert_eq!(item.abstract_text, "美国 / 剧情 犯罪 / 弗兰克·德拉邦特");
        assert_eq!(item.abstract_2, "1994");
        assert_eq!(item.url, "https://movie.douban.com/subject/1292052/");
        assert_eq!(item.rating.value, Some(9.7));
        assert_eq!(item.rating.count, Some(3300949));
        assert_eq!(item.rating.star_count, Some(5.0));
        assert_eq!(item.vote_average, Some(9.7));
        assert!(item.poster_url.starts_with("/api/douban/image?url="));
        let tv_item = &items[1];
        assert_eq!(tv_item.id, "35467152");
        assert_eq!(tv_item.title, "测试剧集");
        assert_eq!(tv_item.abstract_text, "中国大陆 / 剧情");
        assert_eq!(tv_item.abstract_2, "2024");
        assert_eq!(tv_item.rating.value, Some(8.1));
    }

    #[test]
    fn rexxar_search_url_omits_type_filter() {
        let url = rexxar_search_url("测试剧集", "20").expect("search url");
        let pairs = url.query_pairs().collect::<Vec<_>>();

        assert!(pairs.iter().any(|(key, value)| key == "q" && value == "测试剧集"));
        assert!(pairs.iter().any(|(key, value)| key == "count" && value == "20"));
        assert!(!pairs.iter().any(|(key, _)| key == "type"));
    }
}
