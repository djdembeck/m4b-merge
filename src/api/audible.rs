use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
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

    /// Set a custom timeout for requests
    pub fn with_timeout(self, timeout: Duration) -> Result<Self, AudibleError> {
        let client = Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10)) // Connection timeout
            .pool_idle_timeout(Duration::from_secs(30)) // Connection pool timeout
            .build()?;

        Ok(Self { client, base_url: self.base_url })
    }

    /// Validate ASIN format (10 alphanumeric characters)
    fn validate_asin(asin: &str) -> Result<(), AudibleError> {
        if asin.len() == 10 && asin.chars().all(|c| c.is_ascii_alphanumeric()) {
            Ok(())
        } else {
            Err(AudibleError::InvalidAsin(asin.to_string()))
        }
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

    /// Fetch book metadata by ASIN with retry logic
    ///
    /// Audnexus serves chapter markers from a separate endpoint
    /// (`/books/{asin}/chapters`), so they are fetched with their own retry pass.
    /// A chapter lookup failure does not discard the book metadata: the merge
    /// still gets tags and cover art, just without chapters.
    pub async fn fetch_book(&self, asin: &str) -> Result<BookMetadata, AudibleError> {
        Self::validate_asin(asin)?;

        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);

        let base_url = self.base_url.clone();
        let client = self.client.clone();
        let asin = asin.to_string();
        let retry_asin = asin.clone();

        let mut metadata = RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let base_url = base_url.clone();
                let asin = retry_asin.clone();

                async move { Self::fetch_book_once(&client, &base_url, &asin).await }
            },
            Self::is_transient_error,
        )
        .await?;

        match self.fetch_chapters(&asin).await {
            Ok(chapters) => metadata.chapters = chapters,
            Err(e) => tracing::warn!("Failed to fetch chapters for ASIN {}: {}", asin, e),
        }

        Ok(metadata)
    }

    /// Fetch chapter markers for an ASIN with retry logic
    ///
    /// Audnexus exposes chapters at `GET /books/{asin}/chapters`; the book
    /// endpoint (`GET /books/{asin}`) does not include them. A 404 means the book
    /// has no chapter list and yields an empty vector rather than an error.
    pub async fn fetch_chapters(&self, asin: &str) -> Result<Vec<Chapter>, AudibleError> {
        Self::validate_asin(asin)?;

        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);

        let base_url = self.base_url.clone();
        let client = self.client.clone();
        let asin = asin.to_string();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let base_url = base_url.clone();
                let asin = asin.clone();

                async move { Self::fetch_chapters_once(&client, &base_url, &asin).await }
            },
            Self::is_transient_error,
        )
        .await
    }

    /// Single chapter fetch attempt without retry logic
    async fn fetch_chapters_once(
        client: &Client,
        base_url: &str,
        asin: &str,
    ) -> Result<Vec<Chapter>, AudibleError> {
        let url = format!("{}/books/{}/chapters", base_url, asin);
        tracing::debug!("Fetching chapters from: {}", url);

        let response = client.get(&url).send().await?;
        let status = response.status();

        match status {
            StatusCode::OK => {
                let api_response: ApiChapterListResponse = response.json().await?;
                Ok(api_response.chapters.into_iter().map(Chapter::from).collect())
            }
            StatusCode::NOT_FOUND => Ok(Vec::new()),
            StatusCode::TOO_MANY_REQUESTS => Err(AudibleError::RateLimited),
            StatusCode::REQUEST_TIMEOUT => Err(AudibleError::Timeout),
            _ => {
                let message = response.text().await.unwrap_or_default();
                Err(AudibleError::ApiError { status: status.as_u16(), message })
            }
        }
    }

    /// Single fetch attempt without retry logic
    async fn fetch_book_once(
        client: &Client,
        base_url: &str,
        asin: &str,
    ) -> Result<BookMetadata, AudibleError> {
        let url = format!("{}/books/{}", base_url, asin);
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
            StatusCode::NOT_FOUND => Err(AudibleError::NotFound(asin.to_string())),
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

        let series_name = self.series.first().map(|s| s.name.clone());
        let series_position = self.series.first().and_then(|s| s.position.clone());

        BookMetadata {
            asin: self.asin,
            title: self.title,
            subtitle: self.subtitle,
            authors: self.authors.into_iter().map(|a| a.name).collect(),
            narrators: self.narrators.into_iter().map(|n| n.name).collect(),
            series_name,
            series_position,
            description: self.summary.as_deref().map(strip_html).unwrap_or_default(),
            genres: self.genres.into_iter().map(|g| g.name).collect(),
            year,
            cover_url: self.image,
            chapters: Vec::new(),
        }
    }
}

