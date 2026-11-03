use super::YoutubeSettings;
use serde_json::Value;
use streams_tools_core::Event;

pub(crate) enum Live {
    Online(String),
    Offline,
}

pub(crate) struct Bootstrap {
    pub key: String,
    pub client_version: String,
    pub continuation: String,
    pub history: Vec<Event>,
}

pub(crate) async fn live(
    settings: &YoutubeSettings,
    http: &reqwest::Client,
) -> Result<Live, String> {
    if let Some(video_id) = &settings.video_id {
        return Ok(Live::Online(video_id.clone()));
    }
    let channel = settings
        .channel
        .as_deref()
        .ok_or_else(|| "YouTube requiere channel o video_id".to_string())?;
    let path = if channel.starts_with("UC") {
        format!("/channel/{channel}/live")
    } else {
        format!("/@{channel}/live")
    };
    let html = get(http, &format!("{}{path}", base(&settings.urls.web))).await?;
    let player = if html.contains("ytInitialPlayerResponse") {
        Some(assigned_json(&html, "ytInitialPlayerResponse")?)
    } else {
        None
    };
    if let Some(player) = player {
        let is_live = true_key(&player, "isLiveNow") || true_key(&player, "isLive");
        if !is_live {
            return Ok(Live::Offline);
        }
        if let Some(id) = player
            .pointer("/videoDetails/videoId")
            .and_then(Value::as_str)
        {
            return Ok(Live::Online(id.into()));
        }
    }
    if live_marker(&html)
        && let Some(id) = canonical_video_id(&html)
    {
        return Ok(Live::Online(id));
    }
    Ok(Live::Offline)
}

pub(crate) async fn bootstrap(
    settings: &YoutubeSettings,
    http: &reqwest::Client,
    video_id: &str,
) -> Result<Bootstrap, String> {
    let url = format!(
        "{}/live_chat?is_popout=1&v={video_id}",
        base(&settings.urls.web)
    );
    let html = get(http, &url).await?;
    let config = call_json(&html, "ytcfg.set(")?;
    let initial = assigned_json(&html, "ytInitialData")?;
    let renderer = find_key(&initial, "liveChatRenderer")
        .ok_or_else(|| "ytInitialData sin liveChatRenderer".to_string())?;
    let continuation = next_continuation(renderer)
        .map(|(continuation, _)| continuation)
        .ok_or_else(|| "liveChatRenderer sin continuación inicial".to_string())?;
    let history = renderer
        .get("actions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(super::map::event)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("historial de YouTube inválido: {error}"))?
        .into_iter()
        .flatten()
        .collect();
    Ok(Bootstrap {
        key: required_string(&config, "INNERTUBE_API_KEY")?,
        client_version: required_string(&config, "INNERTUBE_CONTEXT_CLIENT_VERSION")?,
        continuation,
        history,
    })
}

pub(crate) fn next_continuation(value: &Value) -> Option<(String, u64)> {
    let values = value.get("continuations")?.as_array()?;
    for item in values {
        for kind in [
            "invalidationContinuationData",
            "timedContinuationData",
            "reloadContinuationData",
        ] {
            let Some(continuation) = item.get(kind) else {
                continue;
            };
            let token = continuation.get("continuation")?.as_str()?.to_owned();
            let timeout = continuation
                .get("timeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            return Some((token, timeout));
        }
    }
    None
}

async fn get(http: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response
        .url()
        .host_str()
        .is_some_and(|host| host == "consent.youtube.com")
    {
        return Err(consent());
    }
    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::TOO_MANY_REQUESTS
    ) {
        return Err(consent());
    }
    if !status.is_success() {
        return Err(format!("YouTube devolvió HTTP {status}"));
    }
    response.text().await.map_err(|error| error.to_string())
}

fn consent() -> String {
    "YouTube exige consentimiento o bloqueó la solicitud; configura youtube.video_id para omitir la resolución del canal; streams-tools no evade controles de consentimiento ni anti-bot".into()
}

fn assigned_json(source: &str, name: &str) -> Result<Value, String> {
    let start = source
        .find(name)
        .ok_or_else(|| format!("HTML sin {name}"))?;
    let tail = &source[start + name.len()..];
    let equals = tail
        .find('=')
        .ok_or_else(|| format!("asignación {name} sin ="))?;
    json_prefix(&tail[equals + 1..]).map_err(|error| format!("{name} inválido: {error}"))
}

fn call_json(source: &str, marker: &str) -> Result<Value, String> {
    let start = source
        .find(marker)
        .ok_or_else(|| format!("HTML sin {marker}"))?;
    json_prefix(&source[start + marker.len()..])
        .map_err(|error| format!("{marker} inválido: {error}"))
}

fn json_prefix(source: &str) -> Result<Value, String> {
    let source = source.trim_start();
    let mut stream = serde_json::Deserializer::from_str(source).into_iter::<Value>();
    stream
        .next()
        .ok_or_else(|| "JSON embebido ausente".to_string())?
        .map_err(|error| error.to_string())
}

fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(values) => values
            .get(key)
            .or_else(|| values.values().find_map(|value| find_key(value, key))),
        Value::Array(values) => values.iter().find_map(|value| find_key(value, key)),
        _ => None,
    }
}

