use std::fmt;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;

pub const MIB: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpClientPolicy {
    pub provider: &'static str,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub redirect_limit: usize,
    pub response_body_limit: usize,
}

impl HttpClientPolicy {
    pub const fn new(
        provider: &'static str,
        connect_timeout: Duration,
        request_timeout: Duration,
        redirect_limit: usize,
        response_body_limit: usize,
    ) -> Self {
        Self {
            provider,
            connect_timeout,
            request_timeout,
            redirect_limit,
            response_body_limit,
        }
    }
}

pub const MTEAM_POLICY: HttpClientPolicy = HttpClientPolicy::new(
    "M-Team",
    Duration::from_secs(5),
    Duration::from_secs(30),
    5,
    2 * MIB,
);
pub const QB_POLICY: HttpClientPolicy = HttpClientPolicy::new(
    "qBittorrent",
    Duration::from_secs(5),
    Duration::from_secs(30),
    15,
    16 * MIB,
);
pub const TMDB_POLICY: HttpClientPolicy = HttpClientPolicy::new(
    "TMDB",
    Duration::from_secs(5),
    Duration::from_secs(30),
    5,
    8 * MIB,
);
pub const DOUBAN_POLICY: HttpClientPolicy = HttpClientPolicy::new(
    "Douban",
    Duration::from_secs(5),
    Duration::from_secs(30),
    10,
    8 * MIB,
);
pub const DOUBAN_LIMITED_REDIRECT_POLICY: HttpClientPolicy = HttpClientPolicy::new(
    "Douban",
    Duration::from_secs(5),
    Duration::from_secs(30),
    3,
    16 * MIB,
);
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    InvalidRequest {
        provider: &'static str,
        message: String,
    },
    Timeout {
        provider: &'static str,
    },
    BodyTooLarge {
        provider: &'static str,
        limit: usize,
    },
    Authentication {
        provider: &'static str,
    },
    RateLimited {
        provider: &'static str,
    },
    Protocol {
        provider: &'static str,
        message: String,
    },
    Unavailable {
        provider: &'static str,
        message: String,
    },
}

impl ClientError {
    pub fn invalid_request(provider: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidRequest {
            provider,
            message: message.into(),
        }
    }

    pub fn protocol(provider: &'static str, message: impl Into<String>) -> Self {
        Self::Protocol {
            provider,
            message: message.into(),
        }
    }

    pub fn unavailable(provider: &'static str, message: impl Into<String>) -> Self {
        Self::Unavailable {
            provider,
            message: message.into(),
        }
    }

    pub fn for_status(provider: &'static str, status: u16) -> Self {
        match status {
            401 | 403 => Self::Authentication { provider },
            429 => Self::RateLimited { provider },
            500..=599 => Self::Unavailable {
                provider,
                message: format!("upstream returned HTTP {status}"),
            },
            _ => Self::Protocol {
                provider,
                message: format!("unexpected upstream HTTP {status}"),
            },
        }
    }

    pub fn provider(&self) -> &'static str {
        match self {
            Self::InvalidRequest { provider, .. }
            | Self::Timeout { provider }
            | Self::BodyTooLarge { provider, .. }
            | Self::Authentication { provider }
            | Self::RateLimited { provider }
            | Self::Protocol { provider, .. }
            | Self::Unavailable { provider, .. } => provider,
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }

    fn from_reqwest(provider: &'static str, error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout { provider }
        } else if error.is_redirect() {
            Self::Protocol {
                provider,
                message: "redirect policy rejected the response".to_string(),
            }
        } else {
            // Do not retain Reqwest's URL-bearing error text. URLs can contain API keys.
            Self::Unavailable {
                provider,
                message: "transport request failed".to_string(),
            }
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest { provider, message } => write!(f, "{provider}: {message}"),
            Self::Timeout { provider } => write!(f, "{provider} request timed out"),
            Self::BodyTooLarge { provider, limit } => {
                write!(f, "{provider} response exceeded the {limit}-byte limit")
            }
            Self::Authentication { provider } => {
                write!(f, "{provider} rejected authentication")
            }
            Self::RateLimited { provider } => write!(f, "{provider} rate limited the request"),
            Self::Protocol { provider, message } => write!(f, "{provider}: {message}"),
            Self::Unavailable { provider, message } => write!(f, "{provider}: {message}"),
        }
    }
}

