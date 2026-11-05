mod archive;
mod download;
mod fs;
mod github;
mod metadata;
mod source;

pub use archive::{ArchiveError, Portability, Staging, ValidatedPackage, validate_package};
pub use download::{DownloadError, DownloadedPackage, Downloader};
pub use fs::{FsTransaction, TransactionError};
pub use github::{Asset, GithubClient, GithubError, Release};
pub use metadata::{InstalledMetadata, MetadataError, MetadataRecord, MetadataStore};
pub use source::{PluginSource, RequestedRelease, SourceError};
