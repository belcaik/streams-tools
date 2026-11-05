mod download;
mod github;
mod source;

pub use download::{DownloadError, DownloadedPackage, Downloader};
pub use github::{Asset, GithubClient, GithubError, Release};
pub use source::{PluginSource, RequestedRelease, SourceError};
