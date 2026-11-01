use std::{collections::HashSet, future::Future, net::SocketAddr};

use axum::{
    Json, Router,
    extract::{
        Query, State,
        rejection::JsonRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use streams_tools_core::{Bus, Event, EventData, ReceiveError, Subscriber};

#[derive(Debug, Deserialize)]
struct WsQuery {
    types: Option<String>,
}

pub fn router(bus: Bus) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(websocket))
        .route("/api/events", post(publish))
        .route("/streams.js", get(streams_js))
        .with_state(bus)
}

pub async fn serve(
    bus: Bus,
    addr: SocketAddr,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(bus))
        .with_graceful_shutdown(shutdown)
        .await
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
