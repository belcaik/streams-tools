mod github;
mod source;

pub use github::{Asset, GithubClient, GithubError, Release};
pub use source::{PluginSource, RequestedRelease, SourceError};
