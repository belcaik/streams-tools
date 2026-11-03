use std::{collections::BTreeSet, error::Error, fmt};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{TwitchEvent, auth::Auth};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct User {
    pub id: String,
    pub login: String,
    #[serde(rename = "display_name")]
    pub name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Helix {
    client_id: String,
    base: String,
    http: reqwest::Client,
    auth: Auth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelixError(pub(crate) String);

impl fmt::Display for HelixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HelixError {}

#[derive(Deserialize)]
struct UsersResponse {
    data: Vec<User>,
}

impl Helix {
    pub(crate) fn new(client_id: String, base: String, http: reqwest::Client, auth: Auth) -> Self {
        Self {
            client_id,
            base: base.trim_end_matches('/').into(),
            http,
            auth,
        }
    }

    pub(crate) async fn resolve_broadcaster(&self, login: &str) -> Result<User, HelixError> {
        let channel = self.user(Some(login)).await?;
        let account = self.user(None).await?;
        if channel.id != account.id {
            return Err(HelixError(
                "la cuenta autorizada no corresponde al canal configurado".into(),
            ));
        }
        Ok(channel)
    }

    async fn user(&self, login: Option<&str>) -> Result<User, HelixError> {
        let query = login.map(|login| vec![("login", login)]);
        let response = self
            .send(reqwest::Method::GET, "/helix/users", query.as_deref(), None)
            .await?;
        let response = success(response).await?;
        let mut users = response
            .json::<UsersResponse>()
            .await
            .map_err(|error| HelixError(format!("respuesta Helix inválida: {error}")))?
            .data;
        if users.len() != 1 {
            return Err(HelixError(format!(
                "Helix debe devolver exactamente un usuario, recibió {}",
                users.len()
            )));
        }
        Ok(users.remove(0))
    }

    pub(crate) async fn create_subscriptions(
        &self,
        broadcaster_id: &str,
        session_id: &str,
        events: &BTreeSet<TwitchEvent>,
    ) -> Result<(), HelixError> {
        for event in events {
            let specs: &[(&str, &str, Value)] = match event {
                TwitchEvent::Follow => &[(
                    "channel.follow",
                    "2",
                    json!({
                        "broadcaster_user_id": broadcaster_id,
                        "moderator_user_id": broadcaster_id,
                    }),
                )],
                TwitchEvent::Sub => &[
                    (
                        "channel.subscribe",
                        "1",
                        json!({"broadcaster_user_id": broadcaster_id}),
                    ),
                    (
                        "channel.subscription.message",
                        "1",
                        json!({"broadcaster_user_id": broadcaster_id}),
                    ),
                ],
                TwitchEvent::GiftSub => &[(
                    "channel.subscription.gift",
                    "1",
                    json!({"broadcaster_user_id": broadcaster_id}),
                )],
                TwitchEvent::Raid => &[(
                    "channel.raid",
                    "1",
                    json!({"to_broadcaster_user_id": broadcaster_id}),
                )],
                TwitchEvent::Redemption => &[(
                    "channel.channel_points_custom_reward_redemption.add",
                    "1",
                    json!({"broadcaster_user_id": broadcaster_id}),
                )],
                TwitchEvent::Chat => &[(
                    "channel.chat.message",
                    "1",
                    json!({
                        "broadcaster_user_id": broadcaster_id,
                        "user_id": broadcaster_id,
                    }),
                )],
                TwitchEvent::Cheer => &[(
                    "channel.cheer",
                    "1",
                    json!({"broadcaster_user_id": broadcaster_id}),
                )],
            };
            for (kind, version, condition) in specs {
                let body = json!({
                    "type": kind,
                    "version": version,
                    "condition": condition,
                    "transport": {"method": "websocket", "session_id": session_id},
                });
                let response = self
                    .send(
                        reqwest::Method::POST,
                        "/helix/eventsub/subscriptions",
                        None,
                        Some(&body),
                    )
                    .await?;
                success(response).await?;
            }
        }
        Ok(())
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: Option<&[(&str, &str)]>,
        body: Option<&Value>,
    ) -> Result<reqwest::Response, HelixError> {
        for attempt in 0..2 {
            let token = self
                .auth
                .access_token(attempt == 1)
                .await
                .map_err(|error| HelixError(error.to_string()))?;
            let mut request = self
                .http
                .request(method.clone(), format!("{}{path}", self.base))
                .header("Client-Id", &self.client_id)
                .bearer_auth(token);
            if let Some(query) = query {
                request = request.query(query);
            }
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = request
                .send()
                .await
                .map_err(|error| HelixError(format!("solicitud Helix {path}: {error}")))?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                continue;
            }
            return Ok(response);
        }
        unreachable!("el bucle de autenticación siempre devuelve una respuesta")
    }
}

async fn success(response: reqwest::Response) -> Result<reqwest::Response, HelixError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|error| format!("no se pudo leer el cuerpo: {error}"));
    Err(HelixError(format!("error Helix {status}: {body}")))
}
