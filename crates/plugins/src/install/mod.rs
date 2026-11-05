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
pub use manager::{InstallError, InstallOptions, InstallOutcome, Installer, PluginStatus};
pub use metadata::{InstalledMetadata, MetadataError, MetadataRecord, MetadataStore};
pub use source::{PluginSource, RequestedRelease, SourceError};
