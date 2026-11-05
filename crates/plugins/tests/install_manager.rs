use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};
use streams_tools_plugins::install::{InstallOptions, Installer};
use zip::{ZipWriter, write::FileOptions};

static ID: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-manager-{}-{}",
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

fn zip() -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("demo/plugin.toml", FileOptions::default())
        .unwrap();
    zip.write_all(b"name='demo'\nversion='1.0.0'\n[permissions]\nlistens=[]\nemits=[]")
        .unwrap();
    zip.start_file("demo/overlay/index.html", FileOptions::default())
        .unwrap();
    zip.write_all(b"safe").unwrap();
    zip.finish().unwrap().into_inner()
}

fn server(requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let asset_url = format!("{base}/assets/demo-any.zip");
    let checksum_url = format!("{base}/assets/SHA256SUMS");
    let zip = zip();
    let checksum = format!("{:x}  demo-any.zip\n", Sha256::digest(&zip));
    let release = format!(
        r#"{{"tag_name":"v1.0.0","draft":false,"prerelease":false,"assets":[{{"name":"demo-any.zip","browser_download_url":"{asset_url}"}},{{"name":"SHA256SUMS","browser_download_url":"{checksum_url}"}}]}}"#
    );
    let handle = thread::spawn(move || {
        for _ in 0..requests {
            let (mut socket, _) = listener.accept().unwrap();
            let mut raw = [0; 4096];
            let size = socket.read(&mut raw).unwrap();
            let path = String::from_utf8_lossy(&raw[..size])
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_owned();
            let body: &[u8] = if path.ends_with("SHA256SUMS") {
                checksum.as_bytes()
            } else if path.ends_with(".zip") {
                &zip
            } else {
                release.as_bytes()
            };
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(body).unwrap();
        }
    });
    (base, handle)
}

#[tokio::test]
#[rustfmt::skip]
async fn add_update_list_remove_and_ensure_missing_are_managed() {
    let temp = Temp::new(); let (base, handle) = server(6);
    let installer = Installer::for_local_tests(InstallOptions { plugins_root: temp.0.join("plugins"), target: "x86_64-unknown-linux-gnu".into() }, &base).unwrap();
    let added = installer.add("https://github.com/acme/demo").await.unwrap();
    assert!(added.changed && added.warning.is_none());
    assert_eq!(installer.list().unwrap()[0].state, "managed");
    let updated = installer.update(Some("demo")).await.pop().unwrap().unwrap();
    assert!(!updated.changed && updated.detail.contains("v1.0.0"));
    assert!(!temp.0.join("executed").exists());
    installer.remove("demo").unwrap(); assert!(installer.list().unwrap().is_empty()); handle.join().unwrap();
}

#[tokio::test]
async fn update_unknown_and_unmanaged_remove_are_explicit_errors() {
    let temp = Temp::new();
    let installer = Installer::new(InstallOptions {
        plugins_root: temp.0.join("plugins"),
        target: "x86_64-unknown-linux-gnu".into(),
    })
    .unwrap();
    assert!(installer.update(Some("missing")).await[0].is_err());
    fs::create_dir_all(temp.0.join("plugins/local")).unwrap();
    fs::write(
        temp.0.join("plugins/local/plugin.toml"),
        "name='local'\nversion='1'\n[permissions]\nlistens=[]\nemits=[]",
    )
    .unwrap();
    assert_eq!(installer.list().unwrap()[0].state, "unmanaged");
    assert!(installer.remove("local").is_err());
}

#[tokio::test]
async fn add_emits_the_third_party_warning_before_publishing() {
    let temp = Temp::new();
    let (base, handle) = server(3);
    let plugin = temp.0.join("plugins/demo");
    let observed = Arc::new(Mutex::new(None));
    let callback_observed = observed.clone();
    let installer = Installer::for_local_tests(
        InstallOptions {
            plugins_root: temp.0.join("plugins"),
            target: "x86_64-unknown-linux-gnu".into(),
        },
        &base,
    )
    .unwrap()
    .with_event_handler(move |event| {
        *callback_observed.lock().unwrap() = Some((event.to_string(), plugin.exists()));
    });

    let outcome = installer.add("https://github.com/acme/demo").await.unwrap();

    assert_eq!(
        observed.lock().unwrap().as_ref(),
        Some(&(
            "ADVERTENCIA: instalar acme/demo permite que streams-tools ejecute su worker y sirva su overlay; instala sólo código de confianza.".into(),
            false,
        ))
    );
    assert!(outcome.warning.is_none());
    handle.join().unwrap();
}

#[cfg(unix)]
#[test]
fn list_cleans_only_stale_installer_entries_without_following_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = Temp::new();
    let plugins = temp.0.join("plugins");
    let tmp = plugins.join(".streams-tools/tmp");
    fs::create_dir_all(&tmp).unwrap();
    let stale_pid = std::process::id().saturating_add(1);
    fs::create_dir(tmp.join(format!("{stale_pid}-1"))).unwrap();
    fs::create_dir(tmp.join(format!("{stale_pid}-2-backup-demo-plugin"))).unwrap();
    fs::create_dir(tmp.join(format!("{}-3", std::process::id()))).unwrap();
    fs::create_dir(tmp.join("unrelated")).unwrap();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "safe").unwrap();
    symlink(&outside, tmp.join(format!("{stale_pid}-4"))).unwrap();
    let installer = Installer::new(InstallOptions {
        plugins_root: plugins,
        target: "x86_64-unknown-linux-gnu".into(),
    })
    .unwrap();

    installer.list().unwrap();

    assert!(!tmp.join(format!("{stale_pid}-1")).exists());
    assert!(
        !tmp.join(format!("{stale_pid}-2-backup-demo-plugin"))
            .exists()
    );
    assert!(tmp.join(format!("{}-3", std::process::id())).exists());
    assert!(tmp.join("unrelated").exists());
    assert!(outside.join("keep").is_file());
}
