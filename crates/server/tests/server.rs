use std::{net::SocketAddr, time::Duration};

use futures_util::StreamExt;
use streams_tools_core::{Bus, ChatData, Event, EventData, FollowData, Platform, User};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

struct TestServer {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start(bus: Bus, overlays: Vec<streams_tools_server::PluginOverlay>) -> TestServer {
    start_with_alerts(bus, overlays, streams_tools_server::AlertAssets::default()).await
}

async fn start_with_alerts(
    bus: Bus,
    overlays: Vec<streams_tools_server::PluginOverlay>,
    alerts: streams_tools_server::AlertAssets,
) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have an address");
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            streams_tools_server::router(bus, overlays, alerts),
        )
        .await
        .expect("test server should run");
    });
    TestServer { addr, task }
}

async fn request(addr: SocketAddr, method: &str, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(addr)
        .await
        .expect("test server should accept connections");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("request should be written");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("response should be read");
    String::from_utf8(response).expect("HTTP response should be UTF-8")
}

fn response_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .expect("response should contain headers")
        .1
}

fn event(id: &str, event: EventData) -> Event {
    Event {
        id: id.into(),
        platform: Platform::Twitch,
        event,
        user: User {
            id: "1".into(),
            name: "ana".into(),
            roles: vec!["mod".into()],
        },
        ts: "2026-10-31T20:00:00Z".into(),
    }
}

async fn next_event(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> Event {
    let message = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("websocket event should arrive before timeout")
        .expect("websocket should remain open")
        .expect("websocket frame should be valid");
    match message {
        Message::Text(text) => {
            serde_json::from_str(text.as_str()).expect("text frame should contain an Event")
        }
        other => panic!("expected text frame, got {other:?}"),
    }
}

#[tokio::test]
async fn health_returns_ok() {
    let server = start(Bus::new(8).expect("capacity should be valid"), Vec::new()).await;
    let response = request(server.addr, "GET", "/health", "").await;

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert_eq!(response_body(&response), "ok");
}

#[tokio::test]
async fn valid_post_reaches_bus_subscriber() {
    let bus = Bus::new(8).expect("capacity should be valid");
    let mut subscriber = bus.subscribe();
    let server = start(bus, Vec::new()).await;
    let expected = event(
        "chat-1",
        EventData::Chat(ChatData {
            message: "hola".into(),
        }),
    );

    let response = request(
        server.addr,
        "POST",
        "/api/events",
        &serde_json::to_string(&expected).expect("event should serialize"),
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"));
    let actual = timeout(Duration::from_secs(2), subscriber.recv())
        .await
        .expect("event should arrive before timeout")
        .expect("event should be published");
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn invalid_post_returns_clear_bad_request() {
    let server = start(Bus::new(8).expect("capacity should be valid"), Vec::new()).await;
    let response = request(server.addr, "POST", "/api/events", "{not-json").await;

    assert!(response.starts_with("HTTP/1.1 400 Bad Request\r\n"));
    assert!(response_body(&response).contains("invalid event JSON:"));
}

#[tokio::test]
async fn valid_post_without_subscribers_is_accepted() {
    let server = start(Bus::new(8).expect("capacity should be valid"), Vec::new()).await;
    let event = event("follow-1", EventData::Follow(FollowData {}));
    let body = serde_json::to_string(&event).expect("event should serialize");
    let response = request(server.addr, "POST", "/api/events", &body).await;

    assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"));
}

#[tokio::test]
async fn websocket_receives_published_event() {
    let bus = Bus::new(8).expect("capacity should be valid");
    let server = start(bus.clone(), Vec::new()).await;
    let (mut socket, _) = connect_async(format!("ws://{}/ws", server.addr))
        .await
        .expect("websocket should connect");
    let expected = event(
        "chat-1",
        EventData::Chat(ChatData {
            message: "hola".into(),
        }),
    );

    assert!(
        bus.publish(expected.clone())
            .expect("websocket subscriber should exist")
            >= 1
    );
    assert_eq!(next_event(&mut socket).await, expected);
}

#[tokio::test]
async fn websocket_filter_discards_unrequested_types() {
    let bus = Bus::new(8).expect("capacity should be valid");
    let server = start(bus.clone(), Vec::new()).await;
    let (mut socket, _) = connect_async(format!("ws://{}/ws?types=chat", server.addr))
        .await
        .expect("websocket should connect");
    let follow = event("follow-1", EventData::Follow(FollowData {}));
    let chat = event(
        "chat-1",
        EventData::Chat(ChatData {
            message: "hola".into(),
        }),
    );

    assert!(
        bus.publish(follow)
            .expect("websocket subscriber should exist")
            >= 1
    );
    assert!(
        bus.publish(chat.clone())
            .expect("websocket subscriber should exist")
            >= 1
    );
    assert_eq!(next_event(&mut socket).await, chat);
}

#[tokio::test]
async fn streams_js_has_javascript_content_type() {
    let server = start(Bus::new(8).expect("capacity should be valid"), Vec::new()).await;
    let response = request(server.addr, "GET", "/streams.js", "").await;

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("\r\ncontent-type: text/javascript\r\n"));
    assert!(response_body(&response).contains("window.streams = { on, emit };"));
}

static NEXT_OVERLAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct OverlayDir(std::path::PathBuf);

impl OverlayDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-overlay-{}-{}",
            std::process::id(),
            NEXT_OVERLAY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("overlay dir should be created");
        Self(path)
    }
}

