use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use tokio_retry::RetryIf;
use tokio_retry::strategy::{ExponentialBackoff, jitter};

use crate::metadata::{BookMetadata, Chapter};

use super::client::{
    BACKOFF_BASE_MS, CONNECT_TIMEOUT_SECS, DEFAULT_TIMEOUT_SECS, MAX_RETRIES,
    POOL_IDLE_TIMEOUT_SECS,
};

/// Default API base URL for audnexus
pub const DEFAULT_API_URL: &str = "https://api.audnex.us";

/// Audible region for Audnexus lookups (`?region=` query parameter).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum MetadataRegion {
    Au,
    Ca,
    De,
    Es,
    Fr,
    In,
    It,
    Jp,
    #[default]
    Us,
    Uk,
}

impl std::fmt::Display for MetadataRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::Au => "au",
            Self::Ca => "ca",
            Self::De => "de",
            Self::Es => "es",
            Self::Fr => "fr",
            Self::In => "in",
            Self::It => "it",
            Self::Jp => "jp",
            Self::Us => "us",
            Self::Uk => "uk",
        };
        f.write_str(code)
    }
}

/// Errors that can occur when calling the Audible API
#[derive(Debug, Error)]
pub enum AudibleError {
    #[error(
        "Invalid metadata_id '{0}': the audnexus source requires a 10-character ASIN (e.g. B08XYZ1234)"
    )]
    InvalidMetadataId(String),

    #[error("Book not found for metadata_id: {0}")]
    NotFound(String),

    #[error(
        "Book not available in region '{region}' for metadata_id: {metadata_id} — try --region with the book's Audible market"
    )]
    RegionUnavailable { metadata_id: String, region: MetadataRegion },

    #[error("Rate limited by API")]
    RateLimited,

    #[error("Request timeout")]
    Timeout,

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    ParseError(#[from] serde_json::Error),

    #[error("API error: {status} - {message}")]
    ApiError { status: u16, message: String },

    #[error("Retry exhausted after {0} attempts")]
    RetryExhausted(usize),
}

/// Client for the Audible/audnexus API
#[derive(Debug, Clone)]
pub struct AudibleClient {
    client: Client,
    base_url: String,
    region: MetadataRegion,
}

impl AudibleClient {
    /// Create a new AudibleClient with the default API URL and US region
    pub fn new() -> Result<Self, AudibleError> {
        Self::with_base_url_region(DEFAULT_API_URL, MetadataRegion::Us)
    }

    /// Create a new AudibleClient with a custom base URL, defaulting to the US region
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, AudibleError> {
        Self::with_base_url_region(base_url, MetadataRegion::Us)
    }

    /// Create a new AudibleClient with a custom base URL and an explicit region
    pub fn with_base_url_region(
        base_url: impl Into<String>,
        region: MetadataRegion,
    ) -> Result<Self, AudibleError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .pool_idle_timeout(Duration::from_secs(POOL_IDLE_TIMEOUT_SECS))
            .build()?;

