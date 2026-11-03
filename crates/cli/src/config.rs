use std::{
    collections::BTreeSet,
    error::Error,
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use streams_tools_adapters::twitch::{TwitchEvent, TwitchSettings};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub twitch: Option<TwitchConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TwitchConfig {
    client_id: String,
    channel: String,
    events: Option<Vec<TwitchEvent>>,
}

#[derive(Debug)]
pub(crate) struct ConfigError {
    path: PathBuf,
    field: String,
    reason: String,
}

impl ConfigError {
    fn new(path: &Path, field: &str, reason: String) -> Self {
        Self {
            path: path.into(),
            field: field.into(),
            reason,
        }
    }

    #[cfg(test)]
    pub(crate) fn field(&self) -> &str {
        &self.field
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: campo {}: {}",
            self.path.display(),
            self.field,
            self.reason
        )
    }
}

impl Error for ConfigError {}

pub(crate) fn load(path: &Path) -> Result<Config, ConfigError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => {
            return Err(ConfigError::new(path, "<file>", error.to_string()));
        }
    };
    let deserializer = toml::Deserializer::parse(&source)
        .map_err(|error| ConfigError::new(path, "<toml>", error.to_string()))?;
    let config: Config = serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let field = error.path().to_string();
        ConfigError::new(
            path,
            if field.is_empty() { "<toml>" } else { &field },
            error.into_inner().message().to_string(),
        )
    })?;
    config.validate(path)?;
    Ok(config)
}

impl Config {
    fn validate(&self, path: &Path) -> Result<(), ConfigError> {
        let Some(twitch) = &self.twitch else {
            return Ok(());
        };
        if twitch.client_id.trim().is_empty() {
            return Err(ConfigError::new(
                path,
                "twitch.client_id",
                "no puede estar vacío".into(),
            ));
        }
        if twitch.channel.trim().is_empty() {
            return Err(ConfigError::new(
                path,
                "twitch.channel",
                "no puede estar vacío".into(),
            ));
        }
        let mut events = BTreeSet::new();
        for event in twitch.events.iter().flatten() {
            if !events.insert(*event) {
                return Err(ConfigError::new(
                    path,
                    "twitch.events",
                    format!("evento duplicado `{}`", event_name(*event)),
                ));
            }
        }
        Ok(())
    }
}

impl TwitchConfig {
    pub(crate) fn into_settings(self, data_root: &Path) -> TwitchSettings {
        let mut settings = TwitchSettings::new(
            self.client_id.trim().into(),
            self.channel.trim().into(),
            data_root.join("twitch"),
        );
        settings.events = self
            .events
            .unwrap_or_else(|| TwitchEvent::ALL.to_vec())
            .into_iter()
            .collect();
        settings
    }
}

fn event_name(event: TwitchEvent) -> &'static str {
    match event {
        TwitchEvent::Follow => "follow",
        TwitchEvent::Sub => "sub",
        TwitchEvent::GiftSub => "gift_sub",
        TwitchEvent::Raid => "raid",
        TwitchEvent::Redemption => "redemption",
        TwitchEvent::Chat => "chat",
        TwitchEvent::Cheer => "cheer",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "streams-tools-config-{}-{}",
                std::process::id(),
                TEMP_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, source: &str) -> PathBuf {
            let path = self.0.join("config.toml");
            std::fs::write(&path, source).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_file_leaves_twitch_disabled() {
        let dir = TempDir::new();
        let config = load(&dir.0.join("missing.toml")).unwrap();
        assert!(config.twitch.is_none());
    }

    #[test]
    fn minimal_config_enables_all_events_and_trims_identity() {
        let dir = TempDir::new();
        let path = dir.write("[twitch]\nclient_id = ' client '\nchannel = ' canal '\n");
        let config = load(&path).unwrap();
        let settings = config.twitch.unwrap().into_settings(Path::new("data"));
        assert_eq!(settings.client_id, "client");
        assert_eq!(settings.channel, "canal");
        assert_eq!(settings.events, TwitchEvent::ALL.into_iter().collect());
        assert_eq!(settings.data_dir, Path::new("data").join("twitch"));
    }

    #[test]
    fn selected_and_empty_event_lists_are_preserved() {
        let dir = TempDir::new();
        let path = dir.write(
            "[twitch]\nclient_id = 'client'\nchannel = 'canal'\nevents = ['chat', 'cheer']\n",
        );
        let config = load(&path).unwrap();
        let settings = config.twitch.unwrap().into_settings(Path::new("data"));
        assert_eq!(
            settings.events,
            [TwitchEvent::Chat, TwitchEvent::Cheer]
                .into_iter()
                .collect()
        );
        let path = dir.write("[twitch]\nclient_id = 'client'\nchannel = 'canal'\nevents = []\n");
        let config = load(&path).unwrap();
        assert!(config.twitch.unwrap().events.unwrap().is_empty());
    }

    #[test]
    fn unknown_fields_report_their_exact_path_and_reason() {
        for (source, field) in [
            ("unknown = true\n", "unknown"),
            (
                "[twitch]\nclient_id = 'client'\nchannel = 'canal'\nunknown = true\n",
                "twitch.unknown",
            ),
        ] {
            let dir = TempDir::new();
            let path = dir.write(source);
            let error = load(&path).unwrap_err();
            assert_eq!(error.field(), field);
            let display = error.to_string();
            assert!(
                display.starts_with(&format!("{}: campo {field}: unknown field", path.display())),
                "unexpected error: {display}"
            );
            assert!(display.contains("expected"));
        }
    }

    #[test]
    fn unknown_event_reports_array_index_and_variant_error() {
        let dir = TempDir::new();
        let path =
            dir.write("[twitch]\nclient_id = 'client'\nchannel = 'canal'\nevents = ['unknown']\n");
        let error = load(&path).unwrap_err();
        assert_eq!(error.field(), "twitch.events[0]");
        assert!(error.to_string().contains("unknown variant"));
    }

    #[test]
    fn empty_identity_and_duplicate_events_have_exact_errors() {
        for (source, field) in [
            (
                "[twitch]\nclient_id = ' '\nchannel = 'canal'\n",
                "twitch.client_id",
            ),
            (
                "[twitch]\nclient_id = 'client'\nchannel = ' '\n",
                "twitch.channel",
            ),
        ] {
            let dir = TempDir::new();
            let error = load(&dir.write(source)).unwrap_err();
            assert_eq!(error.field(), field);
            assert_eq!(error.reason, "no puede estar vacío");
        }
        let dir = TempDir::new();
        let error = load(&dir.write(
            "[twitch]\nclient_id = 'client'\nchannel = 'canal'\nevents = ['chat', 'chat']\n",
        ))
        .unwrap_err();
        assert_eq!(error.field(), "twitch.events");
        assert_eq!(error.reason, "evento duplicado `chat`");
    }
}
