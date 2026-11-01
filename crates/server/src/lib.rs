use axum::{
    Router,
    body::{Body, HttpBody},
    extract::State,
};
use http::{Request, Response, StatusCode};
use rill_core::{Config, Decision, LocalResponse, local, prepare, response_headers_in_place};
use std::time::Duration;

#[derive(Clone)]
struct Proxy {
    config: Config,
    client: reqwest::Client,
}

pub fn router(config: Config) -> Result<Router, reqwest::Error> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(15))
        // A slow model or long SSE stream has no arbitrary overall timeout.
        .build()?;
    Ok(Router::new()
        .fallback(proxy)
        .with_state(Proxy { config, client }))
}

fn immediate(response: LocalResponse) -> Response<Body> {
    let mut result = Response::new(match response.body {
        std::borrow::Cow::Borrowed(body) => Body::from(body),
        std::borrow::Cow::Owned(body) => Body::from(body),
    });
    *result.status_mut() = response.status;
    *result.headers_mut() = response.headers;
    result
}

async fn proxy(State(state): State<Proxy>, request: Request<Body>) -> Response<Body> {
    let (target, headers) = match prepare(
        &state.config,
        request.method(),
        request.uri(),
        request.headers(),
    ) {
        Decision::Local(response) => return immediate(response),
        Decision::Forward { target, headers } => (target, headers),
    };
    let (parts, body) = request.into_parts();
    // A known-empty body (no Content-Length / Transfer-Encoding) is not re-sent as
    // an empty chunked stream; anything else streams through unchanged.
    let has_body = parts.method != http::Method::GET
        && parts.method != http::Method::HEAD
        && body.size_hint().exact() != Some(0);
    let mut request = state.client.request(parts.method, target).headers(headers);
    if has_body {
        request = request.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            // Never log the URL query, headers or bodies; the error kind is enough.
            eprintln!("rill: upstream request failed: {}", error.without_url());
            return immediate(local(
                StatusCode::BAD_GATEWAY,
                "{\"error\":{\"message\":\"Unable to reach upstream.\",\"type\":\"upstream_error\"}}",
            ));
        }
    };
    let status = upstream.status();
    let mut headers = upstream.headers().clone();
    response_headers_in_place(&mut headers);
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