        Ok(Self { client, base_url: base_url.into(), region })
    }

    /// Determine if an error is transient and should trigger retry
    fn is_transient_error(error: &AudibleError) -> bool {
        match error {
            // Network errors (timeouts, connection issues)
            AudibleError::Network(_) => true,
            // Rate limiting (should retry with backoff)
            AudibleError::RateLimited => true,
            // Request timeout
            AudibleError::Timeout => true,
            // Server errors (5xx)
            AudibleError::ApiError { status, .. } => *status >= 500,
            // All other errors are permanent
            _ => false,
        }
    }

    /// Validate that an ID is in the correct format
    pub fn validate_id(id: &str) -> Result<(), AudibleError> {
        if id.len() == 10 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            Ok(())
        } else {
            Err(AudibleError::InvalidMetadataId(id.to_string()))
        }
    }

    /// Fetch book metadata by ID with retry logic
    pub async fn fetch_book(&self, id: &str) -> Result<BookMetadata, AudibleError> {
        Self::validate_id(id)?;

        let retry_strategy =
            ExponentialBackoff::from_millis(BACKOFF_BASE_MS).map(jitter).take(MAX_RETRIES);

        let base_url = self.base_url.clone();
        let client = self.client.clone();
        let region = self.region;
        let id = id.to_string();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let base_url = base_url.clone();
                let id = id.clone();

                async move { Self::fetch_book_once(&client, &base_url, region, &id).await }
            },
            Self::is_transient_error,
        )
        .await
    }

    /// Build the audnexus book URL for the configured region.
    fn book_url(base_url: &str, id: &str, region: MetadataRegion) -> String {
        format!("{}/books/{}?region={}", base_url, id, region)
    }

    /// Audnexus reports a wrong-region item as 404 + `REGION_UNAVAILABLE`; surface it
    /// as a distinct error so the user sees the `--region` hint.
    fn not_found_error(id: &str, region: MetadataRegion, body: &str) -> AudibleError {
        if body.contains("REGION_UNAVAILABLE") {
            AudibleError::RegionUnavailable { metadata_id: id.to_string(), region }
        } else {
            AudibleError::NotFound(id.to_string())
        }
    }

    /// Single fetch attempt without retry logic
    async fn fetch_book_once(
        client: &Client,
        base_url: &str,
        region: MetadataRegion,
        id: &str,
    ) -> Result<BookMetadata, AudibleError> {
        let url = Self::book_url(base_url, id, region);
        tracing::debug!("Fetching metadata from: {}", url);

        let response = client.get(&url).send().await.map_err(|e| {
            tracing::debug!("Request failed: {}", e);
            e
        })?;

        let status = response.status();
        match status {
            StatusCode::OK => {
                let api_response: ApiBookResponse = response.json().await?;
                Ok(api_response.into_book_metadata())
            }
            StatusCode::NOT_FOUND => {
                let body = response.text().await.unwrap_or_default();
                Err(Self::not_found_error(id, region, &body))
            }
            StatusCode::TOO_MANY_REQUESTS => Err(AudibleError::RateLimited),
            StatusCode::REQUEST_TIMEOUT => Err(AudibleError::Timeout),
            _ => {
                let message = response.text().await.unwrap_or_default();
                Err(AudibleError::ApiError { status: status.as_u16(), message })
            }
        }
    }
}

/// API response structure for book lookup
#[derive(Debug, Deserialize)]
struct ApiBookResponse {
    asin: String,
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    authors: Vec<ApiPerson>,
    #[serde(default)]
    narrators: Vec<ApiPerson>,
    #[serde(default)]
    series: Vec<ApiSeries>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    genres: Vec<ApiGenre>,
    #[serde(default, rename = "releaseDate")]
    release_date: Option<String>,
    #[serde(default, rename = "chapterInfo")]
    chapter_info: Option<ApiChapterInfo>,
    #[serde(default)]
    image: Option<String>,
}