fn true_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(values) => {
            values.get(key).and_then(Value::as_bool) == Some(true)
                || values.values().any(|value| true_key(value, key))
        }
        Value::Array(values) => values.iter().any(|value| true_key(value, key)),
        _ => false,
    }
}

fn required_string(value: &Value, key: &str) -> Result<String, String> {
    find_key(value, key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("ytcfg sin {key}"))
}

fn live_marker(html: &str) -> bool {
    ["\"isLiveNow\":true", "\"isLive\":true"]
        .into_iter()
        .any(|marker| html.contains(marker))
}

fn canonical_video_id(html: &str) -> Option<String> {
    for tail in html.split("<link").skip(1) {
        let Some((tag, _)) = tail.split_once('>') else {
            continue;
        };
        if !tag.contains("rel=\"canonical\"") && !tag.contains("rel='canonical'") {
            continue;
        }
        let Some(watch) = tag.find("watch?v=") else {
            continue;
        };
        let value = &tag[watch + "watch?v=".len()..];
        let end = value.find(['&', '"', '\'', '<']).unwrap_or(value.len());
        if end > 0 {
            return Some(value[..end].into());
        }
    }
    None
}

fn base(value: &str) -> &str {
    value.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_player_canonical_config_history_and_all_continuation_kinds() {
        let live = include_str!("../../tests/fixtures/youtube/live.html");
        let player = assigned_json(live, "ytInitialPlayerResponse").unwrap();
        assert_eq!(player["videoDetails"]["videoId"], "video-live");
        assert!(true_key(&player, "isLiveNow"));
        assert_eq!(canonical_video_id(live).as_deref(), Some("canonical-live"));

        let chat = include_str!("../../tests/fixtures/youtube/live_chat.html");
        let config = call_json(chat, "ytcfg.set(").unwrap();
        assert_eq!(
            required_string(&config, "INNERTUBE_API_KEY").unwrap(),
            "test-key"
        );
        let initial = assigned_json(chat, "ytInitialData").unwrap();
        let renderer = find_key(&initial, "liveChatRenderer").unwrap();
        assert_eq!(next_continuation(renderer).unwrap().0, "initial-token");
        for kind in [
            "invalidationContinuationData",
            "timedContinuationData",
            "reloadContinuationData",
        ] {
            let value = serde_json::json!({"continuations":[{(kind):{
                "continuation":"token", "timeoutMs":1234
            }}]});
            assert_eq!(next_continuation(&value), Some(("token".into(), 1234)));
        }
    }
}