/// Convert the audnexus `summary` (an HTML fragment of the audible.com
/// description) into plain text for the comment tag:
///
/// - block-level tags (`<p>`, `<br>`, and list/heading tags) become newlines
/// - every other tag is removed
/// - HTML entities (`&amp;`, `&lt;`, `&quot;`, `&#39;`, numeric, ...) are decoded
/// - the result is trimmed of surrounding whitespace
fn strip_html(html: &str) -> String {
    const BLOCK_TAGS: [&str; 12] =
        ["p", "br", "div", "li", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6"];

    let mut out = String::with_capacity(html.len());
    let mut chars = html.char_indices().peekable();

    while let Some((idx, c)) = chars.next() {
        if c != '<' {
            out.push(c);
            continue;
        }

        // Find the end of the tag
        let tag_start = idx + 1;
        let mut tag_end = None;
        for (i, ch) in chars.by_ref() {
            if ch == '>' {
                tag_end = Some(i);
                break;
            }
        }
        let Some(tag_end) = tag_end else {
            // Unterminated tag: keep the rest as-is
            out.push_str(&html[tag_start..]);
            break;
        };

        let raw_tag = &html[tag_start..tag_end];
        // Strip attributes: first whitespace-delimited token is the name
        let name = raw_tag
            .trim_start_matches('/')
            .split(|ch: char| ch.is_whitespace())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();

        if BLOCK_TAGS.contains(&name.as_str()) {
            // Collapse runs of newlines so nested/adjacent blocks don't pile
            // up blank lines.
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }

    // Decode common HTML entities (named + numeric).
    let decoded = decode_entities(&out);

    // Trim outer whitespace and collapse 3+ consecutive newlines. Lines
    // introduced by block tags often carry a leading space from the source
    // markup ("<br /> 1. Shoot"), so strip whitespace at line starts.
    let mut collapsed = String::with_capacity(decoded.len());
    let mut newline_run = 0usize;
    let mut at_line_start = true;
    for ch in decoded.trim().chars() {
        if ch == '\n' {
            newline_run += 1;
            at_line_start = true;
            if newline_run > 2 {
                continue;
            }
        } else {
            if at_line_start && ch.is_whitespace() && newline_run > 0 {
                // Skip leading whitespace on lines created by block tags
                continue;
            }
            if !at_line_start || !ch.is_whitespace() {
                newline_run = 0;
            }
            at_line_start = false;
        }
        collapsed.push(ch);
    }

    collapsed
}

/// Decode HTML entities: `&amp; &lt; &gt; &quot; &apos; &#39; &#x27;` and
/// numeric references. Unknown entities are left verbatim.
fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];

        // Find the terminating ';' within a reasonable window (entities are
        // short); longer than 10 chars means it's a literal '&'.
        let Some(semi_rel) = rest[..rest.len().min(11)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };

        let entity = &rest[1..semi_rel];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{00A0}'),
            _ => {
                // Numeric: &#123; decimal or &#x1F; hexadecimal
                if let Some(digits) = entity.strip_prefix('#') {
                    let code = if let Some(hex) =
                        digits.strip_prefix('x').or_else(|| digits.strip_prefix('X'))
                    {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        digits.parse::<u32>().ok()
                    };
                    code.and_then(char::from_u32)
                } else {
                    None
                }
            }
        };

        match replacement {
            Some(ch) => {
                out.push(ch);
                rest = &rest[semi_rel + 1..];
            }
            None => {
                // Unknown entity: emit verbatim
                out.push_str(&rest[..semi_rel + 1]);
                rest = &rest[semi_rel + 1..];
            }
        }
    }

    out.push_str(rest);
    out
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

