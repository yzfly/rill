//! Transport-independent policy. Request and response bodies never enter this crate.
use http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use url::Url;

pub const DEFAULT_UPSTREAM: &str = "https://api.openai.com";

#[derive(Clone)]
pub struct Config {
    upstream: Url,
}

impl Config {
    /// Only operators can select the origin. Incoming paths cannot change it.
    pub fn new(origin: &str) -> Result<Self, &'static str> {
        let upstream = Url::parse(origin).map_err(|_| "RILL_UPSTREAM must be an HTTP(S) origin")?;
        if !matches!(upstream.scheme(), "http" | "https")
            || upstream.host_str().is_none()
            || !upstream.username().is_empty()
            || upstream.password().is_some()
            || upstream.path() != "/"
            || upstream.query().is_some()
            || upstream.fragment().is_some()
        {
            return Err(
                "RILL_UPSTREAM must be an HTTP(S) origin without credentials, path, query or fragment",
            );
        }
        Ok(Self { upstream })
    }

    pub fn target(&self, uri: &Uri) -> Option<Url> {
        if !uri.path().starts_with("/v1/") || uri.path().contains('\\') {
            return None;
        }
        let mut target = self.upstream.clone();
        target.set_path(uri.path());
        target.set_query(uri.query());
        // URL canonicalization must not allow dot segments to escape the API prefix.
        target.path().starts_with("/v1/").then_some(target)
    }
}

pub struct LocalResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: &'static str,
}

pub enum Decision {
    Local(LocalResponse),
    Forward { target: Url, headers: HeaderMap },
}

pub fn prepare(config: &Config, method: &Method, uri: &Uri, headers: &HeaderMap) -> Decision {
    if matches!(*method, Method::GET | Method::HEAD) && uri.path() == "/health" {
        return Decision::Local(local(
            StatusCode::OK,
            "{\"status\":\"ok\",\"service\":\"rill\"}",
        ));
    }
    let Some(target) = config.target(uri) else {
        return Decision::Local(local(
            StatusCode::NOT_FOUND,
            "{\"error\":{\"message\":\"Use /v1/* for API requests.\",\"type\":\"invalid_request_error\"}}",
        ));
    };
    if method == Method::OPTIONS {
        let mut response = local(StatusCode::NO_CONTENT, "");
        response.headers.insert(
            "access-control-allow-methods",
            HeaderValue::from_static("GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS"),
        );
        // No cookies / credentialed CORS: clients explicitly supply their API key.
        if let Some(value) = headers.get("access-control-request-headers") {
            response
                .headers
                .insert("access-control-allow-headers", value.clone());
        }
        response
            .headers
            .insert("access-control-max-age", HeaderValue::from_static("600"));
        return Decision::Local(response);
    }
    if !matches!(
        *method,
        Method::GET | Method::HEAD | Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) {
        return Decision::Local(local(
            StatusCode::METHOD_NOT_ALLOWED,
            "{\"error\":{\"message\":\"Method not allowed.\",\"type\":\"invalid_request_error\"}}",
        ));
    }
    // Only WebSocket is refused. Other Upgrade offers (such as curl's h2c) are
    // hop-by-hop, so they are stripped and the request is served as plain HTTP.
    let websocket = headers
        .get_all("upgrade")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.trim().eq_ignore_ascii_case("websocket"));
    if websocket {
        return Decision::Local(local(
            StatusCode::NOT_IMPLEMENTED,
            "{\"error\":{\"message\":\"Use HTTP/SSE; WebSocket forwarding is not supported.\",\"type\":\"unsupported_transport\"}}",
        ));
    }
    let authorized = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .is_some_and(|(scheme, key)| {
            scheme.eq_ignore_ascii_case("bearer")
                && !key.is_empty()
                && !key.contains(char::is_whitespace)
        });
    if !authorized {
        return Decision::Local(local(
            StatusCode::UNAUTHORIZED,
            "{\"error\":{\"message\":\"Supply your OpenAI API key as Authorization: Bearer <key>.\",\"type\":\"authentication_error\"}}",
        ));
    }
    Decision::Forward {
        target,
        headers: request_headers(headers),
    }
}

