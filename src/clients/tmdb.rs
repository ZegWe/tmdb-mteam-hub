use serde_json::Value;

use super::http::{ClientError, PolicyClient, TMDB_POLICY};

const TMDB_BASE_URL: &str = "https://api.themoviedb.org/3";

#[derive(Clone, Debug)]
pub struct TmdbClient {
    http: PolicyClient,
    base_url: String,
}

impl TmdbClient {
    pub fn new() -> Result<Self, ClientError> {
        Self::with_base_url(TMDB_BASE_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, ClientError> {
        Ok(Self::with_http_client(
            base_url,
            PolicyClient::new(TMDB_POLICY)?,
        ))
    }

    pub fn with_http_client(base_url: impl Into<String>, http: PolicyClient) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn get_json(
        &self,
        credential: &str,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, ClientError> {
        if credential.trim().is_empty() {
            return Err(ClientError::invalid_request(
                TMDB_POLICY.provider,
                "API credential is required",
            ));
        }
        if !path.starts_with('/') {
            return Err(ClientError::invalid_request(
                TMDB_POLICY.provider,
                "API path must start with a slash",
            ));
        }
        let mut request = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .header("Accept", "application/json");
        if uses_bearer_token(credential) {
            request = request.bearer_auth(credential.trim());
        } else {
            request = request.query(&[("api_key", credential.trim())]);
        }
        request = request.query(query);
        let response = self.http.execute(request).await?;
        if !response.is_success() {
            return Err(ClientError::for_status(
                TMDB_POLICY.provider,
                response.status,
            ));
        }
        serde_json::from_slice(&response.body)
            .map_err(|_| ClientError::protocol(TMDB_POLICY.provider, "invalid JSON response"))
    }

    pub fn policy(&self) -> super::http::HttpClientPolicy {
        self.http.policy()
    }
}

pub fn uses_bearer_token(credential: &str) -> bool {
    let credential = credential.trim();
    credential.starts_with("eyJ") && credential.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_like_read_tokens_use_authorization_while_api_keys_use_query_auth() {
        assert!(uses_bearer_token("eyJheader.payload.signature"));
        assert!(!uses_bearer_token("legacy-api-key"));
    }
}
