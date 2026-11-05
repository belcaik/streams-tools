use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Paths {
    pub config: PathBuf,
    pub plugins: PathBuf,
    pub data: PathBuf,
    pub alerts: PathBuf,
    pub styles: PathBuf,
}

impl Paths {
    pub fn resolve() -> io::Result<Self> {
        let cwd = std::env::current_dir()?;
        let exe = std::env::current_exe()?;
        Self::resolve_with(&cwd, &exe, cfg!(debug_assertions), |name| {
            std::env::var_os(name)
        })
    }

    fn resolve_with(
        cwd: &Path,
        exe: &Path,
        debug: bool,
        env: impl Fn(&str) -> Option<OsString>,
    ) -> io::Result<Self> {
        let home = match env("STREAMS_HOME") {
            Some(path) => absolute(cwd, Path::new(&path)),
            None if debug => cwd.to_owned(),
            None => exe
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_owned)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "current_exe no tiene directorio padre",
                    )
                })?,
        };
        let resolve = |variable: &str, default: &str| {
            env(variable).map_or_else(
                || home.join(default),
                |path| absolute(cwd, Path::new(&path)),
            )
        };
        Ok(Self {
            config: resolve("STREAMS_CONFIG", "config.toml"),
            plugins: resolve("STREAMS_PLUGINS_DIR", "plugins"),
            data: resolve("STREAMS_DATA_DIR", "data"),
            alerts: resolve("STREAMS_ALERTS_DIR", "alerts"),
            styles: resolve("STREAMS_STYLES_DIR", "styles"),
        })
    }
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.into()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn resolve(debug: bool, values: &[(&str, &str)]) -> io::Result<Paths> {
        let env: BTreeMap<_, _> = values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), OsString::from(value)))
            .collect();
        Paths::resolve_with(
            Path::new("/work/cwd"),
            Path::new("/opt/streams/streams-tools"),
            debug,
            |name| env.get(name).cloned(),
        )
    }
    #[test]
    fn debug_uses_cwd_and_release_uses_executable_parent() {
        assert_eq!(
            resolve(true, &[]).unwrap().plugins,
            Path::new("/work/cwd/plugins")
        );
        assert_eq!(
            resolve(false, &[]).unwrap().plugins,
            Path::new("/opt/streams/plugins")
        );
        assert!(
            Paths::resolve_with(Path::new("/cwd"), Path::new("binary"), false, |_| None).is_err()
        );
    }
    #[test]
    fn home_and_each_override_obey_absolute_and_relative_rules() {
        let paths = resolve(
            false,
            &[
                ("STREAMS_HOME", "portable home"),
                ("STREAMS_CONFIG", "cfg/custom.toml"),
                ("STREAMS_PLUGINS_DIR", "/plugins"),
                ("STREAMS_DATA_DIR", "datos ñ"),
                ("STREAMS_ALERTS_DIR", "/alerts"),
                ("STREAMS_STYLES_DIR", "css"),
            ],
        )
        .unwrap();
        assert_eq!(paths.config, Path::new("/work/cwd/cfg/custom.toml"));
        assert_eq!(paths.plugins, Path::new("/plugins"));
        assert_eq!(paths.data, Path::new("/work/cwd/datos ñ"));
        assert_eq!(paths.alerts, Path::new("/alerts"));
        assert_eq!(paths.styles, Path::new("/work/cwd/css"));
        assert_eq!(
            resolve(false, &[("STREAMS_HOME", "portable home")])
                .unwrap()
                .config,
            Path::new("/work/cwd/portable home/config.toml")
        );
    }
}
