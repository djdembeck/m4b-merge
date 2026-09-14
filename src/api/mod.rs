pub mod audible;
pub mod audiobookdb;

use audible::DEFAULT_API_URL;
pub use audible::{AudibleClient, AudibleError, MetadataRegion};
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
    /// Build the selected source, defaulting to the US region for audnexus.
    /// `api_url`, if provided, overrides the source's built-in default URL.
    pub fn new(kind: MetadataSourceKind, api_url: Option<&str>) -> Result<Self, MetadataError> {
        Self::new_with_region(kind, api_url, MetadataRegion::Us)
    }

    /// Build the selected source with an explicit region. `api_url`, if provided,
    /// overrides the source's built-in default URL. `region` only applies to the
    /// audnexus source; AudiobookDB has no region parameter.
    pub fn new_with_region(
        kind: MetadataSourceKind,
        api_url: Option<&str>,
        region: MetadataRegion,
    ) -> Result<Self, MetadataError> {
        match kind {
            MetadataSourceKind::Audnexus => {
                let client = AudibleClient::with_base_url_region(
                    api_url.unwrap_or(DEFAULT_API_URL),
                    region,
                )?;
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
            let source = MetadataSource::new_with_region(kind, api_url, MetadataRegion::Us)
                .unwrap_or_else(|e| panic!("{kind} {api_url:?} should build: {e}"));
            assert!(
                matches!(source, MetadataSource::Audiobookdb(_)) == expect_audiobookdb,
                "{kind} with {api_url:?} dispatched to the wrong client"
            );
        }
    }

    #[tokio::test]
    async fn test_metadata_source_new_defaults_to_us_region() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        // `new` is the US-default entry point for the audnexus source. The region
        // is only observable where it reaches the wire, so point the source at a
        // local listener and capture the request URL it issues.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        listener.set_nonblocking(true).expect("nonblocking listener");
        let addr = listener.local_addr().expect("listener addr");

        let responder = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "no request reached the listener");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("listener accept failed: {e}"),
                }
            };

            let mut request = Vec::new();
            let mut buf = [0u8; 512];
            loop {
                let n = stream.read(&mut buf).expect("read request");
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if request.windows(2).any(|w| w == b"\r\n") {
                    break;
                }
            }

            // A permanent 404: no retry, we only need the URL that was requested.
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            String::from_utf8_lossy(&request).into_owned()
        });

        let source =
            MetadataSource::new(MetadataSourceKind::Audnexus, Some(&format!("http://{addr}")))
                .expect("audnexus default should build");
        // The lookup itself fails (404); the request it made carries the region.
        let _ = source.fetch_book("B00BM5F96W").await;

        let request = responder.join().expect("listener thread");
        let request_line = request.lines().next().unwrap_or_default();
        assert!(
            request_line.ends_with("?region=us HTTP/1.1"),
            "MetadataSource::new must default the audnexus region to US, got: {request_line}"
        );

        // Defaulting must not affect which client a kind maps to, with or without
        // a URL override.
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