impl Drop for OverlayDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn overlay_response(path: &str, css: Option<&str>) -> String {
    let temp = OverlayDir::new();
    std::fs::write(temp.0.join("index.html"), "plugin index").expect("index should be written");
    std::fs::write(temp.0.join("app.js"), "console.log('ok')").expect("asset should be written");
    std::fs::write(temp.0.join("config.json"), "{}").expect("shadow config should be written");
    std::fs::write(temp.0.join("custom.css"), "fallback").expect("shadow CSS should be written");
    if let Some(css) = css {
        std::fs::write(temp.0.join("user.css"), css).expect("custom CSS should be written");
    }
    let overlay = streams_tools_server::PluginOverlay {
        name: "example".into(),
        root: temp.0.clone(),
        config: serde_json::json!({"title": "configured"}),
        custom_css: temp.0.join("user.css"),
    };
    let server = start(
        Bus::new(8).expect("capacity should be valid"),
        vec![overlay],
    )
    .await;
    request(server.addr, "GET", path, "").await
}

#[tokio::test]
async fn serves_overlay_index_and_secondary_assets() {
    let index = overlay_response("/p/example/", None).await;
    assert!(index.starts_with("HTTP/1.1 200 OK\r\n"), "{index}");
    assert_eq!(response_body(&index), "plugin index");
    let asset = overlay_response("/p/example/app.js", None).await;
    assert_eq!(response_body(&asset), "console.log('ok')");
}

