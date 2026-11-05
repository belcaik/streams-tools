#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Server,
    Version,
    Help,
    Plugin(PluginCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginCommand {
    Add(String),
    Update(Option<String>),
    Remove(String),
    List,
    Help,
}

pub(crate) const USAGE: &str = "Uso:\n  streams-tools\n  streams-tools --version\n  streams-tools plugin add <url>\n  streams-tools plugin update [nombre]\n  streams-tools plugin remove <nombre>\n  streams-tools plugin list";

pub(crate) fn parse(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Server),
        [flag] if flag == "--version" || flag == "-V" => Ok(Command::Version),
        [flag] if flag == "--help" || flag == "-h" => Ok(Command::Help),
        [plugin, flag] if plugin == "plugin" && (flag == "--help" || flag == "-h") => {
            Ok(Command::Plugin(PluginCommand::Help))
        }
        [plugin, action, source] if plugin == "plugin" && action == "add" => {
            Ok(Command::Plugin(PluginCommand::Add(source.clone())))
        }
        [plugin, action] if plugin == "plugin" && action == "update" => {
            Ok(Command::Plugin(PluginCommand::Update(None)))
        }
        [plugin, action, name] if plugin == "plugin" && action == "update" => {
            Ok(Command::Plugin(PluginCommand::Update(Some(name.clone()))))
        }
        [plugin, action, name] if plugin == "plugin" && action == "remove" => {
            Ok(Command::Plugin(PluginCommand::Remove(name.clone())))
        }
        [plugin, action] if plugin == "plugin" && action == "list" => {
            Ok(Command::Plugin(PluginCommand::List))
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn parses_exact_plugin_grammar() {
        assert_eq!(
            parse(&args(&["plugin", "add", "url"])).unwrap(),
            Command::Plugin(PluginCommand::Add("url".into()))
        );
        assert_eq!(
            parse(&args(&["plugin", "update"])).unwrap(),
            Command::Plugin(PluginCommand::Update(None))
        );
        assert_eq!(
            parse(&args(&["plugin", "update", "demo"])).unwrap(),
            Command::Plugin(PluginCommand::Update(Some("demo".into())))
        );
        assert_eq!(
            parse(&args(&["plugin", "remove", "demo"])).unwrap(),
            Command::Plugin(PluginCommand::Remove("demo".into()))
        );
        assert_eq!(
            parse(&args(&["plugin", "list"])).unwrap(),
            Command::Plugin(PluginCommand::List)
        );
        for invalid in [
            &["plugin"][..],
            &["plugin", "add"],
            &["plugin", "list", "extra"],
            &["unknown"],
        ] {
            assert!(parse(&args(invalid)).is_err());
        }
    }
}
