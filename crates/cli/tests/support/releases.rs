use super::http::Server;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use zip::{ZipWriter, write::FileOptions};
pub const TARGET: &str = "x86_64-unknown-linux-gnu";

pub fn zip(root: &str, version: &str, worker: bool, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let worker_toml = if worker {
        "[worker]\ncommand=['bin/worker']\n"
    } else {
        ""
    };
    zip.start_file(format!("{root}/plugin.toml"), FileOptions::default())
        .unwrap();
    write!(
        zip,
        "name='{root}'\nversion='{version}'\n[permissions]\nlistens=[]\nemits=[]\n{worker_toml}"
    )
    .unwrap();
    if worker {
        zip.start_file(
            format!("{root}/bin/worker"),
            FileOptions::default().unix_permissions(0o755),
        )
        .unwrap();
        zip.write_all(b"must never execute").unwrap();
    }
    for (name, body) in extra {
        zip.start_file(*name, FileOptions::default()).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

pub fn raw_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in entries {
        zip.start_file(*name, FileOptions::default()).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

pub fn symlink_zip() -> Vec<u8> {
    let mut bytes = raw_zip(&[
        (
            "demo/plugin.toml",
            b"name='demo'\nversion='1.0.0'\n[permissions]\nlistens=[]\nemits=[]",
        ),
        ("demo/link", b"target"),
    ]);
    let positions: Vec<_> = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(index, bytes)| (bytes == b"PK\x01\x02").then_some(index))
        .collect();
    let central = *positions.last().unwrap();
    bytes[central + 5] = 3;
    bytes[central + 38..central + 42].copy_from_slice(&(0o120777_u32 << 16).to_le_bytes());
    bytes
}

pub fn many_entries() -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..2049 {
        zip.start_file(format!("demo/{index}"), FileOptions::default())
            .unwrap();
    }
    zip.finish().unwrap().into_inner()
}

pub fn publish(
    server: &Server,
    owner: &str,
    repo: &str,
    tag: &str,
    asset: &str,
    bytes: Vec<u8>,
    valid_checksum: bool,
) {
    let asset_path = format!("/assets/{repo}/{asset}");
    let checksum_path = format!("/assets/{repo}/SHA256SUMS");
    let sha = if valid_checksum {
        format!("{:x}", Sha256::digest(&bytes))
    } else {
        "0".repeat(64)
    };
    let release = format!(
        r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"assets":[{{"name":"{asset}","browser_download_url":"{}{asset_path}"}},{{"name":"SHA256SUMS","browser_download_url":"{}{checksum_path}"}}]}}"#,
        server.base(),
        server.base()
    );
    server.route(
        format!("/repos/{owner}/{repo}/releases/latest"),
        200,
        release.clone(),
    );
    server.route(
        format!("/repos/{owner}/{repo}/releases/tags/{tag}"),
        200,
        release,
    );
    server.route(asset_path, 200, bytes);
    server.route(checksum_path, 200, format!("{sha}  {asset}\n"));
}