/// Response body of `GET /books/{asin}/chapters`
#[derive(Debug, Deserialize)]
struct ApiChapterListResponse {
    #[serde(default)]
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

impl From<ApiChapter> for Chapter {
    fn from(chapter: ApiChapter) -> Self {
        Chapter {
            title: chapter.title,
            start_time: Duration::from_millis(chapter.start_offset_ms),
            duration: Duration::from_millis(chapter.length_ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_asin_valid() {
        assert!(AudibleClient::validate_asin("B08XYZ1234").is_ok());
        assert!(AudibleClient::validate_asin("1234567890").is_ok());
        assert!(AudibleClient::validate_asin("ABCDEFGHIJ").is_ok());
    }

    #[test]
    fn test_validate_asin_invalid() {
        assert!(matches!(
            AudibleClient::validate_asin("B08XYZ123"),
            Err(AudibleError::InvalidAsin(_))
        ));
        assert!(matches!(
            AudibleClient::validate_asin("B08XYZ12345"),
            Err(AudibleError::InvalidAsin(_))
        ));
        assert!(matches!(
            AudibleClient::validate_asin("B08-XYZ123"),
            Err(AudibleError::InvalidAsin(_))
        ));
        assert!(matches!(AudibleClient::validate_asin(""), Err(AudibleError::InvalidAsin(_))));
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
            "image": "https://example.com/cover.jpg"
        }"#;

        let response: ApiBookResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.asin, "B08XYZ1234");
        assert_eq!(response.title, "Test Book");
        assert_eq!(response.subtitle, Some("A Test Subtitle".to_string()));
        assert_eq!(response.authors.len(), 1);
        assert_eq!(response.authors[0].name, "Test Author");

        let metadata = response.into_book_metadata();
        assert_eq!(metadata.asin, "B08XYZ1234");
        assert_eq!(metadata.year, Some(2023));
        // The book endpoint does not carry chapters; they come from `/chapters`.
        assert!(metadata.chapters.is_empty());
    }

    #[test]
    fn test_api_response_minimal() {
        let json = r#"{
            "asin": "B08XYZ1234",
            "title": "Test Book"
        }"#;

        let response: ApiBookResponse = serde_json::from_str(json).unwrap();
        let metadata = response.into_book_metadata();
        assert_eq!(metadata.asin, "B08XYZ1234");
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
        assert_eq!(metadata.asin, "B08C6YJ1LS");
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
    }

    #[test]
    fn test_api_chapter_list_deserialization() {
        let json = r#"{
            "asin": "B08XYZ1234",
            "isAccurate": true,
            "chapters": [
                {"title": "Opening Credits", "startOffsetMs": 0, "lengthMs": 11264},
                {"title": "Chapter 1", "startOffsetMs": 11264, "lengthMs": 2203838}
            ]
        }"#;