impl std::error::Error for ClientError {}

#[derive(Clone)]
pub struct ClientResponse {
    pub status: u16,
    pub final_url: String,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl ClientResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self, provider: &'static str) -> Result<&str, ClientError> {
        std::str::from_utf8(&self.body)
            .map_err(|_| ClientError::protocol(provider, "upstream response was not UTF-8"))
    }
}

#[derive(Clone, Debug)]
pub struct PolicyClient {
    policy: HttpClientPolicy,
    inner: reqwest::Client,
}

impl PolicyClient {
    pub fn new(policy: HttpClientPolicy) -> Result<Self, ClientError> {
        Self::with_builder(policy, |builder| builder)
    }

    pub fn with_builder(
        policy: HttpClientPolicy,
        configure: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
    ) -> Result<Self, ClientError> {
        let builder = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(policy.connect_timeout)
            .timeout(policy.request_timeout)
            .redirect(reqwest::redirect::Policy::limited(policy.redirect_limit));
        let inner = configure(builder).build().map_err(|_| {
            ClientError::unavailable(policy.provider, "failed to construct HTTP client")
        })?;
        Ok(Self { policy, inner })
    }

    pub fn policy(&self) -> HttpClientPolicy {
        self.policy
    }

    pub fn get(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.inner.get(url)
    }

    pub fn post(&self, url: impl reqwest::IntoUrl) -> reqwest::RequestBuilder {
        self.inner.post(url)
    }

    pub async fn execute(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<ClientResponse, ClientError> {
        let response = request
            .send()
            .await
            .map_err(|error| ClientError::from_reqwest(self.policy.provider, error))?;
        let status = response.status().as_u16();
        let final_url = redacted_response_url(response.url());
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
            .filter(|value| !value.is_empty());
        let body = read_bounded_response(response, self.policy).await?;
        Ok(ClientResponse {
            status,
            final_url,
            content_type,
            body,
        })
    }
}

fn redacted_response_url(url: &reqwest::Url) -> String {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}

pub async fn read_bounded_response(
    mut response: reqwest::Response,
    policy: HttpClientPolicy,
) -> Result<Vec<u8>, ClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > policy.response_body_limit as u64)
    {
        return Err(ClientError::BodyTooLarge {
            provider: policy.provider,
            limit: policy.response_body_limit,
        });
    }

    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(policy.response_body_limit as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ClientError::from_reqwest(policy.provider, error))?
    {
        let next_len = body.len().saturating_add(chunk.len());
        if next_len > policy.response_body_limit {
            return Err(ClientError::BodyTooLarge {
                provider: policy.provider,
                limit: policy.response_body_limit,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_failures_are_normalized_without_response_content() {
        assert_eq!(
            ClientError::for_status("provider", 401),
            ClientError::Authentication {
                provider: "provider"
            }
        );
        assert_eq!(
            ClientError::for_status("provider", 429),
            ClientError::RateLimited {
                provider: "provider"
            }
        );
        assert!(matches!(
            ClientError::for_status("provider", 503),
            ClientError::Unavailable { .. }
        ));
        assert!(matches!(
            ClientError::for_status("provider", 418),
            ClientError::Protocol { .. }
        ));
    }

    #[test]
    fn named_policies_never_rely_on_zero_or_unlimited_defaults() {
        for policy in [
            MTEAM_POLICY,
            QB_POLICY,
            TMDB_POLICY,
            DOUBAN_POLICY,
            DOUBAN_LIMITED_REDIRECT_POLICY,
        ] {
            assert!(!policy.provider.is_empty());
            assert!(!policy.connect_timeout.is_zero());
            assert!(!policy.request_timeout.is_zero());
            assert!(policy.redirect_limit > 0);
            assert!(policy.response_body_limit > 0);
        }
    }

    #[test]
    fn response_url_metadata_drops_credentials_query_and_fragment() {
        let url = reqwest::Url::parse(
            "https://inline-user:inline-password@example.test/movie/1?api_key=SECRET#fragment",
        )
        .unwrap();

        let redacted = redacted_response_url(&url);

        assert_eq!(redacted, "https://example.test/movie/1");
        assert!(!redacted.contains("inline-user"));
        assert!(!redacted.contains("inline-password"));
        assert!(!redacted.contains("SECRET"));
    }
}
