use serde_json::Value;

use super::http::{ClientError, PolicyClient, MTEAM_POLICY};

const MTEAM_BASE_URL: &str = "https://api.m-team.cc";
const MTEAM_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) tmdb-mteam-hub/0.1";

#[derive(Clone, Debug)]
pub struct MteamClient {
    http: PolicyClient,
    base_url: String,
}

impl MteamClient {
    pub fn new() -> Result<Self, ClientError> {
        Self::with_base_url(MTEAM_BASE_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, ClientError> {
        Ok(Self::with_http_client(
            base_url,
            PolicyClient::new(MTEAM_POLICY)?,
        ))
    }

    pub fn with_http_client(base_url: impl Into<String>, http: PolicyClient) -> Self {
        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub async fn search(&self, api_key: &str, body: &Value) -> Result<Value, ClientError> {
        if api_key.trim().is_empty() {
            return Err(ClientError::invalid_request(
                MTEAM_POLICY.provider,
                "API key is required",
            ));
        }
        let response = self
            .http
            .execute(
                self.http
                    .post(format!("{}/api/torrent/search", self.base_url))
                    .header("Accept", "application/json, text/plain, */*")
                    .header("Content-Type", "application/json")
                    .header("x-api-key", api_key.trim())
                    .header("Origin", "https://kp.m-team.cc/")
                    .header("User-Agent", MTEAM_USER_AGENT)
                    .json(body),
            )
            .await?;
        if !response.is_success() {
            return Err(ClientError::for_status(
                MTEAM_POLICY.provider,
                response.status,
            ));
        }
        serde_json::from_slice(&response.body)
            .map_err(|_| ClientError::protocol(MTEAM_POLICY.provider, "invalid JSON response"))
    }

    pub async fn fetch_download_url(
        &self,
        api_key: &str,
        torrent_id: &str,
    ) -> Result<String, ClientError> {
        let torrent_id = torrent_id.trim();
        if torrent_id.is_empty() {
            return Err(ClientError::invalid_request(
                MTEAM_POLICY.provider,
                "torrent id is required",
            ));
        }
        if api_key.trim().is_empty() {
            return Err(ClientError::invalid_request(
                MTEAM_POLICY.provider,
                "API key is required",
            ));
        }
        let form = reqwest::multipart::Form::new().text("id", torrent_id.to_string());
        let response = self
            .http
            .execute(
                self.http
                    .post(format!("{}/api/torrent/genDlToken", self.base_url))
                    .header("Accept", "application/json, text/plain, */*")
                    .header("x-api-key", api_key.trim())
                    .header("Origin", "https://kp.m-team.cc/")
                    .header("User-Agent", MTEAM_USER_AGENT)
                    .multipart(form),
            )
            .await?;
        if !response.is_success() {
            return Err(ClientError::for_status(
                MTEAM_POLICY.provider,
                response.status,
            ));
        }
        let value: Value = serde_json::from_slice(&response.body)
            .map_err(|_| ClientError::protocol(MTEAM_POLICY.provider, "invalid JSON response"))?;
        let code_ok = match value.get("code") {
            Some(Value::String(code)) => code == "0" || code == "200",
            Some(Value::Number(code)) => code.as_u64() == Some(0) || code.as_u64() == Some(200),
            _ => false,
        };
        if !code_ok {
            return Err(ClientError::protocol(
                MTEAM_POLICY.provider,
                "download token request was rejected",
            ));
        }
        let data = value.get("data").ok_or_else(|| {
            ClientError::protocol(MTEAM_POLICY.provider, "download URL is missing")
        })?;
        let url = data
            .as_str()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .ok_or_else(|| {
                ClientError::protocol(MTEAM_POLICY.provider, "download URL is invalid")
            })?;
        Ok(url.to_string())
    }

    pub fn policy(&self) -> super::http::HttpClientPolicy {
        self.http.policy()
    }
}
