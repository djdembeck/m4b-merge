use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use tokio_retry::RetryIf;
use tokio_retry::strategy::{ExponentialBackoff, jitter};

use crate::metadata::{BookMetadata, Chapter};

/// Default API base URL for audnexus
pub const DEFAULT_API_URL: &str = "https://api.audnex.us";

/// Default request timeout in seconds
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Maximum number of retry attempts
pub const MAX_RETRIES: usize = 3;

/// Errors that can occur when calling the Audible API
#[derive(Debug, Error)]
pub enum AudibleError {
    #[error("Invalid ASIN format: {0}")]
    InvalidAsin(String),

    #[error("Book not found for ASIN: {0}")]
    NotFound(String),

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
}

impl AudibleClient {
    /// Create a new AudibleClient with the default API URL
    pub fn new() -> Result<Self, AudibleError> {
        Self::with_base_url(DEFAULT_API_URL)
    }

    /// Create a new AudibleClient with a custom base URL
    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, AudibleError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(10)) // Connection timeout
            .pool_idle_timeout(Duration::from_secs(30)) // Connection pool timeout
            .build()?;

        Ok(Self { client, base_url: base_url.into() })
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
            Err(AudibleError::InvalidAsin(id.to_string()))
        }
    }

    /// Fetch book metadata by ID with retry logic
    pub async fn fetch_book(&self, id: &str) -> Result<BookMetadata, AudibleError> {
        Self::validate_id(id)?;

        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);

        let base_url = self.base_url.clone();
        let client = self.client.clone();
        let id = id.to_string();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let base_url = base_url.clone();
                let id = id.clone();

                async move { Self::fetch_book_once(&client, &base_url, &id).await }
            },
            Self::is_transient_error,
        )
        .await
    }

    /// Single fetch attempt without retry logic
    async fn fetch_book_once(
        client: &Client,
        base_url: &str,
        id: &str,
    ) -> Result<BookMetadata, AudibleError> {
        let url = format!("{}/books/{}", base_url, id);
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
            StatusCode::NOT_FOUND => Err(AudibleError::NotFound(id.to_string())),
            StatusCode::TOO_MANY_REQUESTS => Err(AudibleError::RateLimited),
            StatusCode::REQUEST_TIMEOUT => Err(AudibleError::Timeout),
            _ => {
                let message = response.text().await.unwrap_or_default();
                Err(AudibleError::ApiError { status: status.as_u16(), message })
            }
        }
    }

    /// Download cover image bytes from the cover URL
    pub async fn download_cover(&self, cover_url: &str) -> Result<Vec<u8>, AudibleError> {
        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);

        let client = self.client.clone();
        let url = cover_url.to_string();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let url = url.clone();

                async move { Self::download_cover_once(&client, &url).await }
            },
            Self::is_transient_error,
        )
        .await
    }

    /// Single download attempt without retry logic
    async fn download_cover_once(client: &Client, url: &str) -> Result<Vec<u8>, AudibleError> {
        let response = client.get(url).send().await?;

        match response.status() {
            StatusCode::OK => Ok(response.bytes().await?.to_vec()),
            StatusCode::NOT_FOUND => Err(AudibleError::NotFound("cover".to_string())),
            StatusCode::TOO_MANY_REQUESTS => Err(AudibleError::RateLimited),
            status if status.is_server_error() => Err(AudibleError::ApiError {
                status: status.as_u16(),
                message: "Server error".to_string(),
            }),
            status => Err(AudibleError::ApiError {
                status: status.as_u16(),
                message: "Failed to download cover".to_string(),
            }),
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
            Err(AudibleError::InvalidAsin(_))
        ));
        assert!(matches!(
            AudibleClient::validate_id("B08XYZ12345"),
            Err(AudibleError::InvalidAsin(_))
        ));
        assert!(matches!(
            AudibleClient::validate_id("B08-XYZ123"),
            Err(AudibleError::InvalidAsin(_))
        ));
        assert!(matches!(AudibleClient::validate_id(""), Err(AudibleError::InvalidAsin(_))));
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
        let client = AudibleClient::new();
        assert!(client.is_ok());

        let client = AudibleClient::with_base_url("https://custom.api.com");
        assert!(client.is_ok());
    }
}
