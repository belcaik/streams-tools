mod command;
mod config;
mod paths;

use std::{io, net::SocketAddr};

use config::{KickConfig, YoutubeConfig};
use streams_tools_adapters::{kick::KickAdapter, twitch::TwitchAdapter, youtube::YoutubeAdapter};
use streams_tools_core::Bus;
use streams_tools_plugins::{
    AlertRegistry, Registry, Supervisor, SupervisorOptions,
    install::{InstallOptions, Installer},
};
use streams_tools_server::{AlertAssets, AlertPackAssets, PluginOverlay};

#[tokio::main]
async fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match command::parse(&args) {
        Ok(command::Command::Version) => {
            println!("streams-tools {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(command::Command::Help | command::Command::Plugin(command::PluginCommand::Help)) => {
            println!("{}", command::USAGE);
            return Ok(());
        }
        Ok(command::Command::Plugin(command)) => {
            if !run_plugin(command, &paths::Paths::resolve()?).await? {
                std::process::exit(1);
            }
            return Ok(());
        }
        Ok(command::Command::Server) => {}
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
    let paths = paths::Paths::resolve()?;

    let config = config::load(&paths.config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let data_root = paths.data.clone();

    let addr: SocketAddr = std::env::var("STREAMS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7777".into())
        .parse()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid STREAMS_ADDR: {error}"),
            )
        })?;
    let installer = Installer::new(InstallOptions {
        plugins_root: paths.plugins.clone(),
        target: env!("STREAMS_TARGET_TRIPLE").into(),
    })
    .map_err(io::Error::other)?
    .with_event_handler(|event| eprintln!("{event}"));
    for result in installer
        .ensure_missing(config.plugins.install.iter().map(String::as_str))
        .await
    {
        print_install_result(result);
    }
    let registry = Registry::discover_with_config(&paths.plugins, &config.plugins.config).map_err(
        |error| {
            io::Error::new(
                error.kind(),
                format!("{}: {error}", paths.plugins.display()),
            )
        },
    )?;
    for error in registry.errors() {
        eprintln!("[plugin] {error}");
    }
    for warning in registry.warnings() {
        eprintln!("[plugin] {warning}");
    }
    let alert_registry = AlertRegistry::discover(&paths.alerts).map_err(|error| {
        io::Error::new(error.kind(), format!("{}: {error}", paths.alerts.display()))
    })?;
    for error in alert_registry.errors() {
        eprintln!(
            "[alert] {}: campo {}: {}",
            paths.alerts.join(&error.file).display(),
            error.field,
            error.reason
        );
    }
    let alert_pack_count = alert_registry.index().packs.len();
    let valid_alert_count = alert_registry.valid_alert_count();
    let alert_error_count = alert_registry.errors().len();
    let alert_assets = AlertAssets {
        index: alert_registry.index_json(),
        packs: alert_registry
            .asset_roots()
            .map(|(name, root)| AlertPackAssets {
                name: name.into(),
                root: root.into(),
            })
            .collect(),
    };

    let styles_root = paths.styles;
    let plugins = registry.into_plugins();
    let mut overlays = Vec::new();
    for plugin in &plugins {
        if let Some(root) = plugin.overlay_dir() {
            overlays.push(PluginOverlay {
                name: plugin.manifest.name.clone(),
                root,
                config: plugin.effective_config.clone(),
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
        concat!(
            "streams-tools listening on http://{}; {} plugins, {} workers, ",
            "{} overlays; twitch {}; kick {}; youtube {}; ",
            "{} alert packs, {} valid alerts, {} alert errors"
        ),
        addr,
        plugin_count,
        workers,
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
        },
        alert_pack_count,
        valid_alert_count,
        alert_error_count
    );
    let result = streams_tools_server::serve(bus, addr, overlays, alert_assets, async {
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

async fn run_plugin(command: command::PluginCommand, paths: &paths::Paths) -> io::Result<bool> {
    let installer = Installer::new(InstallOptions {
        plugins_root: paths.plugins.clone(),
        target: env!("STREAMS_TARGET_TRIPLE").into(),
    })
    .map_err(io::Error::other)?
    .with_event_handler(|event| eprintln!("{event}"));
    let success = match command {
        command::PluginCommand::Add(source) => print_install_result(installer.add(&source).await),
        command::PluginCommand::Update(name) => installer
            .update(name.as_deref())
            .await
            .into_iter()
            .all(print_install_result),
        command::PluginCommand::Remove(name) => print_install_result(installer.remove(&name)),
        command::PluginCommand::List => {
            for plugin in installer.list().map_err(io::Error::other)? {
                println!(
                    "{}\t{}\t{}\t{}",
                    plugin.name,
                    plugin.version.as_deref().unwrap_or("-"),
                    plugin.state,
                    plugin.tag.as_deref().unwrap_or("-")
                );
            }
            true
        }
        command::PluginCommand::Help => true,
    };
    Ok(success)
}

fn print_install_result(
    result: Result<
        streams_tools_plugins::install::InstallOutcome,
        streams_tools_plugins::install::InstallError,
    >,
) -> bool {
    match result {
        Ok(outcome) => {
            if let Some(warning) = outcome.warning {
                eprintln!("{warning}");
            }
            println!("{}: {}", outcome.name, outcome.detail);
            true
        }
        Err(error) => {
            eprintln!("[plugin] {error}");
            false
        }
    }
}
