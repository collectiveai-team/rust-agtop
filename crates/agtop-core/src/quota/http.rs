//! HTTP abstraction for quota provider fetches.
//!
//! The `HttpClient` trait isolates network I/O so providers can be tested
//! against a `FakeHttp` without touching real sockets. `UreqClient` is the
//! production implementation; it wraps a single `ureq::Agent` shared across
//! parallel fetches so HTTP keepalive works.

use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: Method::Get,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: Duration::from_secs(10),
        }
    }

    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            method: Method::Post,
            url: url.into(),
            headers: Vec::new(),
            body: Some(body),
            timeout: Duration::from_secs(10),
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum TransportError {
    Dns(String),
    Connect(String),
    Tls(String),
    Timeout,
    Io(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dns(m) => write!(f, "dns error: {m}"),
            Self::Connect(m) => write!(f, "connect error: {m}"),
            Self::Tls(m) => write!(f, "tls error: {m}"),
            Self::Timeout => write!(f, "timeout"),
            Self::Io(m) => write!(f, "io error: {m}"),
        }
    }
}

impl std::error::Error for TransportError {}

pub trait HttpClient: Send + Sync {
    fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// Production HTTP client backed by ureq. Holds one Agent shared across
/// concurrent calls for HTTP keepalive.
pub struct UreqClient {
    agent: ureq::Agent,
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqClient {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .user_agent(concat!("rust-agtop/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl HttpClient for UreqClient {
    fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        // Build the request with per-request timeout override.
        let mut resp = issue_request(&self.agent, &req, true)?;

        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|s| (k.as_str().to_owned(), s.to_owned()))
            })
            .collect();
        // read_to_vec() returns Result<Vec<u8>, ureq::Error>
        let body = resp.body_mut().read_to_vec().map_err(transport_err)?;

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// Issue a ureq request. When `treat_status_as_err` is true ureq will return
/// `Err(StatusCode(_))` for HTTP 4xx/5xx; we catch that and re-issue with
/// `http_status_as_error(false)` so we can surface the full response body.
fn issue_request(
    agent: &ureq::Agent,
    req: &HttpRequest,
    treat_status_as_err: bool,
) -> Result<ureq::http::Response<ureq::Body>, TransportError> {
    // GET and POST return different typestate variants from ureq, so we handle
    // them separately. Both paths: apply per-request config, add headers, then
    // call/send.
    let result = match req.method {
        Method::Get => {
            let mut builder = agent
                .get(&req.url)
                .config()
                .timeout_global(Some(req.timeout))
                .http_status_as_error(!treat_status_as_err)
                .build();
            for (k, v) in &req.headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            builder.call()
        }
        Method::Post => {
            let mut builder = agent
                .post(&req.url)
                .config()
                .timeout_global(Some(req.timeout))
                .http_status_as_error(!treat_status_as_err)
                .build();
            for (k, v) in &req.headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            match &req.body {
                Some(bytes) => builder.send(bytes.as_slice()),
                None => builder.send_empty(),
            }
        }
    };

    match result {
        Ok(r) => Ok(r),
        Err(ureq::Error::StatusCode(_)) if treat_status_as_err => {
            // Re-issue without treating status as error so we get the body.
            issue_request(agent, req, false)
        }
        Err(e) => Err(transport_err(e)),
    }
}

fn transport_err(e: ureq::Error) -> TransportError {
    match e {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        ureq::Error::HostNotFound => TransportError::Dns("host not found".to_owned()),
        ureq::Error::ConnectionFailed => TransportError::Connect("connection failed".to_owned()),
        ureq::Error::Tls(msg) => TransportError::Tls(msg.to_string()),
        // ureq 3 folds remaining errors (IO, protocol, etc.) into Io.
        other => TransportError::Io(other.to_string()),
    }
}

/// Extract an integer-seconds `Retry-After` value, if present.
/// Handles `Retry-After: 120` and HTTP-date form via `chrono`.
pub fn parse_retry_after(headers: &[(String, String)]) -> Option<u64> {
    let raw = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
        .map(|(_, v)| v.trim().to_owned())?;
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(secs);
    }
    // HTTP-date form. Parse with chrono.
    chrono::DateTime::parse_from_rfc2822(&raw).ok().map(|dt| {
        let now = chrono::Utc::now().timestamp();
        let target = dt.timestamp();
        (target - now).max(0) as u64
    })
}

/// Convert an HTTP response's status into an optional `QuotaError`.
/// Returns `None` for 2xx. Caller owns provider-id / provider-name attribution.
pub fn classify_response(resp: &HttpResponse) -> Option<crate::quota::types::QuotaError> {
    use crate::quota::types::{ErrorKind, QuotaError};
    match resp.status {
        200..=299 => None,
        429 => Some(QuotaError {
            kind: ErrorKind::Http {
                status: 429,
                retry_after: parse_retry_after(&resp.headers),
            },
            detail: truncate_body(&resp.body, 500),
        }),
        s => Some(QuotaError {
            kind: ErrorKind::Http {
                status: s,
                retry_after: None,
            },
            detail: truncate_body(&resp.body, 500),
        }),
    }
}

/// Truncate a byte slice to at most `max` bytes, replacing non-UTF-8 bodies
/// with a placeholder. Used for error detail strings shown in the TUI hover
/// popup.
pub fn truncate_body(body: &[u8], max: usize) -> String {
    match std::str::from_utf8(body) {
        Ok(s) => {
            if s.len() <= max {
                s.to_owned()
            } else {
                // Truncate at a char boundary <= max.
                let mut end = max;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &s[..end])
            }
        }
        Err(_) => format!("(non-UTF-8 response, {} bytes)", body.len()),
    }
}