pub fn local(status: StatusCode, body: &'static str) -> LocalResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response_headers_in_place(&mut headers);
    LocalResponse {
        status,
        headers,
        body,
    }
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let nominated: Vec<String> = headers
        .get_all("connection")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(|s| s.trim().to_owned()))
        .collect();
    for name in nominated {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

pub fn request_headers(original: &HeaderMap) -> HeaderMap {
    let mut headers = original.clone();
    strip_hop_headers(&mut headers);
    let remove: Vec<_> = headers
        .keys()
        .filter(|name| {
            let name = name.as_str();
            name.starts_with("cf-")
                || name.starts_with("x-forwarded-")
                || matches!(
                    name,
                    "host" | "cookie" | "origin" | "referer" | "forwarded" | "x-real-ip"
                )
        })
        .cloned()
        .collect();
    for name in remove {
        headers.remove(name);
    }
    // Native and Worker transports both forward bytes without decompressing/re-encoding.
    headers.insert("accept-encoding", HeaderValue::from_static("identity"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers
}

pub fn response_headers_in_place(headers: &mut HeaderMap) {
    strip_hop_headers(headers);
    for name in [
        "set-cookie",
        "access-control-allow-credentials",
        "content-length",
    ] {
        headers.remove(name);
    }
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert("access-control-expose-headers", HeaderValue::from_static("x-request-id, openai-processing-ms, openai-version, retry-after, x-ratelimit-limit-requests, x-ratelimit-remaining-requests, x-ratelimit-reset-requests, x-ratelimit-limit-tokens, x-ratelimit-remaining-tokens, x-ratelimit-reset-tokens"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_cannot_be_changed_by_request() {
        let config = Config::new(DEFAULT_UPSTREAM).unwrap();
        for path in [
            "//evil.example/v1/models",
            "/v1/../admin",
            "/v1/%2e%2e/admin",
            "/other",
        ] {
            assert!(config.target(&path.parse().unwrap()).is_none(), "{path}");
        }
        let url = config
            .target(&"/v1/responses/r_1?after=4&stream=true".parse().unwrap())
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.openai.com/v1/responses/r_1?after=4&stream=true"
        );
        assert!(Config::new("https://secret@example.com").is_err());
        assert!(Config::new("https://api.openai.com/v1").is_err());
    }

    #[test]
    fn keeps_api_headers_and_removes_transport_credentials() {
        let mut h = HeaderMap::new();
        for (name, value) in [
            ("authorization", "Bearer test"),
            ("openai-project", "proj_test"),
            ("openai-beta", "test=v1"),
            ("connection", "x-private"),
            ("x-private", "secret"),
            ("cookie", "private"),
            ("proxy-authorization", "private"),
            ("cf-connecting-ip", "1.2.3.4"),
        ] {
            h.insert(
                http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        let result = request_headers(&h);
        assert_eq!(result["authorization"], "Bearer test");
        assert_eq!(result["openai-project"], "proj_test");
        assert_eq!(result["openai-beta"], "test=v1");
        assert!(!result.contains_key("connection"));
        for name in [
            "x-private",
            "cookie",
            "proxy-authorization",
            "cf-connecting-ip",
        ] {
            assert!(!result.contains_key(name));
        }
    }

    fn decide(method: Method, path: &str, pairs: &[(&'static str, &'static str)]) -> Decision {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.append(*name, HeaderValue::from_static(value));
        }
        prepare(
            &Config::new(DEFAULT_UPSTREAM).unwrap(),
            &method,
            &path.parse().unwrap(),
            &h,
        )
    }

    fn status(decision: Decision) -> Option<StatusCode> {
        match decision {
            Decision::Local(r) => Some(r.status),
            Decision::Forward { .. } => None,
        }
    }

    #[test]
    fn local_decisions() {
        let auth = ("authorization", "Bearer sk-test");
        assert_eq!(
            status(decide(Method::GET, "/health", &[])),
            Some(StatusCode::OK)
        );
        assert_eq!(
            status(decide(Method::HEAD, "/health", &[])),
            Some(StatusCode::OK)
        );
        assert_eq!(
            status(decide(Method::GET, "/", &[auth])),
            Some(StatusCode::NOT_FOUND)
        );
        assert_eq!(
            status(decide(Method::TRACE, "/v1/models", &[auth])),
            Some(StatusCode::METHOD_NOT_ALLOWED)
        );
        for bad in ["Bearer", "Bearer ", "Basic abc", "Bearer a b"] {
            assert_eq!(
                status(decide(Method::GET, "/v1/models", &[("authorization", bad)])),
                Some(StatusCode::UNAUTHORIZED),
                "{bad}"
            );
        }
        assert_eq!(
            status(decide(
                Method::GET,
                "/v1/realtime",
                &[auth, ("upgrade", "websocket")]
            )),
            Some(StatusCode::NOT_IMPLEMENTED)
        );
        // curl --http2 over cleartext offers h2c; that must not break plain HTTP.
        assert_eq!(
            status(decide(
                Method::GET,
                "/v1/models",
                &[auth, ("upgrade", "h2c")]
            )),
            None
        );
    }
}
