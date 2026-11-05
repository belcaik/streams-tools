use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
};
use streams_tools_plugins::install::Downloader;

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