/// Redact `Authorization` header values in-place. Used before logging request
/// headers in tracing events.
pub fn redact_auth_headers(headers: &mut [(String, String)]) {
    for (k, v) in headers.iter_mut() {
        if k.eq_ignore_ascii_case("authorization") {
            *v = String::from("<redacted>");
        }
    }
}

/// Test double. Queue canned responses with `push_ok` / `push_err`, then
/// `request()` pops them in FIFO order. Panics if asked for more responses
/// than queued — that catches test setup mistakes.
pub struct FakeHttp {
    responses: Mutex<std::collections::VecDeque<Result<HttpResponse, TransportError>>>,
    last_request: Mutex<Option<HttpRequest>>,
}

impl FakeHttp {
    pub fn new() -> Self {
        Self {
            responses: Mutex::new(std::collections::VecDeque::new()),
            last_request: Mutex::new(None),
        }
    }

    pub fn push_ok(&self, status: u16, body: &[u8]) {
        self.responses.lock().unwrap().push_back(Ok(HttpResponse {
            status,
            headers: vec![],
            body: body.to_vec(),
        }));
    }

    pub fn push_ok_with_headers(&self, status: u16, headers: Vec<(String, String)>, body: &[u8]) {
        self.responses.lock().unwrap().push_back(Ok(HttpResponse {
            status,
            headers,
            body: body.to_vec(),
        }));
    }

    pub fn push_err(&self, err: TransportError) {
        self.responses.lock().unwrap().push_back(Err(err));
    }

    pub fn last_request(&self) -> Option<HttpRequest> {
        self.last_request.lock().unwrap().clone()
    }
}

impl Default for FakeHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for FakeHttp {
    fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        *self.last_request.lock().unwrap() = Some(req.clone());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("FakeHttp has no queued response for request to {}", req.url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_http_returns_queued_responses_in_order() {
        let fake = FakeHttp::new();
        fake.push_ok(200, b"first");
        fake.push_ok(404, b"not found");

        let a = fake
            .request(HttpRequest::get("https://example.com/a"))
            .unwrap();
        assert_eq!(a.status, 200);
        assert_eq!(a.body, b"first");

        let b = fake
            .request(HttpRequest::get("https://example.com/b"))
            .unwrap();
        assert_eq!(b.status, 404);
    }

    #[test]
    fn fake_http_records_last_request() {
        let fake = FakeHttp::new();
        fake.push_ok(200, b"");
        let req = HttpRequest::get("https://example.com/x").header("X-Test", "yes");
        let _ = fake.request(req);
        let last = fake.last_request().unwrap();
        assert_eq!(last.url, "https://example.com/x");
        assert_eq!(
            last.headers,
            vec![("X-Test".to_string(), "yes".to_string())]
        );
    }

    #[test]
    fn fake_http_surfaces_transport_errors() {
        let fake = FakeHttp::new();
        fake.push_err(TransportError::Timeout);
        let err = fake
            .request(HttpRequest::get("https://example.com/"))
            .unwrap_err();
        assert!(matches!(err, TransportError::Timeout));
    }

    #[test]
    #[should_panic(expected = "no queued response")]
    fn fake_http_panics_when_unprepared() {
        let fake = FakeHttp::new();
        let _ = fake.request(HttpRequest::get("https://example.com/"));
    }

    #[test]
    fn retry_after_numeric_seconds() {
        let headers = vec![("Retry-After".to_string(), "120".to_string())];
        assert_eq!(parse_retry_after(&headers), Some(120));
    }

    #[test]
    fn retry_after_missing_yields_none() {
        assert_eq!(parse_retry_after(&[]), None);
    }

    #[test]
    fn truncate_utf8() {
        let s = truncate_body(b"hello world", 5);
        assert_eq!(s, "hello...");
    }

    #[test]
    fn truncate_non_utf8_uses_placeholder() {
        let s = truncate_body(&[0xffu8, 0xfe, 0xfd], 50);
        assert!(s.contains("non-UTF-8"));
    }

    #[test]
    fn classify_response_maps_statuses() {
        let ok = HttpResponse {
            status: 200,
            headers: vec![],
            body: b"{}".to_vec(),
        };
        assert!(classify_response(&ok).is_none());

        let err401 = HttpResponse {
            status: 401,
            headers: vec![],
            body: b"nope".to_vec(),
        };
        let q = classify_response(&err401).unwrap();
        match q.kind {
            crate::quota::types::ErrorKind::Http {
                status,
                retry_after,
            } => {
                assert_eq!(status, 401);
                assert!(retry_after.is_none());
            }
            _ => panic!("wrong kind"),
        }

        let err429 = HttpResponse {
            status: 429,
            headers: vec![("Retry-After".to_string(), "30".to_string())],
            body: b"slow down".to_vec(),
        };
        let q = classify_response(&err429).unwrap();
        match q.kind {
            crate::quota::types::ErrorKind::Http {
                status,
                retry_after,
            } => {
                assert_eq!(status, 429);
                assert_eq!(retry_after, Some(30));
            }
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn redacts_authorization_header() {
        let mut h = vec![
            (
                "Authorization".to_string(),
                "Bearer sk-secret-token".to_string(),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        redact_auth_headers(&mut h);
        assert_eq!(h[0].1, "<redacted>");
        assert_eq!(h[1].1, "application/json");
    }
}
