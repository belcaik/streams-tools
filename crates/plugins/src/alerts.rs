use std::{
    collections::HashSet,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const PLACEHOLDERS: [&str; 9] = [
    "user", "amount", "currency", "months", "count", "viewers", "reward", "message", "tier",
];

type Validation = Result<(), (String, String)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertEvent {
    Follow,
    Sub,
    GiftSub,
    Raid,
    Donation,
    Redemption,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformName {
    Twitch,
    Kick,
    Youtube,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    pub currency: Option<String>,
    pub min_amount: Option<f64>,
    pub max_amount: Option<f64>,
    pub min_months: Option<u64>,
    pub min_count: Option<u64>,
    pub min_viewers: Option<u64>,
    pub reward_title: Option<String>,
    pub platforms: Option<Vec<PlatformName>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variation {
    pub template: Option<String>,
    pub image: Option<String>,
    pub sound: Option<String>,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertManifest {
    pub event: AlertEvent,
    pub template: String,
    #[serde(default = "default_priority")]
    pub priority: i32,
    #[serde(default = "default_duration_ms")]
    pub duration_ms: u64,
    #[serde(default = "default_volume")]
    pub volume: f64,
    pub sound: Option<String>,
    #[serde(default)]
    pub conditions: Conditions,
    #[serde(default)]
    pub variations: Vec<Variation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertLoadError {
    path: PathBuf,
    field: String,
    reason: String,
}

const fn default_priority() -> i32 {
    0
}

const fn default_duration_ms() -> u64 {
    5000
}

const fn default_volume() -> f64 {
    1.0
}

const fn default_weight() -> u32 {
    1
}

impl AlertManifest {
    pub fn load(alert_dir: &Path) -> Result<Self, AlertLoadError> {
        let path = alert_dir.join("alert.toml");
        let source = fs::read_to_string(&path)
            .map_err(|error| AlertLoadError::new(&path, "<file>", error.to_string()))?;
        let deserializer = toml::Deserializer::parse(&source)
            .map_err(|error| AlertLoadError::new(&path, "<toml>", error.message().to_string()))?;
        let manifest: Self = serde_path_to_error::deserialize(deserializer).map_err(|error| {
            let field = error.path().to_string();
            AlertLoadError::new(
                &path,
                if field.is_empty() { "<toml>" } else { &field },
                error.into_inner().message().to_string(),
            )
        })?;
        manifest
            .validate(alert_dir)
            .map_err(|(field, reason)| AlertLoadError::new(&path, field, reason))?;
        Ok(manifest)
    }

    fn validate(&self, alert_dir: &Path) -> Validation {
        let name = alert_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ("name".into(), "alert folder is not UTF-8".into()))?;
        validate_name(name).map_err(|reason| ("name".into(), reason))?;
        validate_template(&self.template).map_err(|reason| ("template".into(), reason))?;
        if !(1..=300_000).contains(&self.duration_ms) {
            return Err(("duration_ms".into(), "must be between 1 and 300000".into()));
        }
        if !self.volume.is_finite() || !(0.0..=1.0).contains(&self.volume) {
            return Err(("volume".into(), "must be finite and between 0 and 1".into()));
        }
        self.validate_conditions()?;
        let root =
            fs::canonicalize(alert_dir).map_err(|error| ("<file>".into(), error.to_string()))?;
        validate_asset(&root, "style.css", "style.css")?;
        if let Some(sound) = &self.sound {
            validate_asset(&root, sound, "sound")?;
        }
        for (index, variation) in self.variations.iter().enumerate() {
            let prefix = format!("variations.{index}");
            if variation.template.is_none()
                && variation.image.is_none()
                && variation.sound.is_none()
            {
                return Err((prefix, "must change template, image or sound".into()));
            }
            if !(1..=10_000).contains(&variation.weight) {
                return Err((
                    format!("{prefix}.weight"),
                    "must be between 1 and 10000".into(),
                ));
            }
            if let Some(template) = &variation.template {
                validate_template(template)
                    .map_err(|reason| (format!("{prefix}.template"), reason))?;
            }
            for (asset, suffix) in [(&variation.image, "image"), (&variation.sound, "sound")] {
                if let Some(asset) = asset {
                    validate_asset(&root, asset, &format!("{prefix}.{suffix}"))?;
                }
            }
        }
        Ok(())
    }

    fn validate_conditions(&self) -> Validation {
        let conditions = &self.conditions;
        for (field, present, event) in [
            (
                "currency",
                conditions.currency.is_some(),
                AlertEvent::Donation,
            ),
            (
                "min_amount",
                conditions.min_amount.is_some(),
                AlertEvent::Donation,
            ),
            (
                "max_amount",
                conditions.max_amount.is_some(),
                AlertEvent::Donation,
            ),
            (
                "min_months",
                conditions.min_months.is_some(),
                AlertEvent::Sub,
            ),
            (
                "min_count",
                conditions.min_count.is_some(),
                AlertEvent::GiftSub,
            ),
            (
                "min_viewers",
                conditions.min_viewers.is_some(),
                AlertEvent::Raid,
            ),
            (
                "reward_title",
                conditions.reward_title.is_some(),
                AlertEvent::Redemption,
            ),
        ] {
            if present && self.event != event {
                return Err((
                    format!("conditions.{field}"),
                    format!("sólo es válido para {event:?}"),
                ));
            }
        }
        if conditions.currency.as_deref().is_some_and(str::is_empty) {
            return Err(("conditions.currency".into(), "cannot be empty".into()));
        }
        for (field, value) in [
            ("min_amount", conditions.min_amount),
            ("max_amount", conditions.max_amount),
        ] {
            if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
                return Err((
                    format!("conditions.{field}"),
                    "must be finite and at least 0".into(),
                ));
            }
        }
        if let (Some(min), Some(max)) = (conditions.min_amount, conditions.max_amount)
            && max < min
        {
            return Err((
                "conditions.max_amount".into(),
                "must be at least min_amount".into(),
            ));
        }
        for (field, value) in [
            ("min_months", conditions.min_months),
            ("min_count", conditions.min_count),
            ("min_viewers", conditions.min_viewers),
        ] {
            if value == Some(0) {
                return Err((
                    format!("conditions.{field}"),
                    "must be greater than 0".into(),
                ));
            }
        }
        if let Some(platforms) = &conditions.platforms {
            if platforms.is_empty() {
                return Err(("conditions.platforms".into(), "cannot be empty".into()));
            }
            let mut seen = HashSet::new();
            if platforms.iter().any(|platform| !seen.insert(platform)) {
                return Err((
                    "conditions.platforms".into(),
                    "cannot contain duplicates".into(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!("invalid alert name `{name}`"));
    }
    Ok(())
}

fn validate_template(template: &str) -> Result<(), String> {
    let mut rest = template;
    loop {
        match (rest.find('{'), rest.find('}')) {
            (Some(open), Some(close)) if close < open => {
                return Err("unbalanced braces".into());
            }
            (Some(open), _) => {
                let after = &rest[open + 1..];
                let close = after
                    .find('}')
                    .ok_or_else(|| "unbalanced braces".to_string())?;
                let placeholder = &after[..close];
                if !PLACEHOLDERS.contains(&placeholder) {
                    return Err(format!("unknown placeholder `{{{placeholder}}}`"));
                }
                rest = &after[close + 1..];
            }
            (None, Some(_)) => return Err("unbalanced braces".into()),
            (None, None) => return Ok(()),
        }
    }
}

fn validate_asset(root: &Path, asset: &str, field: &str) -> Validation {
    let path = asset_path(root, asset).map_err(|reason| (field.to_string(), reason))?;
    if !path.is_file() {
        return Err((field.to_string(), "asset is not a regular file".into()));
    }
    Ok(())
}

fn asset_path(root: &Path, asset: &str) -> Result<PathBuf, String> {
    if asset.is_empty()
        || asset.starts_with('/')
        || asset.contains('\\')
        || asset.split('/').any(|component| {
            component.is_empty()
                || component == "."
                || component == ".."
                || !component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
    {
        return Err(format!("invalid portable asset path `{asset}`"));
    }
    let path = fs::canonicalize(root.join(asset)).map_err(|error| error.to_string())?;
    if !path.starts_with(root) {
        return Err("asset escapes the alert directory".into());
    }
    Ok(path)
}

impl AlertLoadError {
    fn new(path: &Path, field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            field: field.into(),
            reason: reason.into(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn field(&self) -> &str {
        &self.field
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for AlertLoadError {
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

impl Error for AlertLoadError {}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertVariationView {
    pub template: Option<String>,
    pub image: Option<String>,
    pub sound: Option<String>,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertView {
    pub name: String,
    pub event: AlertEvent,
    pub template: String,
    pub priority: i32,
    pub duration_ms: u64,
    pub volume: f64,
    pub conditions: serde_json::Value,
    pub sound: Option<String>,
    pub html: Option<String>,
    pub css: String,
    pub js: Option<String>,
    pub variations: Vec<AlertVariationView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertPackView {
    pub name: String,
    pub alerts: Vec<AlertView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlertIssue {
    pub pack: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alert: Option<String>,
    pub file: String,
    pub field: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AlertIndex {
    pub packs: Vec<AlertPackView>,
    pub errors: Vec<AlertIssue>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AlertRegistry {
    index: AlertIndex,
    roots: Vec<(String, PathBuf)>,
}

impl AlertRegistry {
    pub fn discover(root: &Path) -> std::io::Result<Self> {
        let entries = match directories(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        let mut registry = Self::default();
        for entry in entries {
            let pack = entry.file_name().to_string_lossy().into_owned();
            if let Err(reason) = validate_name(&pack) {
                registry.issue(&pack, None, pack.clone(), "name", reason);
                continue;
            }
            let pack_root = match fs::canonicalize(entry.path()) {
                Ok(path) => path,
                Err(error) => {
                    registry.issue(&pack, None, pack.clone(), "<file>", error.to_string());
                    continue;
                }
            };
            let mut view = AlertPackView {
                name: pack.clone(),
                alerts: Vec::new(),
            };
            let alerts = match directories(&entry.path()) {
                Ok(alerts) => alerts,
                Err(error) => {
                    registry.issue(&pack, None, pack.clone(), "<file>", error.to_string());
                    registry.roots.push((pack, pack_root));
                    registry.index.packs.push(view);
                    continue;
                }
            };
            for alert_entry in alerts {
                let alert = alert_entry.file_name().to_string_lossy().into_owned();
                let alert_dir = alert_entry.path();
                if !alert_dir.join("alert.toml").is_file() {
                    continue;
                }
                let file = format!("{pack}/{alert}/alert.toml");
                if let Err(reason) = validate_name(&alert) {
                    registry.issue(&pack, Some(alert), file, "name", reason);
                    continue;
                }
                match AlertManifest::load(&alert_dir) {
                    Ok(manifest) => view
                        .alerts
                        .push(alert_view(&pack, &alert, &alert_dir, manifest)),
                    Err(error) => {
                        registry.issue(&pack, Some(alert), file, error.field(), error.reason())
                    }
                }
            }
            registry.roots.push((pack, pack_root));
            registry.index.packs.push(view);
        }
        Ok(registry)
    }

    pub fn index(&self) -> &AlertIndex {
        &self.index
    }

    pub fn index_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.index).expect("validated alert index should serialize")
    }

    pub fn asset_roots(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.roots
            .iter()
            .map(|(name, path)| (name.as_str(), path.as_path()))
    }

    pub fn valid_alert_count(&self) -> usize {
        self.index.packs.iter().map(|pack| pack.alerts.len()).sum()
    }

    pub fn errors(&self) -> &[AlertIssue] {
        &self.index.errors
    }

    fn issue(
        &mut self,
        pack: &str,
        alert: Option<String>,
        file: String,
        field: &str,
        reason: impl Into<String>,
    ) {
        self.index.errors.push(AlertIssue {
            pack: pack.into(),
            alert,
            file,
            field: field.into(),
            reason: reason.into(),
        });
    }
}

fn directories(path: &Path) -> std::io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.retain(|entry| entry.path().is_dir());
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn alert_view(pack: &str, alert: &str, dir: &Path, manifest: AlertManifest) -> AlertView {
    let asset = |path: &str| format!("/alerts/{pack}/{alert}/{path}");
    let optional = |name: &str| dir.join(name).is_file().then(|| asset(name));
    let conditions = serde_json::to_value(&manifest.conditions)
        .expect("validated alert conditions should serialize");
    let mut conditions = conditions;
    conditions
        .as_object_mut()
        .expect("conditions are an object")
        .retain(|_, value| !value.is_null());
    AlertView {
        name: alert.into(),
        event: manifest.event,
        template: manifest.template,
        priority: manifest.priority,
        duration_ms: manifest.duration_ms,
        volume: manifest.volume,
        conditions,
        sound: manifest.sound.as_deref().map(asset),
        html: optional("index.html"),
        css: asset("style.css"),
        js: optional("script.js"),
        variations: manifest
            .variations
            .into_iter()
            .map(|variation| AlertVariationView {
                template: variation.template,
                image: variation.image.as_deref().map(asset),
                sound: variation.sound.as_deref().map(asset),
                weight: variation.weight,
            })
            .collect(),
    }
}
