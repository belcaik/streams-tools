mod config;

use std::{io, net::SocketAddr, path::PathBuf};

use config::{KickConfig, YoutubeConfig};
use streams_tools_adapters::{kick::KickAdapter, twitch::TwitchAdapter, youtube::YoutubeAdapter};
use streams_tools_core::Bus;
use streams_tools_plugins::{Registry, Supervisor, SupervisorOptions};
use streams_tools_server::PluginOverlay;

#[tokio::main]
async fn main() -> io::Result<()> {
    if let Some("--version" | "-V") = std::env::args().nth(1).as_deref() {
        println!("streams-tools {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config_path = std::env::var_os("STREAMS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./config.toml"));
    let config_path = std::path::absolute(config_path)?;
    let config = config::load(&config_path)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let data_root = std::env::var_os("STREAMS_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./data"));
    let data_root = std::path::absolute(data_root)?;

    let addr: SocketAddr = std::env::var("STREAMS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7777".into())
        .parse()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid STREAMS_ADDR: {error}"),
            )
        })?;
    let plugins_dir = std::env::var_os("STREAMS_PLUGINS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./plugins"));
    let plugins_dir = std::path::absolute(plugins_dir)?;
    let registry = Registry::discover(&plugins_dir).map_err(|error| {
        io::Error::new(error.kind(), format!("{}: {error}", plugins_dir.display()))
    })?;
    for error in registry.errors() {
        eprintln!("[plugin] {error}");
    }

    let styles_root = std::env::var_os("STREAMS_STYLES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./styles"));
    let plugins = registry.into_plugins();
    let mut overlays = Vec::new();
    for plugin in &plugins {
        if let Some(root) = plugin.overlay_dir() {
            overlays.push(PluginOverlay {
                name: plugin.manifest.name.clone(),
                root,
                config: plugin.manifest.config_json(),
                custom_css: styles_root.join(format!("{}.css", plugin.manifest.name)),
            });
        }
    }
    let workers = plugins.iter().filter(|plugin| plugin.has_worker()).count();
    let plugin_count = plugins.len();

    let bus = Bus::new(1024).expect("server bus capacity must be non-zero");
    let options = SupervisorOptions {
        data_root: data_root.clone(),
        ..SupervisorOptions::default()
    };
    let supervisor = Supervisor::start(bus.clone(), plugins, options);
    let twitch = match config.twitch {
        Some(config) => match TwitchAdapter::start(bus.clone(), config.into_settings(&data_root)) {
            Ok(adapter) => Some(adapter),
            Err(error) => {
                supervisor.shutdown().await;
                return Err(io::Error::other(error));
            }
        },
        None => None,
    };
    let kick = match config.kick.map(KickConfig::into_settings) {
        Some(settings) => match KickAdapter::start(bus.clone(), settings) {
            Ok(adapter) => Some(adapter),
            Err(error) => {
                eprintln!("Kick: no se pudo iniciar: {error}");
                None
            }
        },
        None => None,
    };
    let youtube = match config.youtube.map(YoutubeConfig::into_settings) {
        Some(settings) => match YoutubeAdapter::start(bus.clone(), settings) {
            Ok(adapter) => Some(adapter),
            Err(error) => {
                eprintln!("YouTube: no se pudo iniciar: {error}");
                None
            }
        },
        None => None,
    };
    println!(
        "streams-tools listening on http://{addr}; {} plugins, {workers} workers, {} overlays; twitch {}; kick {}; youtube {}",
        plugin_count,
        overlays.len(),
        if twitch.is_some() {
            "enabled"
        } else {
            "disabled"
        },
        if kick.is_some() {
            "enabled"
        } else {
            "disabled"
        },
        if youtube.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    let result = streams_tools_server::serve(bus, addr, overlays, async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("failed to listen for Ctrl+C: {error}");
        }
    })
    .await;
    if let Some(adapter) = youtube {
        adapter.shutdown().await;
    }
    if let Some(adapter) = kick {
        adapter.shutdown().await;
    }
    if let Some(adapter) = twitch {
        adapter.shutdown().await;
    }
    supervisor.shutdown().await;
    result
}
