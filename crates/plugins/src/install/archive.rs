use std::{
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use semver::Version;

use super::DownloadedPackage;
use crate::Manifest;

const ENTRY_LIMIT: usize = 2048;
const FILE_LIMIT: u64 = 64 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 250 * 1024 * 1024;
static STAGING_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Portability {
    Target,
    Any,
}

#[derive(Debug)]
pub struct Staging {
    root: PathBuf,
}

#[derive(Debug)]
pub struct ValidatedPackage {
    staging: Staging,
    pub name: String,
    pub manifest: Manifest,
    pub version: Version,
    pub asset: String,
    pub sha256: String,
    pub portability: Portability,
}

#[derive(Debug)]
pub struct ArchiveError(String);

impl Staging {
    pub fn create(plugins_root: &Path) -> Result<Self, ArchiveError> {
        let tmp = plugins_root.join(".streams-tools/tmp");
        fs::create_dir_all(&tmp).map_err(io_error)?;
        for _ in 0..100 {
            let root = tmp.join(format!(
                "{}-{}",
                std::process::id(),
                STAGING_ID.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Ok(Self { root }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(error)),
            }
        }
        Err(ArchiveError("no se pudo reservar staging único".into()))
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn archive_path(&self) -> PathBuf {
        self.root.join("package.zip")
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl ValidatedPackage {
    pub fn plugin_dir(&self) -> PathBuf {
        self.staging.root.join("unpacked").join(&self.name)
    }
}

pub fn validate_package(
    staging: Staging,
    downloaded: DownloadedPackage,
    asset: &str,
    release_tag: &str,
    target: &str,
) -> Result<ValidatedPackage, ArchiveError> {
    if downloaded.path.parent() != Some(staging.root()) {
        return Err(ArchiveError("el ZIP no pertenece al staging".into()));
    }
    let (prefix, portability) = asset_identity(asset, target)?;
    let unpacked = staging.root.join("unpacked");
    fs::create_dir(&unpacked).map_err(io_error)?;
    extract(&downloaded.path, &unpacked, prefix)?;
    let plugin_dir = unpacked.join(prefix);
    let manifest = Manifest::load(&plugin_dir).map_err(|error| ArchiveError(error.to_string()))?;
    let version_text = release_tag
        .strip_prefix('v')
        .ok_or_else(|| ArchiveError("tag sin prefijo v".into()))?;
    let version = Version::parse(version_text)
        .map_err(|error| ArchiveError(format!("tag inválido: {error}")))?;
    let manifest_version = Version::parse(&manifest.version)
        .map_err(|error| ArchiveError(format!("versión de manifiesto inválida: {error}")))?;
    if version != manifest_version {
        return Err(ArchiveError(
            "tag y versión del manifiesto no coinciden".into(),
        ));
    }
    if manifest.name != prefix {
        return Err(ArchiveError(
            "asset y nombre del manifiesto no coinciden".into(),
        ));
    }
    if portability == Portability::Any && manifest.worker.is_some() {
        return Err(ArchiveError("un asset any no puede contener worker".into()));
    }
    Ok(ValidatedPackage {
        staging,
        name: manifest.name.clone(),
        manifest,
        version,
        asset: asset.into(),
        sha256: downloaded.sha256,
        portability,
    })
}

fn asset_identity<'a>(
    asset: &'a str,
    target: &str,
) -> Result<(&'a str, Portability), ArchiveError> {
    let target_suffix = format!("-{target}.zip");
    if let Some(prefix) = asset
        .strip_suffix(&target_suffix)
        .filter(|value| !value.is_empty())
    {
        Ok((prefix, Portability::Target))
    } else if let Some(prefix) = asset
        .strip_suffix("-any.zip")
        .filter(|value| !value.is_empty())
    {
        Ok((prefix, Portability::Any))
    } else {
        Err(ArchiveError("asset no corresponde al target".into()))
    }
}

fn extract(path: &Path, destination: &Path, expected_root: &str) -> Result<(), ArchiveError> {
    let file = fs::File::open(path).map_err(io_error)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
    if archive.len() > ENTRY_LIMIT {
        return Err(ArchiveError("ZIP supera 2048 entradas".into()));
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let raw = std::str::from_utf8(entry.name_raw())
            .map_err(|_| ArchiveError("path ZIP no es UTF-8".into()))?;
        let relative = portable_path(raw, expected_root)?;
        let mode = entry
            .unix_mode()
            .unwrap_or(if entry.is_dir() { 0o040755 } else { 0o100644 });
        let kind = mode & 0o170000;
        if kind != 0 && kind != 0o040000 && kind != 0o100000 {
            return Err(ArchiveError("ZIP contiene symlink o tipo especial".into()));
        }
        if entry.size() > FILE_LIMIT {
            return Err(ArchiveError("archivo descomprimido supera 64 MiB".into()));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| ArchiveError("tamaño ZIP desbordado".into()))?;
        if total > TOTAL_LIMIT {
            return Err(ArchiveError("ZIP descomprimido supera 250 MiB".into()));
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(io_error)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        let mut output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(io_error)?;
        let copied = io::copy(&mut entry.by_ref().take(FILE_LIMIT + 1), &mut output_file)
            .map_err(io_error)?;
        if copied != entry.size() {
            return Err(ArchiveError("tamaño descomprimido inconsistente".into()));
        }
        preserve_permissions(&output, mode)?;
    }
    if !destination
        .join(expected_root)
        .join("plugin.toml")
        .is_file()
    {
        return Err(ArchiveError("falta plugin.toml en la raíz única".into()));
    }
    Ok(())
}

fn portable_path(raw: &str, expected_root: &str) -> Result<PathBuf, ArchiveError> {
    if raw.is_empty() || raw.contains(['\0', '\\', ':']) || raw.starts_with('/') {
        return Err(ArchiveError("path ZIP no portable".into()));
    }
    let path = Path::new(raw.trim_end_matches('/'));
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
        || path
            .components()
            .next()
            .and_then(|part| part.as_os_str().to_str())
            != Some(expected_root)
    {
        return Err(ArchiveError("path ZIP escapa o tiene otra raíz".into()));
    }
    Ok(path.into())
}

#[cfg(unix)]
fn preserve_permissions(path: &Path, mode: u32) -> Result<(), ArchiveError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777)).map_err(io_error)
}
#[cfg(not(unix))]
fn preserve_permissions(_: &Path, _: u32) -> Result<(), ArchiveError> {
    Ok(())
}
fn io_error(error: io::Error) -> ArchiveError {
    ArchiveError(format!("staging: {error}"))
}
fn zip_error(error: zip::result::ZipError) -> ArchiveError {
    ArchiveError(format!("ZIP inválido: {error}"))
}
impl fmt::Display for ArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for ArchiveError {}
