//! Conservative TV episode coverage recognition for torrent titles and files.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EpisodeCoverage {
    Episode {
        season: Option<u32>,
        episode: u32,
    },
    Range {
        season: Option<u32>,
        start: u32,
        end: u32,
    },
    SeasonPack {
        season: u32,
    },
}

impl EpisodeCoverage {
    pub(crate) fn covers(self, season: u32, episode: u32) -> bool {
        match self {
            Self::Episode {
                season: found,
                episode: found_episode,
            } => found.is_none_or(|found| found == season) && found_episode == episode,
            Self::Range {
                season: found,
                start,
                end,
            } => found.is_none_or(|found| found == season) && (start..=end).contains(&episode),
            Self::SeasonPack { season: found } => found == season,
        }
    }

    pub(crate) const fn parts(self) -> (Option<u32>, Option<u32>, Option<u32>) {
        match self {
            Self::Episode { season, episode } => (season, Some(episode), None),
            Self::Range { season, start, end } => (season, Some(start), Some(end)),
            Self::SeasonPack { season } => (Some(season), None, None),
        }
    }

    pub(crate) fn label(self) -> String {
        match self {
            Self::Episode {
                season: Some(season),
                episode,
            } => format!("S{season:02}E{episode:02}"),
            Self::Episode {
                season: None,
                episode,
            } => format!("E{episode:02}"),
            Self::Range {
                season: Some(season),
                start,
                end,
            } => {
                format!("S{season:02}E{start:02}-E{end:02}")
            }
            Self::Range {
                season: None,
                start,
                end,
            } => format!("E{start:02}-E{end:02}"),
            Self::SeasonPack { season } => format!("S{season:02} 全季"),
        }
    }
}

pub(crate) fn recognize(value: &str) -> Option<EpisodeCoverage> {
    let lower = value.to_ascii_lowercase();
    if let Some((season, episode, end)) = find_season_episode(&lower) {
        return Some(range_or_episode(Some(season), episode, end));
    }
    if let Some((episode, end)) = find_episode(&lower) {
        return Some(range_or_episode(None, episode, end));
    }
    if let Some((episode, end)) = find_chinese_episode(value) {
        return Some(range_or_episode(None, episode, end));
    }
    if let Some((episode, end)) = find_bracket_episode(value) {
        return Some(range_or_episode(None, episode, end));
    }
    find_season_pack(&lower).map(|season| EpisodeCoverage::SeasonPack { season })
}

fn find_bracket_episode(text: &str) -> Option<(u32, Option<u32>)> {
    let bytes = text.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b'[' {
            continue;
        }
        let Some((start, mut cursor)) = read_number(bytes, index + 1) else {
            continue;
        };
        if start == 0 {
            continue;
        }
        let mut end = None;
        if cursor < bytes.len() && matches!(bytes[cursor], b'-' | b'~') {
            if let Some((value, next)) = read_number(bytes, cursor + 1) {
                end = Some(value);
                cursor = next;
            }
        }
        if cursor < bytes.len() && bytes[cursor] == b']' {
            return Some((start, end));
        }
    }
    None
}

fn range_or_episode(season: Option<u32>, start: u32, end: Option<u32>) -> EpisodeCoverage {
    match end.filter(|end| *end > start) {
        Some(end) => EpisodeCoverage::Range { season, start, end },
        None => EpisodeCoverage::Episode {
            season,
            episode: start,
        },
    }
}

fn find_season_episode(text: &str) -> Option<(u32, u32, Option<u32>)> {
    let bytes = text.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b's' || index > 0 && bytes[index - 1].is_ascii_alphanumeric() {
            continue;
        }
        let Some((season, mut cursor)) = read_number(bytes, index + 1) else {
            continue;
        };
        cursor = skip_separators(bytes, cursor);
        if cursor >= bytes.len() || bytes[cursor] != b'e' {
            continue;
        }
        let Some((episode, end)) = read_number(bytes, cursor + 1) else {
            continue;
        };
        if season > 0 && episode > 0 {
            return Some((season, episode, read_range_end(bytes, end)));
        }
    }
    None
}

fn find_episode(text: &str) -> Option<(u32, Option<u32>)> {
    let bytes = text.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b'e' || index > 0 && bytes[index - 1].is_ascii_alphabetic() {
            continue;
        }
        let mut cursor = index + 1;
        if cursor < bytes.len() && bytes[cursor] == b'p' {
            cursor += 1;
        }
        let Some((episode, end)) = read_number(bytes, cursor) else {
            continue;
        };
        if episode > 0 {
            return Some((episode, read_range_end(bytes, end)));
        }
    }
    None
}

