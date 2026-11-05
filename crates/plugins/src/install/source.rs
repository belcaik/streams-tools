use std::{error::Error, fmt, str::FromStr};

use semver::Version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSource {
    owner: String,
    repo: String,
    requested: RequestedRelease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestedRelease {
    Latest,
    Tag(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError(String);

impl PluginSource {
    pub fn parse(value: &str) -> Result<Self, SourceError> {
        const PREFIX: &str = "https://github.com/";
        let rest = value
            .strip_prefix(PREFIX)
            .ok_or_else(|| SourceError("se requiere https://github.com/<owner>/<repo>".into()))?;
        if rest.contains(['?', '#']) || rest.contains('@') && rest.matches('@').count() != 1 {
            return Err(SourceError(
                "la fuente contiene query, fragmento o pin inválido".into(),
            ));
        }
        let (path, requested) = match rest.rsplit_once('@') {
            Some((path, tag)) => {
                let version = tag
                    .strip_prefix('v')
                    .ok_or_else(|| SourceError("el pin debe comenzar con v".into()))?;
                Version::parse(version)
                    .map_err(|error| SourceError(format!("pin semver inválido: {error}")))?;
                (path, RequestedRelease::Tag(tag.into()))
            }
            None => (rest, RequestedRelease::Latest),
        };
        let path = path.strip_suffix('/').unwrap_or(path);
        let (owner, repo) = path
            .split_once('/')
            .ok_or_else(|| SourceError("faltan owner o repo".into()))?;
        if repo.contains('/') {
            return Err(SourceError("no se admiten subrutas".into()));
        }
        let repo = repo.strip_suffix(".git").unwrap_or(repo);
        if !portable(owner) || !portable(repo) {
            return Err(SourceError("owner o repo no son portables".into()));
        }
        Ok(Self {
            owner: owner.into(),
            repo: repo.into(),
            requested,
        })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }
    pub fn repo(&self) -> &str {
        &self.repo
    }
    pub fn requested(&self) -> &RequestedRelease {
        &self.requested
    }
    pub fn repository_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }
}

fn portable(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

impl fmt::Display for PluginSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.repository_url())?;
        if let RequestedRelease::Tag(tag) = &self.requested {
            write!(formatter, "@{tag}")?;
        }
        Ok(())
    }
}

impl FromStr for PluginSource {
    type Err = SourceError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for SourceError {}
