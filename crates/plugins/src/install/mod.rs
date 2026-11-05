mod archive;
mod download;
mod fs;
mod github;
mod manager;
mod metadata;
mod source;

pub use archive::{ArchiveError, Portability, Staging, ValidatedPackage, validate_package};
pub use download::{DownloadError, DownloadedPackage, Downloader};
pub(crate) use fs::cleanup_stale;
pub use fs::{FsTransaction, TransactionError};
pub use github::{Asset, GithubClient, GithubError, Release};
pub use manager::{
    InstallError, InstallEvent, InstallOptions, InstallOutcome, Installer, PluginStatus,
};
pub use metadata::{InstalledMetadata, MetadataError, MetadataRecord, MetadataStore};
pub use source::{PluginSource, RequestedRelease, SourceError};

fn is_local_test_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_some()
        && url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|address| address.is_loopback())
}
