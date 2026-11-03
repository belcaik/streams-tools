use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use streams_tools_adapters::twitch::{TwitchSettings, TwitchUrls};
use tokio::{net::TcpListener, task::JoinHandle};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Server(JoinHandle<()>);

impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct Context {
    pub settings: TwitchSettings,
    pub posts: Arc<Mutex<Vec<Value>>>,
    _dir: TempDir,
    _server: Server,
}

pub async fn context(keepalive_margin: Duration) -> Context {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let posts = Arc::new(Mutex::new(Vec::new()));
    let captured = posts.clone();
    let router = Router::new()
        .route(
            "/helix/users",
            get(|| async {
                Json(json!({"data": [{"id": "42", "login": "canal", "display_name": "Canal"}]}))
            }),
        )
        .route(
            "/helix/eventsub/subscriptions",
            post(move |Json(body): Json<Value>| {
                let captured = captured.clone();
                async move {
                    captured.lock().unwrap().push(body);
                    StatusCode::ACCEPTED
                }
            }),
        );
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let path = std::env::temp_dir().join(format!(
        "streams-tools-eventsub-{}-{}",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    let dir = TempDir(path);
    let data_dir = dir.0.join("data").join("twitch");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(
        data_dir.join("token.json"),
        json!({
            "access_token": "token", "refresh_token": "refresh",
            "expires_at": u64::MAX, "scopes": []
        })
        .to_string(),
    )
    .unwrap();
    let mut settings = TwitchSettings::new("client-id".into(), "canal".into(), data_dir);
    settings.urls = TwitchUrls {
        websocket: String::new(),
        helix: base.clone(),
        identity: base,
    };
    settings.keepalive_margin = keepalive_margin;
    Context {
        settings,
        posts,
        _dir: dir,
        _server: server,
    }
}

pub async fn ws_listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    (listener, url)
}

pub fn message(id: &str, kind: &str, payload: Value) -> String {
    json!({
        "metadata": {
            "message_id": id,
            "message_type": kind,
            "message_timestamp": "2026-01-02T03:04:05.678Z",
            "subscription_type": null,
            "subscription_version": "1"
        },
        "payload": payload
    })
    .to_string()
}

pub fn welcome(session: &str, keepalive: u64) -> String {
    message(
        &format!("welcome-{session}"),
        "session_welcome",
        json!({"session": {
            "id": session,
            "status": "connected",
            "keepalive_timeout_seconds": keepalive,
            "reconnect_url": null
        }}),
    )
}
