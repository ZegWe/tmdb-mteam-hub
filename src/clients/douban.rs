use std::sync::Arc;

use reqwest::cookie::{CookieStore, Jar};
use reqwest::Url;

use super::http::{
    ClientError, ClientResponse, HttpClientPolicy, PolicyClient, DOUBAN_LIMITED_REDIRECT_POLICY,
    DOUBAN_POLICY,
};

const DESKTOP_CHROME_UA: &str = concat!(
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 ",
    "(KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36"
);
const LOGIN_REFERER: &str = "https://accounts.douban.com/passport/login";
const IMAGE_REFERER: &str = "https://movie.douban.com/";

#[derive(Clone, Debug)]
pub struct DoubanClient {
    web: PolicyClient,
    limited_redirect: PolicyClient,
}

#[derive(Clone)]
pub(crate) struct DoubanSession {
    http: PolicyClient,
    jar: Arc<Jar>,
}

impl DoubanClient {
    pub fn new() -> Result<Self, ClientError> {
        Ok(Self {
            web: PolicyClient::with_builder(DOUBAN_POLICY, |builder| {
                builder.user_agent(DESKTOP_CHROME_UA)
            })?,
            limited_redirect: PolicyClient::with_builder(
                DOUBAN_LIMITED_REDIRECT_POLICY,
                |builder| builder.user_agent(DESKTOP_CHROME_UA),
            )?,
        })
    }

    pub(crate) fn isolated_cookie_session(&self) -> Result<DoubanSession, ClientError> {
        let jar = Arc::new(Jar::default());
        let client = PolicyClient::with_builder(DOUBAN_POLICY, {
            let jar = jar.clone();
            move |builder| builder.user_agent(DESKTOP_CHROME_UA).cookie_provider(jar)
        })?;
        Ok(DoubanSession { http: client, jar })
    }

    pub fn policies(&self) -> [HttpClientPolicy; 2] {
        [self.web.policy(), self.limited_redirect.policy()]
    }

    pub(crate) async fn rexxar_json(
        &self,
        url: Url,
        cookie_header: &str,
        referer: &str,
    ) -> Result<ClientResponse, ClientError> {
        let mut request = self
            .web
            .get(url)
            .header("Accept", "application/json")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .header("Origin", "https://m.douban.com")
            .header("Referer", referer);
        if !cookie_header.trim().is_empty() {
            request = request.header("Cookie", cookie_header);
        }
        self.web.execute(request).await
    }

    pub(crate) async fn html(
        &self,
        url: &str,
        cookie_header: &str,
        referer: &str,
    ) -> Result<ClientResponse, ClientError> {
        self.web
            .execute(
                self.web
                    .get(url)
                    .header(
                        "Accept",
                        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                    )
                    .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                    .header("Referer", referer)
                    .header("Cookie", cookie_header),
            )
            .await
    }

    pub(crate) async fn mark_interest(
        &self,
        url: &str,
        detail_url: &str,
        cookie_header: &str,
        form: &[(&str, &str)],
    ) -> Result<ClientResponse, ClientError> {
        self.limited_redirect
            .execute(
                self.limited_redirect
                    .post(url)
                    .header("Accept", "application/json, text/javascript, */*; q=0.01")
                    .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                    .header("Referer", detail_url)
                    .header("X-Requested-With", "XMLHttpRequest")
                    .header("Cookie", cookie_header)
                    .form(form),
            )
            .await
    }

    pub(crate) async fn image(&self, url: Url) -> Result<ClientResponse, ClientError> {
        self.limited_redirect
            .execute(
                self.limited_redirect
                    .get(url)
                    .header(
                        "Accept",
                        "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
                    )
                    .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                    .header("Referer", IMAGE_REFERER),
            )
            .await
    }
}

impl DoubanSession {
    pub(crate) async fn request_json(&self, url: &str) -> Result<ClientResponse, ClientError> {
        self.http
            .execute(
                self.http
                    .get(url)
                    .header("Accept", "application/json, text/javascript, */*; q=0.01")
                    .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                    .header("Referer", LOGIN_REFERER)
                    .header("X-Requested-With", "XMLHttpRequest"),
            )
            .await
    }

    pub(crate) async fn qr_image(&self, url: &str) -> Result<ClientResponse, ClientError> {
        self.http
            .execute(
                self.http
                    .get(url)
                    .header("Accept", "image/png,image/*,*/*;q=0.8")
                    .header("Referer", LOGIN_REFERER),
            )
            .await
    }

    pub(crate) fn cookie_header(&self, base_url: &str) -> Result<String, ClientError> {
        let url = Url::parse(base_url)
            .map_err(|_| ClientError::protocol(DOUBAN_POLICY.provider, "invalid cookie URL"))?;
        let Some(value) = self.jar.cookies(&url) else {
            return Ok(String::new());
        };
        value
            .to_str()
            .map(str::to_string)
            .map_err(|_| ClientError::protocol(DOUBAN_POLICY.provider, "invalid cookie header"))
    }
}