#[tokio::test]
async fn config_route_returns_descriptor_json() {
    let response = overlay_response("/p/example/config.json", None).await;
    assert!(response.contains("\r\ncontent-type: application/json\r\n"));
    assert_eq!(response_body(&response), r#"{"title":"configured"}"#);
}

#[tokio::test]
async fn overlay_rejects_parent_traversal() {
    let literal = overlay_response("/p/example/../../Cargo.toml", None).await;
    assert!(literal.starts_with("HTTP/1.1 404 Not Found\r\n"));
    assert!(!response_body(&literal).contains("[workspace]"));
    let encoded = overlay_response("/p/example/%2e%2e/%2e%2e/Cargo.toml", None).await;
    assert!(
        encoded.starts_with("HTTP/1.1 400 Bad Request\r\n")
            || encoded.starts_with("HTTP/1.1 404 Not Found\r\n")
    );
}

#[tokio::test]
async fn unknown_overlay_returns_not_found() {
    let response = overlay_response("/p/other/", None).await;
    assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"));
}

#[tokio::test]
async fn custom_css_is_fresh_and_never_not_found() {
    let existing =
        overlay_response("/p/example/custom.css", Some(".st-echo { color: red; }")).await;
    assert!(existing.contains("\r\ncontent-type: text/css; charset=utf-8\r\n"));
    assert_eq!(response_body(&existing), ".st-echo { color: red; }");
    let missing = overlay_response("/p/example/custom.css", None).await;
    assert!(missing.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(missing.contains("\r\ncontent-type: text/css; charset=utf-8\r\n"));
    assert_eq!(response_body(&missing), "");
}

static NEXT_ALERT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct AlertDir(std::path::PathBuf);

impl Drop for AlertDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn alert_server() -> (TestServer, AlertDir) {
    let path = std::env::temp_dir().join(format!(
        "streams-alert-server-{}-{}",
        std::process::id(),
        NEXT_ALERT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let pack = path.join("neon");
    std::fs::create_dir_all(&pack).expect("alert pack should be created");
    std::fs::write(pack.join("style.css"), "alert css").expect("CSS should be written");
    std::fs::write(pack.join("pixel.png"), "png").expect("PNG should be written");
    let alerts = streams_tools_server::AlertAssets {
        index: serde_json::json!({
            "packs": [{"name": "neon", "alerts": []}],
            "errors": []
        }),
        packs: vec![streams_tools_server::AlertPackAssets {
            name: "neon".into(),
            root: std::fs::canonicalize(pack).expect("pack root should canonicalize"),
        }],
    };
    let server = start_with_alerts(Bus::new(8).expect("capacity"), Vec::new(), alerts).await;
    (server, AlertDir(path))
}

#[tokio::test]
async fn alert_index_returns_the_exact_catalog() {
    let (server, _temp) = alert_server().await;
    let response = request(server.addr, "GET", "/alerts/index.json", "").await;
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert_eq!(
        response_body(&response),
        r#"{"errors":[],"packs":[{"alerts":[],"name":"neon"}]}"#
    );
}

#[tokio::test]
async fn serves_alert_css_and_png_with_content_types() {
    let (server, _temp) = alert_server().await;
    let css = request(server.addr, "GET", "/alerts/neon/style.css", "").await;
    assert!(css.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(css.contains("\r\ncontent-type: text/css; charset=utf-8\r\n"));
    assert_eq!(response_body(&css), "alert css");
    let png = request(server.addr, "GET", "/alerts/neon/pixel.png", "").await;
    assert!(png.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(png.contains("\r\ncontent-type: image/png\r\n"));
    assert_eq!(response_body(&png), "png");
}

#[tokio::test]
async fn unknown_alert_pack_returns_not_found() {
    let (server, _temp) = alert_server().await;
    let response = request(server.addr, "GET", "/alerts/missing/style.css", "").await;
    assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"));
    assert_eq!(response_body(&response), "");
}

#[tokio::test]
async fn alert_assets_reject_traversal_and_escaping_symlinks() {
    let (server, _temp) = alert_server().await;
    for path in [
        "/alerts/neon/../../Cargo.toml",
        "/alerts/neon/%2e%2e/%2e%2e/Cargo.toml",
    ] {
        let response = request(server.addr, "GET", path, "").await;
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request\r\n")
                || response.starts_with("HTTP/1.1 404 Not Found\r\n"),
            "{path}: {response}"
        );
        assert!(!response_body(&response).contains("[workspace]"));
    }
    #[cfg(unix)]
    {
        let outside = _temp.0.join("outside.txt");
        std::fs::write(&outside, "secret").expect("outside file should be written");
        std::os::unix::fs::symlink(&outside, _temp.0.join("neon/escape.txt"))
            .expect("escaping symlink should be created");
        let response = request(server.addr, "GET", "/alerts/neon/escape.txt", "").await;
        assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(!response_body(&response).contains("secret"));
    }
}
