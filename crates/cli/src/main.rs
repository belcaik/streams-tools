use std::{io, net::SocketAddr};

use streams_tools_core::Bus;

#[tokio::main]
async fn main() -> io::Result<()> {
    if let Some("--version" | "-V") = std::env::args().nth(1).as_deref() {
        println!("streams-tools {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let addr: SocketAddr = std::env::var("STREAMS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7777".into())
        .parse()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid STREAMS_ADDR: {error}"),
            )
        })?;
    let bus = Bus::new(1024).expect("server bus capacity must be non-zero");

    println!("streams-tools listening on http://{addr}");
    streams_tools_server::serve(bus, addr, Vec::new(), async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("failed to listen for Ctrl+C: {error}");
        }
    })
    .await
}
