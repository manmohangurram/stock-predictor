//! The proxy is chosen by the settings row and by nothing else.

use std::time::Duration;

use stock_predicter::http;
use stock_predicter::rules::runtime::Runtime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve_once(listener: TcpListener, expect: &'static str) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(req.contains(expect), "unexpected request: {req}");
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await;
        req
    })
}

/// The environment names a proxy that answers; the request still does not reach it. The target
/// is an unroutable address, so a request that *did* use the proxy would return 200.
#[tokio::test]
async fn the_environment_never_chooses_the_proxy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    std::env::set_var("HTTP_PROXY", format!("http://127.0.0.1:{port}"));
    std::env::set_var("HTTPS_PROXY", format!("http://127.0.0.1:{port}"));
    std::env::set_var("ALL_PROXY", format!("http://127.0.0.1:{port}"));

    let client = http::client(&Runtime::default(), 2).unwrap();
    let result = client.get("http://10.255.255.1/unreachable").send().await;

    assert!(result.is_err(), "an unroutable address must not succeed");
    assert!(
        tokio::time::timeout(Duration::from_millis(500), listener.accept())
            .await
            .is_err(),
        "the environment's proxy was used — .no_proxy() is missing"
    );

    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("HTTPS_PROXY");
    std::env::remove_var("ALL_PROXY");
}

/// With a proxy in settings, an absolute-form request lands on the proxy.
#[tokio::test]
async fn a_configured_proxy_is_used() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = serve_once(listener, "example.invalid").await;

    let rt = Runtime {
        proxy: Some(format!("http://127.0.0.1:{port}")),
        ..Runtime::default()
    };
    let client = http::client(&rt, 5).unwrap();
    let resp = client
        .get("http://example.invalid/anything")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("the proxy must answer");

    assert_eq!(resp.status(), 200);
    let seen = server.await.unwrap();
    assert!(seen.starts_with("GET http://example.invalid"), "{seen}");
}

/// One builder, in one file. A fourth construction site would silently skip `.no_proxy()`.
#[test]
fn only_one_client_builder_exists() {
    let mut offenders = Vec::new();
    for entry in std::fs::read_dir("src").unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        if text.contains("Client::builder") && path.file_name().unwrap() != "http.rs" {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "build clients through crate::http, not directly: {offenders:?}"
    );
}
