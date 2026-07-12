use crate::config::FileConfig;
use std::fmt;

const REDACTED: &str = "[REDACTED]";
const MAX_DIAGNOSTIC_CHARS: usize = 1_000;

/// Config-aware redaction for subscription diagnostics crossing an HTTP or
/// operation-log boundary.
#[derive(Clone)]
pub(crate) struct SubscriptionDiagnosticRedactor {
    secrets: Vec<String>,
}

impl fmt::Debug for SubscriptionDiagnosticRedactor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionDiagnosticRedactor")
            .field("secret_count", &self.secrets.len())
            .finish()
    }
}

impl SubscriptionDiagnosticRedactor {
    pub(crate) fn from_config(config: &FileConfig) -> Self {
        let mut secrets = Vec::new();
        add_configured_secret(&mut secrets, &config.tmdb_api_key);
        add_configured_secret(&mut secrets, &config.mteam_api_key);
        add_configured_secret(&mut secrets, &config.douban_cookie);
        add_configured_secret(&mut secrets, &config.management.admin_token);
        for server in &config.qb_servers {
            add_configured_secret(&mut secrets, &server.password);
        }
        for component in config.douban_cookie.split(';') {
            let Some((_, value)) = component.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            if value.chars().count() >= 4 {
                secrets.push(value.to_string());
            }
        }
        secrets.sort_unstable_by(|left, right| {
            right.len().cmp(&left.len()).then_with(|| left.cmp(right))
        });
        secrets.dedup();
        Self { secrets }
    }

    pub(crate) fn redact(&self, value: &str) -> String {
        // Parse URL structure before configured replacement. Even a legitimate short secret such
        // as `h`, `passkey`, `@`, or `://` must not be allowed to destroy URL delimiters before we
        // have removed unconfigured userinfo and sensitive query values.
        let urls = redact_embedded_urls(value);
        let configured = redact_configured_secrets(&urls, &self.secrets);
        configured
            .trim()
            .chars()
            .take(MAX_DIAGNOSTIC_CHARS)
            .collect()
    }

    pub(crate) fn redact_or(&self, value: &str, fallback: &str) -> String {
        let redacted = self.redact(value);
        if redacted.is_empty() {
            fallback.to_string()
        } else {
            redacted
        }
    }
}

fn add_configured_secret(secrets: &mut Vec<String>, value: &str) {
    if !value.is_empty() {
        secrets.push(value.to_string());
    }
    let trimmed = value.trim();
    if !trimmed.is_empty() && trimmed != value {
        secrets.push(trimmed.to_string());
    }
}

fn redact_configured_secrets(value: &str, secrets: &[String]) -> String {
    let mut output = String::with_capacity(value.len());
    let mut offset = 0;
    while offset < value.len() {
        let remaining = &value[offset..];
        if let Some(secret) = secrets
            .iter()
            .find(|secret| remaining.starts_with(secret.as_str()))
        {
            output.push_str(REDACTED);
            offset += secret.len();
            continue;
        }
        let next = remaining
            .chars()
            .next()
            .expect("offset remains on a character boundary before the end");
        output.push(next);
        offset += next.len_utf8();
    }
    output
}

fn redact_embedded_urls(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut copied_until = 0;
    let mut search_from = 0;
    while let Some(delimiter_offset) = value[search_from..].find("://") {
        let delimiter = search_from + delimiter_offset;
        let Some(start) = url_scheme_start(value, delimiter) else {
            search_from = delimiter + 3;
            continue;
        };
        let end = url_end(value, delimiter + 3);
        let (url_end, suffix_start) = trim_url_suffix(value, start, end);
        output.push_str(&value[copied_until..start]);
        output.push_str(&redact_url(&value[start..url_end]));
        output.push_str(&value[url_end..suffix_start]);
        copied_until = suffix_start;
        search_from = suffix_start;
    }
    output.push_str(&value[copied_until..]);
    output
}

fn url_scheme_start(value: &str, delimiter: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut start = delimiter;
    while start > 0 && is_scheme_byte(bytes[start - 1]) {
        start -= 1;
    }
    let scheme = bytes.get(start..delimiter)?;
    (!scheme.is_empty() && scheme[0].is_ascii_alphabetic()).then_some(start)
}

const fn is_scheme_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'+' | b'-' | b'.')
}

fn url_end(value: &str, start: usize) -> usize {
    for (offset, character) in value[start..].char_indices() {
        if character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>') {
            return start + offset;
        }
    }
    value.len()
}