impl ApiBookResponse {
    fn into_book_metadata(self) -> BookMetadata {
        let year = self
            .release_date
            .as_ref()
            .and_then(|date| date.split('-').next())
            .and_then(|year_str| year_str.parse().ok());

        let chapters = self
            .chapter_info
            .map(|info| {
                info.chapters
                    .into_iter()
                    .map(|ch| Chapter {
                        title: ch.title,
                        start_time: Duration::from_millis(ch.start_offset_ms),
                        duration: Duration::from_millis(ch.length_ms),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let series_name = self.series.first().map(|s| s.name.clone());
        let series_position = self.series.first().and_then(|s| s.position.clone());

        BookMetadata {
            metadata_id: self.asin,
            title: self.title,
            subtitle: self.subtitle,
            authors: self.authors.into_iter().map(|a| a.name).collect(),
            narrators: self.narrators.into_iter().map(|n| n.name).collect(),
            series_name,
            series_position,
            description: self.summary.unwrap_or_default(),
            genres: self.genres.into_iter().map(|g| g.name).collect(),
            year,
            cover_url: self.image,
            chapters,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiPerson {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiSeries {
    name: String,
    #[serde(default)]
    position: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiGenre {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiChapterInfo {
    chapters: Vec<ApiChapter>,
}

#[derive(Debug, Deserialize)]
struct ApiChapter {
    title: String,
    #[serde(rename = "startOffsetMs")]
    start_offset_ms: u64,
    #[serde(rename = "lengthMs")]
    length_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_id_valid() {
        assert!(AudibleClient::validate_id("B08XYZ1234").is_ok());
        assert!(AudibleClient::validate_id("1234567890").is_ok());
        assert!(AudibleClient::validate_id("ABCDEFGHIJ").is_ok());
    }

    #[test]
    fn test_validate_id_invalid() {
        assert!(matches!(
            AudibleClient::validate_id("B08XYZ123"),
            Err(AudibleError::InvalidMetadataId(_))
        ));
        assert!(matches!(
            AudibleClient::validate_id("B08XYZ12345"),
            Err(AudibleError::InvalidMetadataId(_))
        ));
        assert!(matches!(
            AudibleClient::validate_id("B08-XYZ123"),
            Err(AudibleError::InvalidMetadataId(_))
        ));
        assert!(matches!(AudibleClient::validate_id(""), Err(AudibleError::InvalidMetadataId(_))));
    }

    #[test]
    fn test_api_response_deserialization() {
        let json = r#"{
            "asin": "B08XYZ1234",
            "title": "Test Book",
            "subtitle": "A Test Subtitle",
            "authors": [{"name": "Test Author"}],
            "narrators": [{"name": "Test Narrator"}],
            "series": [{"name": "Test Series", "position": "1"}],
            "summary": "Test description",
            "genres": [{"name": "Fiction"}],
            "releaseDate": "2023-01-15",
            "chapterInfo": {
                "chapters": [
                    {"title": "Chapter 1", "startOffsetMs": 0, "lengthMs": 360000}
                ]
            },
            "image": "https://example.com/cover.jpg"
        }"#;

        let response: ApiBookResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.asin, "B08XYZ1234");
        assert_eq!(response.title, "Test Book");
        assert_eq!(response.subtitle, Some("A Test Subtitle".to_string()));
        assert_eq!(response.authors.len(), 1);
        assert_eq!(response.authors[0].name, "Test Author");

        let metadata = response.into_book_metadata();
        assert_eq!(metadata.metadata_id, "B08XYZ1234");
        assert_eq!(metadata.year, Some(2023));
        assert_eq!(metadata.chapters.len(), 1);
        assert_eq!(metadata.chapters[0].start_time, Duration::from_millis(0));
    }

    #[test]
    fn test_api_response_minimal() {
        let json = r#"{
            "asin": "B08XYZ1234",
            "title": "Test Book"
        }"#;

        let response: ApiBookResponse = serde_json::from_str(json).unwrap();
        let metadata = response.into_book_metadata();
        assert_eq!(metadata.metadata_id, "B08XYZ1234");
        assert_eq!(metadata.title, "Test Book");
        assert!(metadata.authors.is_empty());
        assert!(metadata.chapters.is_empty());
        assert_eq!(metadata.description, "");
    }

    #[test]
    fn test_api_response_multi_author_multi_narrator() {
        let json = r#"{
            "asin": "B08C6YJ1LS",
            "title": "The Coldest Case",
            "subtitle": "A Black Book Audio Drama",
            "authors": [
                {"name": "Ryan Silbert"},
                {"name": "Alex Finlay"}
            ],
            "narrators": [
                {"name": "Luke Treadaway"},
                {"name": "Krysten Ritter"}
            ],
            "series": [{"name": "Black Book", "position": "2"}],
            "summary": "An audio drama",
            "genres": [{"name": "Mystery, Thriller & Suspense"}, {"name": "Thriller & Suspense"}],
            "releaseDate": "2020-06-23",
            "chapterInfo": {
                "chapters": [
                    {"title": "Chapter 1", "startOffsetMs": 0, "lengthMs": 3600000},
                    {"title": "Chapter 2", "startOffsetMs": 3600000, "lengthMs": 3700000}
                ]
            },
            "image": "https://example.com/cover.jpg"
        }"#;

        let response: ApiBookResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.asin, "B08C6YJ1LS");
        assert_eq!(response.title, "The Coldest Case");
        assert_eq!(response.authors.len(), 2);
        assert_eq!(response.authors[0].name, "Ryan Silbert");
        assert_eq!(response.authors[1].name, "Alex Finlay");
        assert_eq!(response.narrators.len(), 2);
        assert_eq!(response.narrators[0].name, "Luke Treadaway");
        assert_eq!(response.narrators[1].name, "Krysten Ritter");
        assert_eq!(response.series.len(), 1);
        assert_eq!(response.series[0].name, "Black Book");
        assert_eq!(response.series[0].position, Some("2".to_string()));

        let metadata = response.into_book_metadata();
        assert_eq!(metadata.metadata_id, "B08C6YJ1LS");
        assert_eq!(metadata.title, "The Coldest Case");
        assert_eq!(metadata.authors.len(), 2);
        assert_eq!(metadata.authors[0], "Ryan Silbert");
        assert_eq!(metadata.authors[1], "Alex Finlay");
        assert_eq!(metadata.narrators.len(), 2);
        assert_eq!(metadata.narrators[0], "Luke Treadaway");
        assert_eq!(metadata.narrators[1], "Krysten Ritter");
        assert_eq!(metadata.series_name, Some("Black Book".to_string()));
        assert_eq!(metadata.series_position, Some("2".to_string()));
        assert_eq!(metadata.year, Some(2020));
        assert_eq!(metadata.chapters.len(), 2);
    }

    #[tokio::test]
    async fn test_client_creation() {
        let client = AudibleClient::new().expect("default client should build");
        assert_eq!(client.base_url, DEFAULT_API_URL);
        assert_eq!(client.region, MetadataRegion::Us);

        let client =
            AudibleClient::with_base_url("https://custom.api.com").expect("client should build");
        assert_eq!(client.base_url, "https://custom.api.com");
        assert_eq!(client.region, MetadataRegion::Us, "with_base_url must keep the US default");

        let client =
            AudibleClient::with_base_url_region("https://custom.api.com", MetadataRegion::De)
                .expect("client should build");
        assert_eq!(client.base_url, "https://custom.api.com", "base_url must be stored as given");
        assert_eq!(
            client.region,
            MetadataRegion::De,
            "with_base_url_region must store the constructor-supplied region"
        );
    }

    #[tokio::test]
    async fn test_constructor_region_reaches_query_param() {
        // The contract spans constructor -> stored region -> `?region=` in the
        // request URL: the region handed to the constructor is what book_url renders.
        for (region, expected) in [
            (MetadataRegion::Us, "?region=us"),
            (MetadataRegion::De, "?region=de"),
            (MetadataRegion::Jp, "?region=jp"),
        ] {
            let client = AudibleClient::with_base_url_region("https://api.audnex.us", region)
                .expect("client should build");
            assert_eq!(client.region, region);
            assert!(
                AudibleClient::book_url(&client.base_url, "B00BM5F96W", client.region)
                    .ends_with(expected),
                "{region} must reach the query parameter"
            );
        }
    }

    #[test]
    fn test_book_url_appends_region() {
        assert_eq!(
            AudibleClient::book_url("https://api.audnex.us", "B00BM5F96W", MetadataRegion::De),
            "https://api.audnex.us/books/B00BM5F96W?region=de"
        );
        assert!(
            AudibleClient::book_url("https://api.audnex.us", "B00BM5F96W", MetadataRegion::Us)
                .ends_with("?region=us")
        );
    }

    #[test]
    fn test_not_found_error_maps_region_unavailable() {
        let body = r#"{"error":{"code":"REGION_UNAVAILABLE","message":"Item not available in region 'us' for ASIN: B00BM5F96W","details":{"asin":"B00BM5F96W","code":"REGION_UNAVAILABLE"}}}"#;
        let err = AudibleClient::not_found_error("B00BM5F96W", MetadataRegion::Jp, body);
        match &err {
            AudibleError::RegionUnavailable { metadata_id, region } => {
                assert_eq!(metadata_id, "B00BM5F96W");
                assert_eq!(*region, MetadataRegion::Jp);
            }
            other => panic!("expected RegionUnavailable, got {other:?}"),
        }
        let message = err.to_string();
        assert!(
            message.contains("region 'jp'"),
            "message should name the actual region: {message}"
        );
        assert!(message.contains("B00BM5F96W"), "message should name the metadata_id: {message}");
        assert!(
            !message.contains("e.g. --region de"),
            "message must not hard-code a region example: {message}"
        );

        assert!(matches!(
            AudibleClient::not_found_error("B00BM5F96W", MetadataRegion::Us, "not found"),
            AudibleError::NotFound(_)
        ));
        assert!(matches!(
            AudibleClient::not_found_error("B00BM5F96W", MetadataRegion::Us, ""),
            AudibleError::NotFound(_)
        ));
    }

    #[test]
    fn test_region_display_codes() {
        assert_eq!(MetadataRegion::Au.to_string(), "au");
        assert_eq!(MetadataRegion::Ca.to_string(), "ca");
        assert_eq!(MetadataRegion::De.to_string(), "de");
        assert_eq!(MetadataRegion::Es.to_string(), "es");
        assert_eq!(MetadataRegion::Fr.to_string(), "fr");
        assert_eq!(MetadataRegion::In.to_string(), "in");
        assert_eq!(MetadataRegion::It.to_string(), "it");
        assert_eq!(MetadataRegion::Jp.to_string(), "jp");
        assert_eq!(MetadataRegion::Us.to_string(), "us");
        assert_eq!(MetadataRegion::Uk.to_string(), "uk");
    }
}
