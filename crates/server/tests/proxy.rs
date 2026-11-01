use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
};
use futures_util::{StreamExt, stream};
use http::{Request, Response, StatusCode};
use rill_core::Config;
use std::{
    convert::Infallible,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Running {
    origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn(app: Router) -> Running {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Running { origin, task }
}

async fn proxy(upstream: &str) -> Running {
    spawn(rill_server::router(Config::new(upstream).unwrap()).unwrap()).await
}

#[tokio::test]
async fn preserves_payload_query_error_and_api_headers() {
    let upstream = spawn(Router::new().fallback(|request: Request<Body>| async move {
        assert_eq!(
            request.uri().path_and_query().unwrap().as_str(),
            "/v1/responses?test=one%20two"
        );
        assert_eq!(request.headers()["authorization"], "Bearer test-key");
        assert_eq!(request.headers()["openai-project"], "proj_test");
        assert!(!request.headers().contains_key("cookie"));
        assert!(!request.headers().contains_key("x-secret-hop"));
        assert_eq!(
            to_bytes(request.into_body(), 4096).await.unwrap(),
            "{\"input\":\"hello\",\"stream\":false}"
        );
        Response::builder()
            .status(429)
            .header("x-request-id", "req_test")
            .header("retry-after", "2")
            .header("content-type", "application/json")
            .body(Body::from("{\"error\":{\"code\":\"rate_limit_exceeded\"}}"))
            .unwrap()
    }))
    .await;
    let gateway = proxy(&upstream.origin).await;
    let response = reqwest::Client::new()
        .post(format!("{}/v1/responses?test=one%20two", gateway.origin))
        .bearer_auth("test-key")
        .header("openai-project", "proj_test")
        .header("cookie", "private")
        .header("connection", "x-secret-hop")
        .header("x-secret-hop", "private")
        .body("{\"input\":\"hello\",\"stream\":false}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["x-request-id"], "req_test");
    assert_eq!(response.headers()["retry-after"], "2");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.text().await.unwrap(),
        "{\"error\":{\"code\":\"rate_limit_exceeded\"}}"
    );
}

#[tokio::test]
async fn streams_first_event_before_upstream_finishes() {
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let upstream = spawn(Router::new().fallback(move || {
        let wait = wait.lock().unwrap().take().unwrap();
        async move {
            let first = stream::once(async {
                Ok::<_, Infallible>(Bytes::from_static(
                    b"event: response.output_text.delta\ndata: {\"delta\":\"hello\"}\n\n",
                ))
            });
            let last = stream::once(async move {
                wait.await.unwrap();
                Ok::<_, Infallible>(Bytes::from_static(
                    b"event: response.completed\ndata: {}\n\n",
                ))
            });
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(first.chain(last)))
                .unwrap()
        }
    }))
    .await;
    let gateway = proxy(&upstream.origin).await;
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        reqwest::Client::new()
            .post(format!("{}/v1/responses", gateway.origin))
            .bearer_auth("test-key")
            .body("{\"stream\":true}")
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.bytes_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(5), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&chunk).contains("hello"));
    release.send(()).unwrap();
    let mut remainder = Vec::new();
    while let Some(chunk) = body.next().await {
        remainder.extend_from_slice(&chunk.unwrap());
    }
    assert!(String::from_utf8_lossy(&remainder).contains("response.completed"));
}

#[tokio::test]
async fn rejects_missing_keys_and_handles_preflight_without_upstream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let upstream = spawn(Router::new().fallback(move || {
        count.fetch_add(1, Ordering::SeqCst);
        async { "unexpected" }
    }))
    .await;
    let gateway = proxy(&upstream.origin).await;
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{}/v1/models", gateway.origin))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = client
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/v1/responses", gateway.origin),
        )
        .header(
            "access-control-request-headers",
            "authorization, openai-project",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()["access-control-allow-headers"],
        "authorization, openai-project"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn does_not_follow_redirects_with_the_api_key() {
    let upstream = spawn(Router::new().fallback(|| async {
        Response::builder()
            .status(307)
            .header("location", "http://127.0.0.1:1/never-contact")
            .body(Body::from("redirect"))
            .unwrap()
    }))
    .await;
    let gateway = proxy(&upstream.origin).await;
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!("{}/v1/models", gateway.origin))
        .bearer_auth("test-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers()["location"],
        "http://127.0.0.1:1/never-contact"
    );
}

#[tokio::test]
async fn bodyless_delete_is_not_sent_as_chunked() {
    let upstream = spawn(Router::new().fallback(|request: Request<Body>| async move {
        assert_eq!(request.method(), http::Method::DELETE);
        assert!(!request.headers().contains_key("transfer-encoding"));
        "{\"deleted\":true}"
    }))
    .await;
    let gateway = proxy(&upstream.origin).await;
    let response = reqwest::Client::new()
        .delete(format!("{}/v1/files/file_1", gateway.origin))
        .bearer_auth("test-key")
        .header("upgrade", "h2c")
        .header("connection", "Upgrade")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "{\"deleted\":true}");
}
