pub mod audible;
pub mod audiobookdb;

pub use audible::{AudibleClient, AudibleError};
pub use audiobookdb::{AudiobookdbClient, AudiobookdbError};

use crate::metadata::BookMetadata;

/// Unified metadata-source error.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error(transparent)]
    Audible(#[from] AudibleError),
    #[error(transparent)]
    Audiobookdb(#[from] AudiobookdbError),
}

/// User-selectable source (config + CLI).
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    clap::ValueEnum,
    serde::Deserialize,
    serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum MetadataSourceKind {
    #[default]
    Audiobookdb,
    Audnexus,
}

impl std::fmt::Display for MetadataSourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Audiobookdb => write!(f, "audiobookdb"),
            Self::Audnexus => write!(f, "audnexus"),
        }
    }
}

/// Runtime dispatch over the two clients.
pub enum MetadataSource {
    Audible(AudibleClient),
    Audiobookdb(AudiobookdbClient),
}

impl MetadataSource {
    /// Build the selected source. `api_url`, if provided, overrides the source's
    /// built-in default URL.
    pub fn new(kind: MetadataSourceKind, api_url: Option<&str>) -> Result<Self, MetadataError> {
        match kind {
            MetadataSourceKind::Audnexus => {
                let client = match api_url {
                    Some(u) => AudibleClient::with_base_url(u)?,
                    None => AudibleClient::new()?,
                };
                Ok(Self::Audible(client))
            }
            MetadataSourceKind::Audiobookdb => {
                let client = match api_url {
                    Some(u) => AudiobookdbClient::with_base_url(u),
                    None => AudiobookdbClient::new(),
                };
                Ok(Self::Audiobookdb(client))
            }
        }
    }

    pub async fn fetch_book(&self, id: &str) -> Result<BookMetadata, MetadataError> {
        match self {
            Self::Audible(c) => Ok(c.fetch_book(id).await?),
            Self::Audiobookdb(c) => Ok(c.fetch_book(id).await?),
        }
    }

    pub async fn download_cover(&self, url: &str) -> Result<Vec<u8>, MetadataError> {
        match self {
            Self::Audible(c) => Ok(c.download_cover(url).await?),
            Self::Audiobookdb(c) => Ok(c.download_cover(url).await?),
        }
    }
}
