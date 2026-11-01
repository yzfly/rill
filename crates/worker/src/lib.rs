use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use rill_core::{
    Config, DEFAULT_UPSTREAM, Decision, LocalResponse, local, prepare, response_headers_in_place,
};
use worker::{
    CacheMode, Context, Env, Fetch, Headers, Request, RequestInit, RequestRedirect, Response,
    Result, event,
};

fn http_headers(headers: &Headers) -> Result<HeaderMap> {
    let mut result = HeaderMap::new();
    for (name, value) in headers.entries() {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| worker::Error::RustError("Invalid header name".into()))?;
        let value = HeaderValue::from_str(&value)
            .map_err(|_| worker::Error::RustError("Invalid header value".into()))?;
        result.append(name, value);
    }
    Ok(result)
}

fn worker_headers(headers: &HeaderMap) -> Result<Headers> {
    let result = Headers::new();
    for (name, value) in headers {
        result.append(
            name.as_str(),
            // Values come from JS strings or ASCII literals, so UTF-8 is lossless here;
            // HeaderValue::to_str would reject legitimate non-ASCII values.
            std::str::from_utf8(value.as_bytes())
                .map_err(|_| worker::Error::RustError("Invalid header value".into()))?,
        )?;
    }
    Ok(result)
}

fn immediate(response: LocalResponse) -> Result<Response> {
    Ok(Response::ok(response.body)?
        .with_status(response.status.as_u16())
        .with_headers(worker_headers(&response.headers)?))
}

#[event(fetch)]
pub async fn fetch(request: Request, env: Env, _ctx: Context) -> Result<Response> {
    let origin = env
        .var("RILL_UPSTREAM")
        .map(|v| v.to_string())
        .unwrap_or_else(|_| DEFAULT_UPSTREAM.into());
    let config = match Config::new(&origin) {
        Ok(config) => config,
        Err(_) => {
            return immediate(local(
                StatusCode::INTERNAL_SERVER_ERROR,
                "{\"error\":{\"message\":\"Invalid upstream configuration.\",\"type\":\"configuration_error\"}}",
            ));
        }
    };
    let method = request
        .method()
        .to_string()
        .parse()
        .map_err(|_| worker::Error::RustError("Invalid method".into()))?;
    let uri = request
        .url()?
        .as_str()
        .parse()
        .map_err(|_| worker::Error::RustError("Invalid URL".into()))?;
    let headers = http_headers(request.headers())?;
    let (target, headers) = match prepare(&config, &method, &uri, &headers) {
        Decision::Local(response) => return immediate(response),
        Decision::Forward { target, headers } => (target, headers),
    };
    let mut init = RequestInit::new();
    init.with_method(request.method())
        .with_headers(worker_headers(&headers)?)
        .with_redirect(RequestRedirect::Manual)
        .with_cache(CacheMode::NoStore)
        // Keep the JS ReadableStream: no body buffering or per-token WASM crossings.
        .with_body(request.inner().body().map(Into::into));
    let outgoing = Request::new_with_init(target.as_str(), &init)?;
    let signal = request.inner().signal().into();
    match Fetch::Request(outgoing).send_with_signal(&signal).await {
        Ok(response) => {
            let mut headers = http_headers(response.headers())?;
            response_headers_in_place(&mut headers);
            Ok(response.with_headers(worker_headers(&headers)?))
        }
        Err(_) => immediate(local(
            StatusCode::BAD_GATEWAY,
            "{\"error\":{\"message\":\"Unable to reach upstream.\",\"type\":\"upstream_error\"}}",
        )),
    }
}
