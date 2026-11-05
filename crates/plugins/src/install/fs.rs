use std::{
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{InstalledMetadata, MetadataStore, ValidatedPackage, metadata::validate_name};

static TRANSACTION_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct TransactionError(String);

pub struct FsTransaction {
    plugins_root: PathBuf,
    metadata: MetadataStore,
}

impl FsTransaction {
    pub fn new(plugins_root: &Path) -> Result<Self, TransactionError> {
        fs::create_dir_all(plugins_root).map_err(error)?;
        let plugins_root = fs::canonicalize(plugins_root).map_err(error)?;
        fs::create_dir_all(plugins_root.join(".streams-tools/tmp")).map_err(error)?;
        Ok(Self {
            metadata: MetadataStore::new(&plugins_root),
            plugins_root,
        })
    }

    pub fn publish(
        &self,
        package: &ValidatedPackage,
        metadata: &InstalledMetadata,
    ) -> Result<(), TransactionError> {
        metadata
            .validate(Some(&package.name))
            .map_err(|error| TransactionError(error.to_string()))?;
        let destination = self.child(&package.name)?;
        let metadata_path = self.metadata.path(&package.name).map_err(meta)?;
        if destination.exists() || metadata_path.exists() {
            return Err(TransactionError("plugin o metadata ya existe".into()));
        }
        fs::rename(package.plugin_dir(), &destination).map_err(error)?;
        if let Err(failure) = self.write_metadata(metadata, &metadata_path) {
            let _ = fs::rename(&destination, package.plugin_dir());
            return Err(failure);
        }
        Ok(())
    }

    pub fn update(
        &self,
        package: &ValidatedPackage,
        metadata: &InstalledMetadata,
    ) -> Result<Option<String>, TransactionError> {
        let destination = self.child(&package.name)?;
        let metadata_path = self.metadata.path(&package.name).map_err(meta)?;
        if !destination.is_dir() || self.metadata.read(&package.name).map_err(meta)?.is_none() {
            return Err(TransactionError("plugin no administrado".into()));
        }
        let backup = self.backup(&package.name, "plugin");
        let metadata_backup = self.backup(&package.name, "toml");
        fs::rename(&destination, &backup).map_err(error)?;
        if let Err(failure) = fs::rename(package.plugin_dir(), &destination).map_err(error) {
            let _ = fs::rename(&backup, &destination);
            return Err(failure);
        }
        if let Err(failure) = fs::rename(&metadata_path, &metadata_backup)
            .map_err(error)
            .and_then(|_| self.write_metadata(metadata, &metadata_path))
        {
            let _ = fs::remove_dir_all(&destination);
            let _ = fs::rename(&backup, &destination);
            let _ = fs::rename(&metadata_backup, &metadata_path);
            return Err(failure);
        }
        let mut warnings = Vec::new();
        if let Err(failure) = fs::remove_dir_all(backup) {
            warnings.push(format!("backup de plugin: {failure}"));
        }
        if let Err(failure) = fs::remove_file(metadata_backup) {
            warnings.push(format!("backup de metadata: {failure}"));
        }
        Ok((!warnings.is_empty()).then(|| {
            format!(
                "actualización confirmada; limpieza pendiente: {}",
                warnings.join("; ")
            )
        }))
    }

    pub fn remove(&self, name: &str) -> Result<(), TransactionError> {
        let destination = self.child(name)?;
        let metadata_path = self.metadata.path(name).map_err(meta)?;
        if !destination.is_dir() || self.metadata.read(name).map_err(meta)?.is_none() {
            return Err(TransactionError("plugin no administrado".into()));
        }
        let backup = self.backup(name, "removed");
        fs::rename(&destination, &backup).map_err(error)?;
        if let Err(failure) = fs::remove_file(&metadata_path).map_err(error) {
            let _ = fs::rename(&backup, &destination);
            return Err(failure);
        }
        fs::remove_dir_all(backup).map_err(error)
    }

    fn child(&self, name: &str) -> Result<PathBuf, TransactionError> {
        validate_name(name).map_err(meta)?;
        let child = self.plugins_root.join(name);
        if child.parent() != Some(self.plugins_root.as_path()) {
            return Err(TransactionError("destino fuera de plugins".into()));
        }
        Ok(child)
    }
    fn backup(&self, name: &str, suffix: &str) -> PathBuf {
        self.plugins_root.join(".streams-tools/tmp").join(format!(
            "{}-{}-backup-{name}-{suffix}",
            std::process::id(),
            TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }
    fn write_metadata(
        &self,
        metadata: &InstalledMetadata,
        destination: &Path,
    ) -> Result<(), TransactionError> {
        fs::create_dir_all(self.metadata.root()).map_err(error)?;
        let temporary = destination.with_extension(format!(
            "toml.tmp-{}",
            TRANSACTION_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let source =
            toml::to_string(metadata).map_err(|failure| TransactionError(failure.to_string()))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(error)?;
        file.write_all(source.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(error)?;
        fs::rename(&temporary, destination).map_err(error)
    }
}

pub(crate) fn cleanup_stale(plugins_root: &Path) -> Result<(), TransactionError> {
    let transaction = FsTransaction::new(plugins_root)?;
    let tmp = transaction.plugins_root.join(".streams-tools/tmp");
    for entry in fs::read_dir(&tmp).map_err(error)? {
        let entry = entry.map_err(error)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(pid) = installer_entry_pid(&name) else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let path = entry.path();
        if path.parent() != Some(tmp.as_path()) {
            return Err(TransactionError("temporal fuera del staging".into()));
        }
        let kind = fs::symlink_metadata(&path).map_err(error)?.file_type();
        if kind.is_dir() {
            fs::remove_dir_all(path).map_err(error)?;
        } else {
            fs::remove_file(path).map_err(error)?;
        }
    }
    Ok(())
}

fn installer_entry_pid(name: &str) -> Option<u32> {
    let mut parts = name.splitn(3, '-');
    let pid = parts.next()?.parse().ok()?;
    parts.next()?.parse::<u64>().ok()?;
    if let Some(backup) = parts.next() {
        let (name, suffix) = backup.strip_prefix("backup-")?.rsplit_once('-')?;
        if !matches!(suffix, "plugin" | "toml" | "removed") || validate_name(name).is_err() {
            return None;
        }
    }
    Some(pid)
}

fn error(error: std::io::Error) -> TransactionError {
    TransactionError(error.to_string())
}
fn meta(error: impl fmt::Display) -> TransactionError {
    TransactionError(error.to_string())
}
impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for TransactionError {}
