//! A minimal HTTP interface, so providers can be tested without a network.

use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct HttpError(pub String);

pub trait Http: Send + Sync {
    fn get(&self, url: &str) -> Result<Response, HttpError>;
    fn post_json(&self, url: &str, body: &str) -> Result<Response, HttpError>;
    /// GET with extra request headers (for example an API key, which must never
    /// go into the URL). Implementations that cannot send headers fall back to
    /// a plain GET; [`ReqwestHttp`] sends them.
    fn get_with_headers(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, HttpError> {
        let _ = headers;
        self.get(url)
    }
    /// Wait between requests to respect provider rate limits.
    fn pause(&self, d: Duration);
}

/// Honest identification; Mizan never imitates a browser.
pub fn user_agent() -> String {
    format!(
        "Mizan/{} (+https://github.com/sercanatalik/mizan)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Real HTTP over rustls. Blocking: call it from a background thread.
pub struct ReqwestHttp {
    client: reqwest::blocking::Client,
}

impl ReqwestHttp {
    pub fn new() -> Result<Self, HttpError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(user_agent())
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| HttpError(e.to_string()))?;
        Ok(ReqwestHttp { client })
    }

    fn finish(r: reqwest::Result<reqwest::blocking::Response>) -> Result<Response, HttpError> {
        let r = r.map_err(|e| HttpError(e.to_string()))?;
        let status = r.status().as_u16();
        let body = r.text().map_err(|e| HttpError(e.to_string()))?;
        Ok(Response { status, body })
    }
}

impl Http for ReqwestHttp {
    fn get(&self, url: &str) -> Result<Response, HttpError> {
        Self::finish(self.client.get(url).send())
    }

    fn get_with_headers(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, HttpError> {
        let mut req = self.client.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        Self::finish(req.send())
    }

    fn post_json(&self, url: &str, body: &str) -> Result<Response, HttpError> {
        Self::finish(
            self.client
                .post(url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .body(body.to_owned())
                .send(),
        )
    }

    fn pause(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
