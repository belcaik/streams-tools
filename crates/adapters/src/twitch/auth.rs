use std::{
    error::Error,
    fmt,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time::Instant};

#[derive(Debug, Clone)]
pub(crate) struct Auth {
    inner: Arc<Inner>,
}

struct Inner {
    client_id: String,
    scopes: Vec<String>,
    token_path: PathBuf,
    identity_base: String,
    http: reqwest::Client,
    prompt: Prompt,
    state: Mutex<State>,
}

impl fmt::Debug for Inner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Inner")
            .field("client_id", &self.client_id)
            .field("scopes", &self.scopes)
            .field("token_path", &self.token_path)
            .field("identity_base", &self.identity_base)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct State {
    loaded: bool,
    token: Option<Token>,
}

#[derive(Serialize, Deserialize)]
struct Token {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthError(pub(crate) String);

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for AuthError {}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

pub(crate) type Prompt = Arc<dyn Fn(&DeviceAuthorization) + Send + Sync>;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
    scope: Vec<String>,
}

impl TokenResponse {
    fn into_token(self, previous_refresh: Option<&str>) -> Result<Token, AuthError> {
        let refresh_token = self
            .refresh_token
            .or_else(|| previous_refresh.map(str::to_owned))
            .ok_or_else(|| AuthError("Twitch no devolvió un refresh token".into()))?;
        Ok(Token {
            access_token: self.access_token,
            refresh_token,
            expires_at: unix_now().saturating_add(self.expires_in),
            scopes: self.scope,
        })
    }
}

impl Auth {
    pub(crate) fn new(
        client_id: String,
        scopes: Vec<String>,
        token_path: PathBuf,
        identity_base: String,
        http: reqwest::Client,
        prompt: Prompt,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                client_id,
                scopes,
                token_path,
                identity_base: identity_base.trim_end_matches('/').into(),
                http,
                prompt,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub(crate) async fn requires_login(&self) -> Result<bool, AuthError> {
        let mut state = self.inner.state.lock().await;
        self.load(&mut state).await?;
        Ok(state
            .token
            .as_ref()
            .is_none_or(|token| token.refresh_token.is_empty()))
    }

    pub(crate) async fn access_token(&self, force_refresh: bool) -> Result<String, AuthError> {
        let mut state = self.inner.state.lock().await;
        self.load(&mut state).await?;
        if let Some(token) = &state.token {
            if !force_refresh && token.expires_at > unix_now().saturating_add(30) {
                return Ok(token.access_token.clone());
            }
            if !token.refresh_token.is_empty() {
                if let Some(response) = self.refresh(&token.refresh_token).await? {
                    let token = response.into_token(None)?;
                    let access = token.access_token.clone();
                    self.persist(&token).await?;
                    state.token = Some(token);
                    return Ok(access);
                }
                state.token = None;
            }
        }
        self.device_flow(&mut state).await
    }

    async fn load(&self, state: &mut State) -> Result<(), AuthError> {
        if state.loaded {
            return Ok(());
        }
        state.token = match tokio::fs::read(&self.inner.token_path).await {
            Ok(bytes) => Some(serde_json::from_slice(&bytes).map_err(|error| {
                AuthError(format!("{}: {error}", self.inner.token_path.display()))
            })?),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AuthError(format!(
                    "{}: {error}",
                    self.inner.token_path.display()
                )));
            }
        };
        state.loaded = true;
        Ok(())
    }

    async fn refresh(&self, refresh_token: &str) -> Result<Option<TokenResponse>, AuthError> {
        let response = self
            .post(
                "/oauth2/token",
                &[
                    ("client_id", &self.inner.client_id),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh_token),
                ],
            )
            .await?;
        if !response.status().is_success() {
            return Ok(None);
        }
        response
            .json()
            .await
            .map(Some)
            .map_err(|error| AuthError(format!("respuesta de renovación inválida: {error}")))
    }

    async fn device_flow(&self, state: &mut State) -> Result<String, AuthError> {
        let scopes = self.inner.scopes.join(" ");
        let response = self
            .post(
                "/oauth2/device",
                &[("client_id", &self.inner.client_id), ("scopes", &scopes)],
            )
            .await?;
        let authorization = parse_success(response).await?;
        (self.inner.prompt)(&authorization);
        let deadline = Instant::now() + Duration::from_secs(authorization.expires_in);
        let mut interval = Duration::from_secs(authorization.interval);
        loop {
            if Instant::now() >= deadline {
                return Err(AuthError(
                    "el código de dispositivo de Twitch expiró".into(),
                ));
            }
            let response = self
                .post(
                    "/oauth2/token",
                    &[
                        ("client_id", &self.inner.client_id),
                        ("device_code", &authorization.device_code),
                        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ("scopes", &scopes),
                    ],
                )
                .await?;
            if response.status().is_success() {
                let token = parse_success::<TokenResponse>(response)
                    .await?
                    .into_token(None)?;
                let access = token.access_token.clone();
                self.persist(&token).await?;
                state.token = Some(token);
                return Ok(access);
            }
            let status = response.status();
            let body = response.text().await.map_err(|error| {
                AuthError(format!("no se pudo leer el error OAuth {status}: {error}"))
            })?;
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
                .unwrap_or(body);
            match message.as_str() {
                "authorization_pending" => {}
                "slow_down" => interval += Duration::from_secs(5),
                "expired_token" | "access_denied" => {
                    return Err(AuthError(format!(
                        "autorización Twitch terminada: {message}"
                    )));
                }
                _ => return Err(AuthError(format!("error OAuth {status}: {message}"))),
            }
            let wake = Instant::now() + interval;
            if wake >= deadline {
                return Err(AuthError(
                    "el código de dispositivo de Twitch expiró".into(),
                ));
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn post(
        &self,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<reqwest::Response, AuthError> {
        self.inner
            .http
            .post(format!("{}{path}", self.inner.identity_base))
            .form(form)
            .send()
            .await
            .map_err(|error| AuthError(format!("solicitud OAuth {path}: {error}")))
    }

    async fn persist(&self, token: &Token) -> Result<(), AuthError> {
        let path = &self.inner.token_path;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| path_error(parent, error))?;
        }
        let temporary = path.with_file_name("token.json.tmp");
        let bytes = serde_json::to_vec(token)
            .map_err(|error| AuthError(format!("no se pudo serializar el token: {error}")))?;
        tokio::fs::write(&temporary, bytes)
            .await
            .map_err(|error| path_error(&temporary, error))?;
        #[cfg(not(windows))]
        tokio::fs::rename(&temporary, path)
            .await
            .map_err(|error| path_error(path, error))?;
        #[cfg(windows)]
        if let Err(error) = tokio::fs::rename(&temporary, path).await {
            if !matches!(tokio::fs::try_exists(path).await, Ok(true)) {
                return Err(path_error(path, error));
            }
            tokio::fs::remove_file(path)
                .await
                .map_err(|error| path_error(path, error))?;
            tokio::fs::rename(&temporary, path)
                .await
                .map_err(|error| path_error(path, error))?;
        }
        Ok(())
    }
}

async fn parse_success<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T, AuthError> {
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| error.to_string());
        return Err(AuthError(format!("error OAuth {status}: {body}")));
    }
    response
        .json()
        .await
        .map_err(|error| AuthError(format!("respuesta OAuth inválida: {error}")))
}

fn path_error(path: &Path, error: std::io::Error) -> AuthError {
    AuthError(format!("{}: {error}", path.display()))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
