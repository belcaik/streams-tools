use std::{collections::HashSet, future::Future, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    extract::{
        Path as RequestPath, Query, State,
        rejection::JsonRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use streams_tools_core::{Bus, Event, EventData, ReceiveError, Subscriber};
use tower_http::services::ServeDir;

#[derive(Debug, Deserialize)]
struct WsQuery {
    types: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PluginOverlay {
    pub name: String,
    pub root: PathBuf,
    pub config: serde_json::Value,
    pub custom_css: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AlertPackAssets {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct AlertAssets {
    pub index: serde_json::Value,
    pub packs: Vec<AlertPackAssets>,
}

impl Default for AlertAssets {
    fn default() -> Self {
        Self {
            index: serde_json::json!({"packs": [], "errors": []}),
            packs: Vec::new(),
        }
    }
}

pub fn router(bus: Bus, overlays: Vec<PluginOverlay>, alerts: AlertAssets) -> Router {
    let alerts_router = Router::new()
        .route("/index.json", get(alert_index))
        .route("/{pack}/{*path}", get(alert_asset))
        .with_state(Arc::new(alerts));
    let mut router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(websocket))
        .route("/api/events", post(publish))
        .route("/streams.js", get(streams_js))
        .with_state(bus)
        .nest("/alerts", alerts_router);
    for overlay in overlays {
        let config = overlay.config;
        let custom_css_path = overlay.custom_css;
        let overlay_router = Router::new()
            .route(
                "/config.json",
                get(move || {
                    let config = config.clone();
                    async move { Json(config) }
                }),
            )
            .route(
                "/custom.css",
                get(move || {
                    let path = custom_css_path.clone();
                    async move { custom_css(path).await }
                }),
            )
            .fallback_service(ServeDir::new(overlay.root).append_index_html_on_directories(true));
        router = router.nest(&format!("/p/{}/", overlay.name), overlay_router);
    }
    router
}

pub async fn serve(
    bus: Bus,
    addr: SocketAddr,
    overlays: Vec<PluginOverlay>,
    alerts: AlertAssets,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(bus, overlays, alerts))
        .with_graceful_shutdown(shutdown)
        .await
}

async fn custom_css(path: PathBuf) -> Response {
    let body = tokio::fs::read(path).await.unwrap_or_default();
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], body).into_response()
}

async fn alert_index(State(alerts): State<Arc<AlertAssets>>) -> Json<serde_json::Value> {
    Json(alerts.index.clone())
}

async fn alert_asset(
    State(alerts): State<Arc<AlertAssets>>,
    RequestPath((pack, path)): RequestPath<(String, String)>,
) -> Response {
    let Some(pack) = alerts.packs.iter().find(|candidate| candidate.name == pack) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !valid_asset_path(&path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(root) = tokio::fs::canonicalize(&pack.root).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(canonical) = tokio::fs::canonicalize(root.join(&path)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !canonical.starts_with(&root)
        || !tokio::fs::metadata(&canonical)
            .await
            .is_ok_and(|metadata| metadata.is_file())
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(body) = tokio::fs::read(&canonical).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ([(header::CONTENT_TYPE, alert_content_type(&path))], body).into_response()
}

fn valid_asset_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

fn alert_content_type(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

async fn streams_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript")],
        include_str!("../assets/streams.js"),
    )
}

async fn publish(State(bus): State<Bus>, payload: Result<Json<Event>, JsonRejection>) -> Response {
    match payload {
        Ok(Json(event)) => {
            let _ = bus.publish(event);
            StatusCode::ACCEPTED.into_response()
        }
        Err(rejection) => (
            StatusCode::BAD_REQUEST,
            format!("invalid event JSON: {}", rejection.body_text()),
        )
            .into_response(),
    }
}

async fn websocket(
    State(bus): State<Bus>,
    Query(query): Query<WsQuery>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    let subscriber = bus.subscribe();
    let types = query.types.map(|types| {
        types
            .split(',')
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_owned)
            .collect()
    });
    upgrade.on_upgrade(move |socket| forward(socket, subscriber, types))
}

async fn forward(
    mut socket: WebSocket,
    mut subscriber: Subscriber,
    types: Option<HashSet<String>>,
) {
    loop {
        let text = tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => continue,
            },
            received = subscriber.recv() => match received {
                Ok(event) if accepts(&types, &event) => {
                    let Ok(json) = serde_json::to_string(&event) else {
                        continue;
                    };
                    json
                }
                Ok(_) => continue,
                Err(ReceiveError::Lagged(skipped)) => {
                    serde_json::json!({
                        "type": "system",
                        "data": { "lagged": skipped }
                    })
                    .to_string()
                }
                Err(ReceiveError::Closed) => return,
            },
        };

        if socket.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
}

fn accepts(types: &Option<HashSet<String>>, event: &Event) -> bool {
    types
        .as_ref()
        .is_none_or(|types| types.contains(event_type(event)))
}

fn event_type(event: &Event) -> &str {
    match &event.event {
        EventData::Follow(_) => "follow",
        EventData::Sub(_) => "sub",
        EventData::GiftSub(_) => "gift_sub",
        EventData::Raid(_) => "raid",
        EventData::Donation(_) => "donation",
        EventData::Redemption(_) => "redemption",
        EventData::Chat(_) => "chat",
        EventData::Plugin(data) => &data.name,
    }
}