        let response: ApiChapterListResponse = serde_json::from_str(json).unwrap();
        let chapters: Vec<Chapter> = response.chapters.into_iter().map(Chapter::from).collect();

        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].title, "Opening Credits");
        assert_eq!(chapters[0].start_time, Duration::from_millis(0));
        assert_eq!(chapters[1].start_time, Duration::from_millis(11264));
        assert_eq!(chapters[1].duration, Duration::from_millis(2203838));
    }

    /// Spin up a minimal HTTP server that mimics audnexus: book metadata at
    /// `/books/{asin}` (which, as on the real API, carries no chapters) and the
    /// chapter list at `/books/{asin}/chapters`. Returns the server base URL.
    fn spawn_mock_audnexus() -> String {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 4096];
                let read = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]);
                let path = request.split_whitespace().nth(1).unwrap_or("");

                let body = if path.ends_with("/chapters") {
                    r#"{"asin":"B08XYZ1234","isAccurate":true,"chapters":[{"title":"Chapter 1","startOffsetMs":0,"lengthMs":3600000},{"title":"Chapter 2","startOffsetMs":3600000,"lengthMs":3700000}]}"#
                } else {
                    r#"{"asin":"B08XYZ1234","title":"Test Book","authors":[{"name":"Test Author"}]}"#
                };

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_fetch_book_loads_chapters_from_chapters_endpoint() {
        let base_url = spawn_mock_audnexus();
        let client = AudibleClient::with_base_url(base_url).unwrap();

        let metadata = client.fetch_book("B08XYZ1234").await.unwrap();

        assert_eq!(metadata.title, "Test Book");
        assert_eq!(metadata.authors, vec!["Test Author".to_string()]);
        // The mock's book body has no chapters, so a populated list here proves
        // `fetch_book` queried `/books/{asin}/chapters`.
        assert_eq!(metadata.chapters.len(), 2);
        assert_eq!(metadata.chapters[0].title, "Chapter 1");
        assert_eq!(metadata.chapters[1].start_time, Duration::from_millis(3600000));
    }

    #[tokio::test]
    async fn test_client_creation() {
        let client = AudibleClient::new();
        assert!(client.is_ok());

        let client = AudibleClient::with_base_url("https://custom.api.com");
        assert!(client.is_ok());
    }

    #[test]
    fn test_strip_html_reporter_example() {
        // The exact sample from issue #429: inline tags removed, block tags
        // become newlines, entities decoded.
        let html = "<i>FM & AM</i> found Carlin officially poking fun at, while \
            incorporating, his early material, performed in the lounges of America and on \
            <i>The Ed Sullivan Show</i>. It also marked Carlin's metamorphosis from \
            straight-laced to hippie, as he intentionally embraced the growing \
            counterculture.<p> The recording is divided into two separate manifestations of \
            humor, making it a sort of comedy concept album. One section focuses on \
            references geared toward the more wholesome, commercial oriented AM-radio \
            audience; the remaining material was intended for the \"hipper\" FM \
            audience.</p><p> Tracks: <br /> 1. Shoot <br /> 2. The Hair Piece <br /> 3. Sex \
            in Commercials <br /> 4. Drugs <br /> 5. Birth Control <br /> 6. Son of Wino \
            <br /> 7. Divorce Game <br /> 8. Ed Sullivan Self Taught <br /> 9. Let's Make a \
            Deal <br /> 10. The 11 O'Clock News</p><p> </p><b>Explicit Language Warning: \
            You must be 18 years or older to purchase this program.</b>";

        let text = strip_html(html);

        assert!(!text.contains('<'), "no tags may remain: {}", text);
        assert!(!text.contains("&amp;"), "entities must be decoded");
        assert!(text.starts_with("FM & AM found Carlin"), "inline text kept: {}", text);
        assert!(text.contains("The Ed Sullivan Show"), "inline tag content kept");
        // Block boundaries produce line breaks
        assert!(text.contains('\n'), "block tags must produce newlines");
        assert!(text.lines().any(|l| l.starts_with("1. Shoot")));
        assert!(text.lines().any(|l| l.starts_with("10. The 11 O'Clock News")));
        assert!(
            text.contains("Explicit Language Warning: You must be 18 years or older"),
            "trailing bold text kept: {}",
            text
        );
    }

    #[test]
    fn test_strip_html_entities() {
        assert_eq!(
            strip_html("A &amp; B &lt;tag&gt; &quot;quoted&quot; &#39;x&#39;"),
            "A & B <tag> \"quoted\" 'x'"
        );
        assert_eq!(strip_html("hex &#x27; quote"), "hex ' quote");
        assert_eq!(strip_html("unknown &bogus; entity"), "unknown &bogus; entity");
        assert_eq!(strip_html("lone & ampersand"), "lone & ampersand");
    }

    #[test]
    fn test_strip_html_block_tags_newlines() {
        let text = strip_html("<p>one</p><p>two</p><p>three</p>");
        assert_eq!(text, "one\ntwo\nthree");
    }

    #[test]
    fn test_strip_html_empty_and_plain() {
        assert_eq!(strip_html(""), "");
        assert_eq!(strip_html("no tags here"), "no tags here");
        assert_eq!(strip_html("<p></p>"), "");
    }

    #[test]
    fn test_into_book_metadata_strips_summary_html() {
        let json = r#"{
            "asin": "B08XYZ1234",
            "title": "Test Book",
            "summary": "<p><b>Bold</b> intro &amp; more</p><p>Second para</p>"
        }"#;

        let response: ApiBookResponse = serde_json::from_str(json).unwrap();
        let metadata = response.into_book_metadata();
        assert_eq!(metadata.description, "Bold intro & more\nSecond para");
    }
}
