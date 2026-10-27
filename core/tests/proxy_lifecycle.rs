use axum::Router;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use bytes::Bytes;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use yi_core::proxy::ProxyHandle;
use yi_core::{CoreError, UpstreamError};

fn handle() -> ProxyHandle {
    ProxyHandle::new(|| Router::new().route("/ping", get(|| async { "pong" })))
}

fn free_addr() -> SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap()
}

async fn ping(addr: SocketAddr) -> Option<String> {
    let mut s = tokio::net::TcpStream::connect(addr).await.ok()?;
    s.write_all(b"GET /ping HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).await.ok()?;
    Some(out)
}

#[tokio::test]
async fn start_stop_restart() {
    let h = handle();
    let a = free_addr();
    assert!(!h.status().running);
    h.start(a).await.unwrap();
    let st = h.status();
    assert!(st.running);
    assert_eq!(st.addr, Some(a));
    assert!(st.started_at.is_some());
    assert!(ping(a).await.unwrap().contains("pong"));

    h.stop().await;
    assert!(!h.status().running);
    assert!(ping(a).await.is_none());

    h.start(a).await.unwrap();
    assert!(ping(a).await.unwrap().contains("pong"));

    let b = free_addr();
    h.restart(b).await.unwrap();
    assert_eq!(h.status().addr, Some(b));
    assert!(ping(b).await.unwrap().contains("pong"));
    assert!(ping(a).await.is_none());
    h.stop().await;
}

#[tokio::test]
async fn start_on_busy_port_returns_io_error() {
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = blocker.local_addr().unwrap();
    let h = handle();
    let err = h.start(addr).await.unwrap_err();
    match &err {
        CoreError::Io(m) => {
            assert!(m.contains(&addr.port().to_string()), "{m}");
            assert!(m.contains("已被占用或无法监听"), "{m}");
        }
        other => panic!("expected Io, got {other:?}"),
    }
    let st = h.status();
    assert!(!st.running);
    assert!(st.last_error.is_some());
    drop(blocker);
    h.start(addr).await.unwrap();
    assert!(h.status().running);
    assert!(h.status().last_error.is_none());
    h.stop().await;
}

async fn respond(e: CoreError) -> (StatusCode, Option<String>, Bytes) {
    let app = Router::new().route("/", get(move || async move { e.clone().into_response() }));
    let resp = app
        .oneshot(Request::get("/").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string());
    (
        status,
        ct,
        to_bytes(resp.into_body(), usize::MAX).await.unwrap(),
    )
}

#[tokio::test]
async fn validation_error_maps_to_400() {
    let (s, _, b) = respond(CoreError::Validation("bad".into())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["error"]["type"], "validation");
    assert_eq!(v["error"]["message"], "bad");
}

#[tokio::test]
async fn conflict_maps_to_409() {
    let (s, _, b) = respond(CoreError::Conflict("dup".into())).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["error"]["type"], "conflict");
}

#[tokio::test]
async fn not_found_and_others_map() {
    assert_eq!(
        respond(CoreError::NotFound("x".into())).await.0,
        StatusCode::NOT_FOUND
    );
    for e in [
        CoreError::Database("x".into()),
        CoreError::Io("x".into()),
        CoreError::Internal("x".into()),
    ] {
        assert_eq!(respond(e).await.0, StatusCode::INTERNAL_SERVER_ERROR);
    }
}

#[tokio::test]
async fn upstream_error_passthrough_status_and_body() {
    let e = CoreError::Upstream(UpstreamError {
        status: 429,
        body: Bytes::from_static(b"{\"error\":{\"message\":\"slow\"}}"),
        content_type: Some("application/json".into()),
    });
    let (s, ct, b) = respond(e).await;
    assert_eq!(s.as_u16(), 429);
    assert_eq!(ct.as_deref(), Some("application/json"));
    assert_eq!(&b[..], b"{\"error\":{\"message\":\"slow\"}}");
}
