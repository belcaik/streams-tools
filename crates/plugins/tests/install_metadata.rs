use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use streams_tools_plugins::install::{
    DownloadedPackage, FsTransaction, InstalledMetadata, MetadataRecord, MetadataStore, Staging,
    validate_package,
};
use zip::{ZipWriter, write::FileOptions};

static ID: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-metadata-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[rustfmt::skip]
fn metadata(version: &str) -> InstalledMetadata { InstalledMetadata {
    schema: 1, name: "demo".into(), version: version.into(), source: "https://github.com/acme/demo".into(),
    requested: "latest".into(), tag: format!("v{version}"), asset: "demo-any.zip".into(), target: "any".into(), sha256: "a".repeat(64),
} }

#[rustfmt::skip]
fn package(root: &std::path::Path, version: &str) -> streams_tools_plugins::install::ValidatedPackage {
    let staging = Staging::create(root).unwrap(); let path = staging.archive_path();
    let mut zip = ZipWriter::new(fs::File::create(&path).unwrap());
    zip.start_file("demo/plugin.toml", FileOptions::default()).unwrap();
    write!(zip, "name='demo'\nversion='{version}'\n[permissions]\nlistens=[]\nemits=[]").unwrap(); zip.finish().unwrap();
    validate_package(staging, DownloadedPackage { path, sha256: "a".repeat(64) }, "demo-any.zip", &format!("v{version}"), "x86_64-unknown-linux-gnu").unwrap()
}

#[test]
#[rustfmt::skip]
fn metadata_roundtrip_transactions_and_unmanaged_protection() {
    let temp = Temp::new(); let plugins = temp.0.join("plugins"); let transaction = FsTransaction::new(&plugins).unwrap();
    let first = package(&plugins, "1.0.0"); transaction.publish(&first, &metadata("1.0.0")).unwrap();
    assert_eq!(MetadataStore::new(&plugins).read("demo").unwrap(), Some(metadata("1.0.0")));
    let second = package(&plugins, "1.1.0"); let _ = transaction.update(&second, &metadata("1.1.0")).unwrap();
    assert!(fs::read_to_string(plugins.join("demo/plugin.toml")).unwrap().contains("1.1.0"));
    transaction.remove("demo").unwrap(); assert!(!plugins.join("demo").exists());
    fs::create_dir_all(plugins.join("demo")).unwrap();
    assert!(transaction.remove("demo").unwrap_err().to_string().contains("no administrado"));
    let store = MetadataStore::new(&plugins); fs::create_dir_all(plugins.join(".streams-tools/installed")).unwrap();
    fs::write(store.path("broken").unwrap(), "schema = 9").unwrap();
    assert!(matches!(store.list().unwrap().as_slice(), [MetadataRecord::Broken { name, .. }] if name == "broken"));
}

#[test]
#[rustfmt::skip]
fn rejects_corrupt_schema_name_sha_and_target() {
    let mut value = metadata("1.0.0"); value.schema = 2; assert!(value.validate(None).is_err());
    value = metadata("1.0.0"); value.name = "../demo".into(); assert!(value.validate(None).is_err());
    value = metadata("1.0.0"); value.sha256 = "A".repeat(64); assert!(value.validate(None).is_err());
    value = metadata("1.0.0"); value.target = "other".into(); assert!(value.validate(None).is_err());
}

#[test]
fn remove_moves_a_managed_plugin_when_staging_did_not_exist() {
    let temp = Temp::new();
    let plugins = temp.0.join("plugins");
    let transaction = FsTransaction::new(&plugins).unwrap();
    fs::create_dir(plugins.join("demo")).unwrap();
    let store = MetadataStore::new(&plugins);
    fs::create_dir_all(plugins.join(".streams-tools/installed")).unwrap();
    fs::write(
        store.path("demo").unwrap(),
        toml::to_string(&metadata("1.0.0")).unwrap(),
    )
    .unwrap();

    transaction.remove("demo").unwrap();

    assert!(!plugins.join("demo").exists());
}

#[cfg(unix)]
#[test]
fn update_reports_cleanup_failure_after_committing_new_plugin() {
    use std::os::unix::fs::PermissionsExt;

    let temp = Temp::new();
    let plugins = temp.0.join("plugins");
    let transaction = FsTransaction::new(&plugins).unwrap();
    let first = package(&plugins, "1.0.0");
    transaction.publish(&first, &metadata("1.0.0")).unwrap();
    let locked = plugins.join("demo/locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("file"), "old").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        // Con privilegios (root en contenedores CI) los permisos no bloquean el borrado.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    let second = package(&plugins, "1.1.0");

    let result = transaction.update(&second, &metadata("1.1.0"));

    for entry in fs::read_dir(plugins.join(".streams-tools/tmp")).unwrap() {
        let locked_backup = entry.unwrap().path().join("locked");
        if locked_backup.exists() {
            fs::set_permissions(locked_backup, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    assert!(
        result
            .unwrap()
            .is_some_and(|warning| warning.contains("limpieza pendiente"))
    );
    assert!(
        fs::read_to_string(plugins.join("demo/plugin.toml"))
            .unwrap()
            .contains("1.1.0")
    );
}