fn find_chinese_episode(text: &str) -> Option<(u32, Option<u32>)> {
    let chars = text.chars().collect::<Vec<_>>();
    for (index, character) in chars.iter().enumerate() {
        if *character != '第' {
            continue;
        }
        let (start, mut cursor) = read_char_number(&chars, index + 1)?;
        if cursor < chars.len() && chars[cursor] == '集' {
            cursor += 1;
        }
        while cursor < chars.len() && chars[cursor].is_whitespace() {
            cursor += 1;
        }
        if cursor >= chars.len() || !matches!(chars[cursor], '-' | '~' | '至' | '到') {
            return Some((start, None));
        }
        cursor += 1;
        while cursor < chars.len() && chars[cursor].is_whitespace() {
            cursor += 1;
        }
        if cursor < chars.len() && chars[cursor] == '第' {
            cursor += 1;
        }
        let (end, _) = read_char_number(&chars, cursor)?;
        return Some((start, Some(end)));
    }
    None
}

fn find_season_pack(text: &str) -> Option<u32> {
    let pack = [
        "complete",
        "season pack",
        "full season",
        "batch",
        "collection",
        "全季",
        "全集",
        "合集",
        "整季",
    ]
    .iter()
    .any(|keyword| text.contains(keyword));
    let bytes = text.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] == b's' && (index == 0 || !bytes[index - 1].is_ascii_alphanumeric()) {
            if let Some((season, end)) = read_number(bytes, index + 1) {
                if season > 0
                    && (end >= bytes.len() || bytes[end] != b'e')
                    && (pack || end < bytes.len())
                {
                    return Some(season);
                }
            }
        }
    }
    None
}

fn read_range_end(bytes: &[u8], mut cursor: usize) -> Option<u32> {
    if cursor < bytes.len() && bytes[cursor] == b'e' {
        cursor += 1;
    } else {
        if cursor >= bytes.len() || !matches!(bytes[cursor], b'-' | b'_' | b'~' | b' ') {
            return None;
        }
        cursor = skip_range_separators(bytes, cursor);
        if cursor < bytes.len() && bytes[cursor] == b'e' {
            cursor += 1;
        }
        if cursor < bytes.len() && bytes[cursor] == b'p' {
            cursor += 1;
        }
    }
    read_number(bytes, cursor)
        .map(|(value, _)| value)
        .filter(|value| *value > 0)
}

fn read_number(bytes: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut cursor = start;
    let mut value = 0_u32;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(u32::from(bytes[cursor] - b'0'));
        cursor += 1;
    }
    (cursor > start).then_some((value, cursor))
}

fn read_char_number(chars: &[char], start: usize) -> Option<(u32, usize)> {
    let mut cursor = start;
    let mut digits = String::new();
    while cursor < chars.len() && chars[cursor].is_ascii_digit() {
        digits.push(chars[cursor]);
        cursor += 1;
    }
    (!digits.is_empty())
        .then(|| digits.parse().ok())
        .flatten()
        .map(|value| (value, cursor))
}

fn skip_separators(bytes: &[u8], mut cursor: usize) -> usize {
    while cursor < bytes.len() && matches!(bytes[cursor], b'.' | b'_' | b'-' | b' ' | b'[' | b']') {
        cursor += 1;
    }
    cursor
}

fn skip_range_separators(bytes: &[u8], mut cursor: usize) -> usize {
    while cursor < bytes.len() && matches!(bytes[cursor], b'-' | b'_' | b'~' | b' ') {
        cursor += 1;
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_episode_partial_and_season_pack_coverage() {
        assert_eq!(
            recognize("Show.S02E03.1080p"),
            Some(EpisodeCoverage::Episode {
                season: Some(2),
                episode: 3
            })
        );
        assert_eq!(
            recognize("Show.S02E03-E06"),
            Some(EpisodeCoverage::Range {
                season: Some(2),
                start: 3,
                end: 6
            })
        );
        assert_eq!(
            recognize("Show.EP07-EP09"),
            Some(EpisodeCoverage::Range {
                season: None,
                start: 7,
                end: 9
            })
        );
        assert_eq!(
            recognize("剧名 第3集-第6集"),
            Some(EpisodeCoverage::Range {
                season: None,
                start: 3,
                end: 6
            })
        );
        assert_eq!(
            recognize("Show.[03-06].1080p"),
            Some(EpisodeCoverage::Range {
                season: None,
                start: 3,
                end: 6
            })
        );
        assert_eq!(
            recognize("Show.S02.Complete"),
            Some(EpisodeCoverage::SeasonPack { season: 2 })
        );
        assert_eq!(recognize("Show.2026.1080p"), None);
    }
}
