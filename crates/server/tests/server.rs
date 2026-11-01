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

async fn start(bus: Bus) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have an address");
    let task = tokio::spawn(async move {
        axum::serve(listener, streams_tools_server::router(bus))
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
    let server = start(Bus::new(8).expect("capacity should be valid")).await;
    let response = request(server.addr, "GET", "/health", "").await;

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert_eq!(response_body(&response), "ok");
}

#[tokio::test]
async fn valid_post_reaches_bus_subscriber() {
    let bus = Bus::new(8).expect("capacity should be valid");
    let mut subscriber = bus.subscribe();
    let server = start(bus).await;
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
    let server = start(Bus::new(8).expect("capacity should be valid")).await;
    let response = request(server.addr, "POST", "/api/events", "{not-json").await;

    assert!(response.starts_with("HTTP/1.1 400 Bad Request\r\n"));
    assert!(response_body(&response).contains("invalid event JSON:"));
}

#[tokio::test]
async fn valid_post_without_subscribers_is_accepted() {
    let server = start(Bus::new(8).expect("capacity should be valid")).await;
    let event = event("follow-1", EventData::Follow(FollowData {}));
    let body = serde_json::to_string(&event).expect("event should serialize");
    let response = request(server.addr, "POST", "/api/events", &body).await;

    assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"));
}

#[tokio::test]
async fn websocket_receives_published_event() {
    let bus = Bus::new(8).expect("capacity should be valid");
    let server = start(bus.clone()).await;
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
    let server = start(bus.clone()).await;
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
    let server = start(Bus::new(8).expect("capacity should be valid")).await;
    let response = request(server.addr, "GET", "/streams.js", "").await;

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("\r\ncontent-type: text/javascript\r\n"));
    assert!(response_body(&response).contains("window.streams = { on, emit };"));
}
