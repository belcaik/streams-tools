use std::{
    error::Error,
    fmt,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use serde_json::{Map, Value, json};
use streams_tools_core::{Event, EventData, Platform, PluginData, User};

use crate::Permissions;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerOutput {
    #[serde(rename = "type")]
    kind: String,
    data: Map<String, Value>,
    user: Option<User>,
}

#[derive(Debug)]
pub(crate) enum ProtocolError {
    Json(serde_json::Error),
    EmptyType,
    Permission { plugin: String, kind: String },
    CorePayload(serde_json::Error),
}

pub(crate) fn parse_output(
    line: &str,
    plugin_name: &str,
    permissions: &Permissions,
    counter: &AtomicU64,
) -> Result<Event, ProtocolError> {
    let output: WorkerOutput = serde_json::from_str(line).map_err(ProtocolError::Json)?;
    if output.kind.is_empty() {
        return Err(ProtocolError::EmptyType);
    }
    if !permissions.may_emit(&output.kind) {
        return Err(ProtocolError::Permission {
            plugin: plugin_name.into(),
            kind: output.kind,
        });
    }
    let event = if is_core(&output.kind) {
        serde_json::from_value(json!({"type": output.kind, "data": output.data}))
            .map_err(ProtocolError::CorePayload)?
    } else {
        EventData::Plugin(PluginData {
            name: output.kind,
            data: Value::Object(output.data),
        })
    };
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let sequence = counter.fetch_add(1, Ordering::Relaxed);
    Ok(Event {
        id: format!("{plugin_name}-{sequence}-{}", elapsed.as_nanos()),
        platform: Platform::Plugin(plugin_name.into()),
        event,
        user: output.user.unwrap_or_else(|| User {
            id: plugin_name.into(),
            name: plugin_name.into(),
            roles: Vec::new(),
        }),
        ts: format_utc(elapsed),
    })
}

fn is_core(kind: &str) -> bool {
    matches!(
        kind,
        "follow" | "sub" | "gift_sub" | "raid" | "donation" | "redemption" | "chat"
    )
}

fn format_utc(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let days = (seconds / 86_400) as i64;
    let within_day = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        month,
        within_day / 3_600,
        (within_day / 60) % 60,
        within_day % 60,
        elapsed.subsec_millis()
    )
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "invalid worker JSON: {error}"),
            Self::EmptyType => formatter.write_str("worker event type cannot be empty"),
            Self::Permission { plugin, kind } => {
                write!(formatter, "plugin `{plugin}` may not emit `{kind}`")
            }
            Self::CorePayload(error) => write!(formatter, "invalid core event payload: {error}"),
        }
    }
}

impl Error for ProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn permissions(emits: &[&str]) -> Permissions {
        Permissions {
            listens: Vec::new(),
            emits: emits.iter().map(|kind| (*kind).into()).collect(),
        }
    }

    #[test]
    fn plugin_event_gets_identity_timestamp_and_default_user() {
        let event = parse_output(
            r#"{"type":"observed","data":{"value":1}}"#,
            "echo",
            &permissions(&["observed"]),
            &AtomicU64::new(7),
        )
        .expect("event should parse");
        assert_eq!(event.platform, Platform::Plugin("echo".into()));
        assert_eq!(event.user.id, "echo");
        assert!(event.id.starts_with("echo-7-"));
        assert!(event.ts.ends_with('Z') && event.ts.len() == 24);
        assert!(matches!(event.event, EventData::Plugin(_)));
    }

    #[test]
    fn provided_user_is_preserved() {
        let event = parse_output(
            r#"{"type":"observed","data":{},"user":{"id":"u","name":"User","roles":["mod"]}}"#,
            "echo",
            &permissions(&["observed"]),
            &AtomicU64::new(0),
        )
        .expect("event should parse");
        assert_eq!(event.user.name, "User");
        assert_eq!(event.user.roles, ["mod"]);
    }

    #[test]
    fn undeclared_types_are_rejected_even_when_core() {
        for kind in ["other", "chat"] {
            let line = format!(r#"{{"type":"{kind}","data":{{}}}}"#);
            let error = parse_output(&line, "echo", &permissions(&[]), &AtomicU64::new(0))
                .expect_err("permission should fail");
            assert!(matches!(error, ProtocolError::Permission { .. }));
        }
    }

    #[test]
    fn declared_core_type_uses_typed_payload() {
        let event = parse_output(
            r#"{"type":"chat","data":{"message":"hello"}}"#,
            "echo",
            &permissions(&["chat"]),
            &AtomicU64::new(0),
        )
        .expect("chat should parse");
        assert!(matches!(event.event, EventData::Chat(_)));
        let error = parse_output(
            r#"{"type":"chat","data":{"wrong":true}}"#,
            "echo",
            &permissions(&["chat"]),
            &AtomicU64::new(0),
        )
        .expect_err("invalid chat should fail");
        assert!(matches!(error, ProtocolError::CorePayload(_)));
    }

    #[test]
    fn invalid_json_data_and_empty_type_are_rejected() {
        for line in [
            "not-json",
            r#"{"type":"observed","data":[]}"#,
            r#"{"type":"","data":{}}"#,
        ] {
            let result = parse_output(
                line,
                "echo",
                &permissions(&["observed"]),
                &AtomicU64::new(0),
            );
            assert!(result.is_err(), "input should fail: {line}");
        }
    }

    #[test]
    fn utc_conversion_handles_epoch_next_day_and_leap_date() {
        assert_eq!(format_utc(Duration::ZERO), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_utc(Duration::from_secs(86_401)),
            "1970-01-02T00:00:01.000Z"
        );
        assert_eq!(
            format_utc(Duration::from_millis(1_582_934_423_456)),
            "2020-02-29T00:00:23.456Z"
        );
    }
}
