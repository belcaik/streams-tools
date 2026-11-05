use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
};
use streams_tools_plugins::install::{
    DownloadedPackage, Downloader, Portability, Staging, validate_package,
};
use zip::{ZipWriter, write::FileOptions};

static ID: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "streams-archive-{}-{}",
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

fn server(zip: Vec<u8>, checksum: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for body in [checksum.into_bytes(), zip] {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        }
    });
    format!("http://{address}")
}

#[tokio::test]
async fn downloads_streams_and_verifies_checksum_before_use() {
    let temp = Temp::new();
    let zip = b"fake zip".to_vec();
    let sha = format!("{:x}", Sha256::digest(&zip));
    let base = server(zip.clone(), format!("{sha}  demo-any.zip\n"));
    let result = Downloader::for_local_tests()
        .unwrap()
        .fetch_verified(
            &format!("{base}/SHA256SUMS"),
            &format!("{base}/demo-any.zip"),
            "demo-any.zip",
            &temp.0.join("package.zip"),
        )
        .await
        .unwrap();
    assert_eq!(result.sha256, sha);
    assert_eq!(fs::read(result.path).unwrap(), zip);
    let base = server(zip, format!("{}  demo-any.zip\n", "0".repeat(64)));
    assert!(
        Downloader::for_local_tests()
            .unwrap()
            .fetch_verified(
                &format!("{base}/SHA256SUMS"),
                &format!("{base}/demo-any.zip"),
                "demo-any.zip",
                &temp.0.join("bad.zip")
            )
            .await
            .is_err()
    );
    assert!(!temp.0.join("bad.zip").exists());
}

fn package(temp: &Temp, asset: &str, entries: &[(&str, &str)]) -> (Staging, DownloadedPackage) {
    let plugins = temp.0.join("plugins");
    let staging = Staging::create(&plugins).unwrap();
    let path = staging.archive_path();
    let mut zip = ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, body) in entries {
        zip.start_file(
            *name,
            FileOptions::default().unix_permissions(if name.contains("/bin/") {
                0o755
            } else {
                0o644
            }),
        )
        .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    (
        staging,
        DownloadedPackage {
            path,
            sha256: format!("{}-{asset}", "a".repeat(64)),
        },
    )
}

fn manifest(name: &str, version: &str, worker: bool) -> String {
    let worker = if worker {
        "[worker]\ncommand = ['bin/worker']\n"
    } else {
        ""
    };
    format!(
        "name = '{name}'\nversion = '{version}'\n[permissions]\nlistens = []\nemits = []\n{worker}"
    )
}

#[test]
fn extracts_valid_target_and_any_packages_without_executing_content() {
    let temp = Temp::new();
    let body = manifest("demo", "1.2.3", false);
    let (staging, downloaded) = package(
        &temp,
        "demo-any.zip",
        &[
            ("demo/plugin.toml", body.as_str()),
            ("demo/overlay/index.html", "ok"),
        ],
    );
    let validated = validate_package(
        staging,
        downloaded,
        "demo-any.zip",
        "v1.2.3",
        "x86_64-unknown-linux-gnu",
    )
    .unwrap();
    assert_eq!(validated.portability, Portability::Any);
    assert!(validated.plugin_dir().join("overlay/index.html").is_file());
    let body = manifest("demo", "1.2.3", true);
    let (staging, downloaded) = package(
        &temp,
        "demo-x86_64-unknown-linux-gnu.zip",
        &[
            ("demo/plugin.toml", body.as_str()),
            ("demo/bin/worker", "must not run"),
        ],
    );
    let validated = validate_package(
        staging,
        downloaded,
        "demo-x86_64-unknown-linux-gnu.zip",
        "v1.2.3",
        "x86_64-unknown-linux-gnu",
    )
    .unwrap();
    assert_eq!(validated.portability, Portability::Target);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(validated.plugin_dir().join("bin/worker"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}

#[test]
fn rejects_traversal_multiple_roots_version_mismatch_and_worker_any() {
    let temp = Temp::new();
    for (asset, tag, entries) in [
        ("demo-any.zip", "v1.0.0", vec![("../escape", "x")]),
        ("demo-any.zip", "v1.0.0", vec![("other/plugin.toml", "x")]),
        (
            "demo-any.zip",
            "v2.0.0",
            vec![(
                "demo/plugin.toml",
                "name='demo'\nversion='1.0.0'\n[permissions]\nlistens=[]\nemits=[]",
            )],
        ),
        (
            "demo-any.zip",
            "v1.0.0",
            vec![(
                "demo/plugin.toml",
                "name='demo'\nversion='1.0.0'\n[permissions]\nlistens=[]\nemits=[]\n[worker]\ncommand=['bad']",
            )],
        ),
    ] {
        let (staging, downloaded) = package(&temp, asset, &entries);
        assert!(
            validate_package(staging, downloaded, asset, tag, "x86_64-unknown-linux-gnu").is_err()
        );
    }
}
