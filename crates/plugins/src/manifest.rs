use std::{
    collections::HashSet,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub permissions: Permissions,
    pub worker: Option<Worker>,
    #[serde(default)]
    pub config: toml::Table,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    pub listens: Vec<String>,
    pub emits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    pub command: Vec<String>,
    pub command_windows: Option<Vec<String>>,
    pub command_macos: Option<Vec<String>>,
    pub command_linux: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginOs {
    Windows,
    Macos,
    Linux,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    path: PathBuf,
    field: String,
    reason: String,
}

impl Permissions {
    pub fn listens_to(&self, kind: &str) -> bool {
        self.listens.iter().any(|item| item == "*" || item == kind)
    }

    pub fn may_emit(&self, kind: &str) -> bool {
        self.emits.iter().any(|item| item == kind)
    }
}

impl Worker {
    pub fn command_for(&self, os: PluginOs) -> &[String] {
        match os {
            PluginOs::Windows => self.command_windows.as_deref(),
            PluginOs::Macos => self.command_macos.as_deref(),
            PluginOs::Linux => self.command_linux.as_deref(),
            PluginOs::Other => None,
        }
        .unwrap_or(&self.command)
    }
}

impl PluginOs {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

impl Manifest {
    pub fn load(plugin_dir: &Path) -> Result<Self, ManifestError> {
        let path = plugin_dir.join("plugin.toml");
        let source = fs::read_to_string(&path)
            .map_err(|error| ManifestError::new(&path, "<file>", error.to_string()))?;
        let deserializer = toml::Deserializer::parse(&source)
            .map_err(|error| ManifestError::new(&path, "<toml>", error.to_string()))?;
        let manifest: Self = serde_path_to_error::deserialize(deserializer).map_err(|error| {
            let field = error.path().to_string();
            ManifestError::new(
                &path,
                if field.is_empty() { "<toml>" } else { &field },
                error.into_inner().to_string(),
            )
        })?;
        manifest
            .validate(plugin_dir)
            .map_err(|(field, reason)| ManifestError::new(&path, field, reason))?;
        Ok(manifest)
    }

    pub fn config_json(&self) -> Value {
        serde_json::to_value(&self.config)
            .expect("config JSON conversion was validated during load")
    }

    fn validate(&self, plugin_dir: &Path) -> Result<(), (String, String)> {
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(("name".into(), format!("invalid name `{}`", self.name)));
        }
        let folder = plugin_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| ("name".into(), "plugin folder is not UTF-8".into()))?;
        if self.name != folder {
            return Err((
                "name".into(),
                format!("`{}` does not match folder `{folder}`", self.name),
            ));
        }
        if self.version.is_empty() {
            return Err(("version".into(), "version cannot be empty".into()));
        }
        if self
            .description
            .as_deref()
            .is_some_and(|text| text.trim().is_empty())
        {
            return Err(("description".into(), "description cannot be empty".into()));
        }
        validate_list("permissions.listens", &self.permissions.listens, true)?;
        validate_list("permissions.emits", &self.permissions.emits, false)?;
        if let Some(worker) = &self.worker {
            for (field, command) in [
                ("worker.command", Some(&worker.command)),
                ("worker.command_windows", worker.command_windows.as_ref()),
                ("worker.command_macos", worker.command_macos.as_ref()),
                ("worker.command_linux", worker.command_linux.as_ref()),
            ] {
                if let Some(command) = command {
                    validate_command(field, command)?;
                }
            }
        }
        serde_json::to_value(&self.config)
            .map_err(|error| ("config".into(), format!("not JSON serializable: {error}")))?;
        Ok(())
    }
}

fn validate_list(field: &str, values: &[String], wildcard: bool) -> Result<(), (String, String)> {
    let mut seen = HashSet::new();
    for value in values {
        if value.is_empty() {
            return Err((field.into(), "entries cannot be empty".into()));
        }
        if !seen.insert(value.as_str()) {
            return Err((field.into(), format!("duplicate entry `{value}`")));
        }
        if value == "*" && (!wildcard || values.len() != 1) {
            return Err((field.into(), "invalid use of wildcard `*`".into()));
        }
    }
    Ok(())
}

fn validate_command(field: &str, command: &[String]) -> Result<(), (String, String)> {
    if command.is_empty() {
        return Err((field.into(), "command cannot be empty".into()));
    }
    if let Some(index) = command.iter().position(String::is_empty) {
        return Err((field.into(), format!("empty command element {index}")));
    }
    Ok(())
}

impl ManifestError {
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

impl fmt::Display for ManifestError {
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

impl Error for ManifestError {}
