pub mod audible;
pub mod audiobookdb;

pub use audible::{AudibleClient, AudibleError};
pub use audiobookdb::{AudiobookdbClient, AudiobookdbError};

use crate::metadata::BookMetadata;

/// HTTP client settings shared by the metadata-source clients.
pub(crate) mod client {
    /// Default request timeout in seconds
    pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

    /// Connection-establishment timeout in seconds
    pub const CONNECT_TIMEOUT_SECS: u64 = 10;

    /// Connection-pool idle timeout in seconds
    pub const POOL_IDLE_TIMEOUT_SECS: u64 = 30;

    /// Maximum number of retry attempts after the initial request.
    /// Both metadata sources make at most `1 + MAX_RETRIES` total requests.
    pub const MAX_RETRIES: usize = 3;

    /// Base delay for backoff in milliseconds
    pub const BACKOFF_BASE_MS: u64 = 1000;
}

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
                    Some(u) => AudiobookdbClient::with_base_url(u)?,
                    None => AudiobookdbClient::new()?,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_source_dispatch_matrix() {
        // Audnexus maps to the Audible client; Audiobookdb to the AudiobookDB
        // client, independent of whether an API URL override is supplied.
        let cases = [
            (MetadataSourceKind::Audnexus, None, false),
            (MetadataSourceKind::Audnexus, Some("http://localhost:1"), false),
            (MetadataSourceKind::Audiobookdb, None, true),
            (MetadataSourceKind::Audiobookdb, Some("http://localhost:1"), true),
        ];
        for (kind, api_url, expect_audiobookdb) in cases {
            let source = MetadataSource::new(kind, api_url)
                .unwrap_or_else(|e| panic!("{kind} {api_url:?} should build: {e}"));
            assert!(
                matches!(source, MetadataSource::Audiobookdb(_)) == expect_audiobookdb,
                "{kind} with {api_url:?} dispatched to the wrong client"
            );
        }
    }
}
