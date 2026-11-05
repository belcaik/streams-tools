use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};
use streams_tools_plugins::install::{
    Asset, GithubClient, PluginSource, Release, RequestedRelease,
};

fn source(value: &str) -> PluginSource {
    PluginSource::parse(value).unwrap()
}

fn server(status: &str, headers: &str, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (status, headers, body) = (status.to_owned(), headers.to_owned(), body.to_owned());
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let read = socket.read(&mut request).unwrap();
        assert!(
            String::from_utf8_lossy(&request[..read]).starts_with("GET /repos/acme/demo/releases/")
        );
        let length = if headers.to_ascii_lowercase().contains("content-length:") {
            String::new()
        } else {
            format!("Content-Length: {}\r\n", body.len())
        };
        write!(
            socket,
            "HTTP/1.1 {status}\r\n{headers}{length}Connection: close\r\n\r\n{body}"
        )
        .unwrap();
    });
    format!("http://{address}")
}

#[test]
#[rustfmt::skip]
fn parses_and_normalizes_strict_github_sources() {
    for (input, expected) in [
        ("https://github.com/acme/demo", "https://github.com/acme/demo"),
        ("https://github.com/acme/demo.git/", "https://github.com/acme/demo"),
        ("https://github.com/acme/demo@v1.2.3-rc.1", "https://github.com/acme/demo@v1.2.3-rc.1"),
    ] { assert_eq!(source(input).to_string(), expected); }
    assert!(matches!(source("https://github.com/a/b@v1.0.0").requested(), RequestedRelease::Tag(tag) if tag == "v1.0.0"));
    for invalid in [
        "http://github.com/a/b", "https://gitlab.com/a/b", "https://user@github.com/a/b",
        "https://github.com/a/b/c", "https://github.com/./b", "https://github.com/a/..",
        "https://github.com/a/b?x=1", "https://github.com/a/b#x", "git@github.com:a/b.git",
        "https://github.com/a/b@1.0.0", "https://github.com/a/b@vlatest",
    ] { assert!(PluginSource::parse(invalid).is_err(), "{invalid}"); }
}

#[test]
#[rustfmt::skip]
fn selects_exact_target_then_any_and_rejects_foreign_or_ambiguous_assets() {
    let source = source("https://github.com/acme/demo@v1.0.0");
    let asset = |name: &str| Asset { name: name.into(), url: format!("https://github.com/acme/demo/releases/download/v1.0.0/{name}") };
    let release = Release { tag: "v1.0.0".into(), draft: false, prerelease: false,
        assets: vec![asset("demo-any.zip"), asset("demo-x86_64-unknown-linux-gnu.zip")] };
    assert_eq!(release.select_asset(&source, "x86_64-unknown-linux-gnu").unwrap().name, "demo-x86_64-unknown-linux-gnu.zip");
    assert_eq!(release.select_asset(&source, "aarch64-apple-darwin").unwrap().name, "demo-any.zip");
    let mut ambiguous = release.clone();
    ambiguous.assets.push(asset("other-any.zip"));
    assert!(ambiguous.select_asset(&source, "unknown").is_err());
    let mut foreign = release;
    foreign.assets[0].url = "https://example.com/demo-any.zip".into();
    assert!(foreign.select_asset(&source, "unknown").is_err());
}

#[tokio::test]
#[rustfmt::skip]
async fn fetches_latest_and_tag_and_reports_api_failures() {
    let json = r#"{"tag_name":"v1.0.0","draft":false,"prerelease":false,"assets":[]}"#;
    for requested in ["https://github.com/acme/demo", "https://github.com/acme/demo@v1.0.0"] {
        let base = server("200 OK", "Content-Type: application/json\r\n", json);
        let release = GithubClient::with_api_base(&base).unwrap().release(&source(requested)).await.unwrap();
        assert_eq!(release.tag, "v1.0.0");
    }
    let base = server("403 Forbidden", "X-RateLimit-Remaining: 0\r\n", "{}");
    let error = GithubClient::with_api_base(&base).unwrap().release(&source("https://github.com/acme/demo")).await.unwrap_err();
    assert!(error.to_string().contains("tasa"));
    let base = server("200 OK", "Content-Length: 1048577\r\n", "{}");
    let error = GithubClient::with_api_base(&base).unwrap().release(&source("https://github.com/acme/demo")).await.unwrap_err();
    assert!(error.to_string().contains("1 MiB"));
}