fn trim_url_suffix(value: &str, start: usize, mut end: usize) -> (usize, usize) {
    let suffix_start = end;
    while end > start {
        let character = value[..end]
            .chars()
            .next_back()
            .expect("non-empty URL candidate has a final character");
        if !matches!(character, ',' | '.' | ')' | ']' | '}') {
            break;
        }
        end -= character.len_utf8();
    }
    (end, suffix_start)
}

fn redact_url(value: &str) -> String {
    let Some(scheme_end) = value.find("://").map(|offset| offset + 3) else {
        return value.to_string();
    };
    let authority_end = value[scheme_end..]
        .find(['/', '?', '#'])
        .map_or(value.len(), |offset| scheme_end + offset);
    let authority = &value[scheme_end..authority_end];
    let mut output = String::with_capacity(value.len());
    output.push_str(&value[..scheme_end]);
    if let Some(at) = authority.rfind('@') {
        output.push_str(REDACTED);
        output.push('@');
        output.push_str(&authority[at + 1..]);
    } else {
        output.push_str(authority);
    }

    let remainder = &value[authority_end..];
    let Some(query_start) = remainder.find('?') else {
        output.push_str(remainder);
        return output;
    };
    let query_value_start = query_start + 1;
    let query_end = remainder[query_value_start..]
        .find('#')
        .map_or(remainder.len(), |offset| query_value_start + offset);
    output.push_str(&remainder[..query_value_start]);
    output.push_str(&redact_query(&remainder[query_value_start..query_end]));
    output.push_str(&remainder[query_end..]);
    output
}

fn redact_query(query: &str) -> String {
    let mut output = String::with_capacity(query.len());
    let mut start = 0;
    for (offset, delimiter) in query.match_indices(['&', ';']) {
        output.push_str(&redact_query_component(&query[start..offset]));
        output.push_str(delimiter);
        start = offset + delimiter.len();
    }
    output.push_str(&redact_query_component(&query[start..]));
    output
}

fn redact_query_component(component: &str) -> String {
    let Some((key, _)) = component.split_once('=') else {
        return component.to_string();
    };
    if !is_secret_query_key(key) {
        return component.to_string();
    }
    format!("{key}={REDACTED}")
}

fn is_secret_query_key(key: &str) -> bool {
    let decoded = percent_decode_ascii(key);
    let normalized = decoded.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    let compact = normalized.replace('_', "");
    matches!(
        normalized.as_str(),
        "key"
            | "apikey"
            | "api_key"
            | "passkey"
            | "password"
            | "passwd"
            | "pwd"
            | "secret"
            | "client_secret"
            | "token"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "auth"
            | "authorization"
            | "cookie"
            | "session"
            | "session_id"
            | "signature"
            | "sig"
    ) || normalized.ends_with("_key")
        || normalized.ends_with("_token")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_password")
        || normalized.contains("credential")
        || normalized.contains("signature")
        || normalized.contains("authorization")
        || normalized.starts_with("x_amz_security_token")
        || matches!(
            compact.as_str(),
            "apikey"
                | "passkey"
                | "password"
                | "clientsecret"
                | "token"
                | "accesstoken"
                | "refreshtoken"
                | "idtoken"
                | "authtoken"
                | "authorization"
                | "sessionid"
                | "signature"
        )
        || compact.ends_with("token")
        || compact.ends_with("secret")
        || compact.ends_with("password")
}

fn percent_decode_ascii(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'%' && offset + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (
                decode_hex_nibble(bytes[offset + 1]),
                decode_hex_nibble(bytes[offset + 2]),
            ) {
                output.push(char::from((high << 4) | low));
                offset += 3;
                continue;
            }
        }
        let next = value[offset..]
            .chars()
            .next()
            .expect("offset remains before the end of the query key");
        output.push(next);
        offset += next.len_utf8();
    }
    output
}

const fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{SubscriptionDiagnosticRedactor, MAX_DIAGNOSTIC_CHARS, REDACTED};
    use crate::config::{FileConfig, ManagementConfig, QbServerEntry};

    fn config() -> FileConfig {
        FileConfig {
            tmdb_api_key: "shared-secret-long".to_string(),
            mteam_api_key: "shared-secret".to_string(),
            douban_cookie: "dbcl2=DOUBAN_COMPONENT; ck=abcd; short=xyz".to_string(),
            management: ManagementConfig {
                admin_token: "MANAGEMENT_SECRET_123456789".to_string(),
                ..ManagementConfig::default()
            },
            qb_servers: vec![QbServerEntry {
                id: "nas".to_string(),
                name: "NAS".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: "QB_PASSWORD".to_string(),
                insecure_tls: false,
            }],
            ..FileConfig::default()
        }
    }

    #[test]
    fn configured_secrets_and_cookie_components_are_redacted_longest_first() {
        let config = config();
        let redactor = SubscriptionDiagnosticRedactor::from_config(&config);
        let message = format!(
            "{} | shared-secret-long | shared-secret | DOUBAN_COMPONENT | abcd | xyz | MANAGEMENT_SECRET_123456789 | QB_PASSWORD",
            config.douban_cookie
        );

        let redacted = redactor.redact(&message);

        for secret in [
            config.douban_cookie.as_str(),
            "shared-secret-long",
            "shared-secret",
            "DOUBAN_COMPONENT",
            "abcd",
            "MANAGEMENT_SECRET_123456789",
            "QB_PASSWORD",
        ] {
            assert!(!redacted.contains(secret), "leaked {secret}");
        }
        assert!(
            redacted.contains("xyz"),
            "short cookie fragments stay diagnostic"
        );
        assert!(redacted.contains(REDACTED));
        assert!(!redacted.contains("[REDACTED]-long"));
        let debug = format!("{redactor:?}");
        assert!(debug.contains("secret_count"));
        assert!(!debug.contains("shared-secret"));
    }

    #[test]
    fn embedded_url_userinfo_and_secret_query_values_are_redacted() {
        let redactor = SubscriptionDiagnosticRedactor::from_config(&FileConfig::default());
        let message = "failed (https://user:password@example.test/download?passkey=abc&file=movie&X-Amz-Signature=siggy&clientSecret=camel), then https://TOKENVALUE@example.test/path?access%5Ftoken=encoded&safe=ok#frag";

        let redacted = redactor.redact(message);

        for secret in [
            "user:password",
            "abc",
            "siggy",
            "camel",
            "TOKENVALUE",
            "encoded",
        ] {
            assert!(!redacted.contains(secret), "URL leaked {secret}");
        }
        assert!(redacted.contains("https://[REDACTED]@example.test"));
        assert!(redacted.contains("file=movie"));
        assert!(redacted.contains("safe=ok#frag"));
        assert!(redacted.ends_with("#frag"));
    }

    #[test]
    fn url_structure_is_redacted_before_short_or_syntax_shaped_configured_secrets() {
        let config = FileConfig {
            tmdb_api_key: "h".to_string(),
            mteam_api_key: "passkey".to_string(),
            management: ManagementConfig {
                admin_token: "://".to_string(),
                ..ManagementConfig::default()
            },
            qb_servers: vec![QbServerEntry {
                id: "syntax".to_string(),
                name: "Syntax".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: "configured:p@ss".to_string(),
                insecure_tls: false,
            }],
            ..FileConfig::default()
        };
        let redactor = SubscriptionDiagnosticRedactor::from_config(&config);
        let message = concat!(
            "configured configured:p@ss; ",
            "https://dynamic-user:dynamic-password@example.test/download?",
            "passkey=DYNAMIC_PASSKEY&access_token=DYNAMIC_ACCESS_TOKEN&safe=ok; ",
            "custom://other-user:other-password@example.test/path?refresh-token=OTHER_TOKEN"
        );

        let redacted = redactor.redact(message);

        for secret in [
            "configured:p@ss",
            "dynamic-user",
            "dynamic-password",
            "DYNAMIC_PASSKEY",
            "DYNAMIC_ACCESS_TOKEN",
            "other-user",
            "other-password",
            "OTHER_TOKEN",
        ] {
            assert!(
                !redacted.contains(secret),
                "combined redaction leaked {secret}"
            );
        }
        assert!(redacted.contains("safe=ok"));
        assert!(redacted.contains(REDACTED));
    }

    #[test]
    fn output_is_trimmed_capped_and_can_use_a_fallback() {
        let redactor = SubscriptionDiagnosticRedactor::from_config(&FileConfig::default());
        let input = format!("  {}  ", "界".repeat(MAX_DIAGNOSTIC_CHARS + 50));

        let redacted = redactor.redact(&input);

        assert_eq!(redacted.chars().count(), MAX_DIAGNOSTIC_CHARS);
        assert!(!redacted.starts_with(char::is_whitespace));
        assert!(!redacted.ends_with(char::is_whitespace));
        assert_eq!(redactor.redact_or("   ", "fallback"), "fallback");
    }
}
