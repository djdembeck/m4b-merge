//! AudiobookDB client (public audiobookdb.org v1.0.0 API).
//!
//! Resolution flow: Audible ASINs are resolved via `GET /audiobooks/external/audible/{asin}`,
//! internal AudiobookDB IDs via `GET /books/{id}`, and any other identifier falls back to
//! `POST /search` (only a hit whose `id` exactly matches the input is used; otherwise
//! the lookup errors). Chapter data comes from `GET /releases/{id}` (the matched release when the
//! ASIN endpoint links one, otherwise the book's first release).
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::{Duration, SystemTime};
use thiserror::Error;
use tokio::time::sleep;
use tokio_retry::strategy::jitter;

use crate::metadata::{BookMetadata, Chapter};

use super::client::{
    BACKOFF_BASE_MS, CONNECT_TIMEOUT_SECS, DEFAULT_TIMEOUT_SECS, MAX_RETRIES,
    POOL_IDLE_TIMEOUT_SECS,
};

pub const DEFAULT_API_URL: &str = "https://audiobookdb.org/api";
/// Upper bound (seconds) for a parsed `Retry-After` wait; longer waits are
/// clamped so a malformed or hostile header cannot stall the client.
const RETRY_AFTER_MAX_SECS: u64 = 60;
const USER_AGENT: &str =
    concat!("m4b-merge/", env!("CARGO_PKG_VERSION"), " (https://github.com/djdembeck/m4b-merge)");
const BOOK_INCLUDE: &str = "people,releases,series,genres,images";

#[derive(Debug, Error)]
pub enum AudiobookdbError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("API error {status}: {message}")]
    ApiError { status: u16, message: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("rate limited")]
    RateLimited,
    #[error("timeout")]
    Timeout,
    #[error("no book found for ID: {0}")]
    IdNotFound(String),
    #[error("connection error: {0}")]
    Connection(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct AudiobookdbClient {
    client: Client,
    base_url: String,
}

impl AudiobookdbClient {
    pub fn new() -> Result<Self, AudiobookdbError> {
        Self::with_base_url(DEFAULT_API_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, AudiobookdbError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .pool_idle_timeout(Duration::from_secs(POOL_IDLE_TIMEOUT_SECS))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| AudiobookdbError::Connection(e.to_string()))?;
        Ok(Self { client, base_url: base_url.into() })
    }

    fn is_transient_error(error: &AudiobookdbError) -> bool {
        match error {
            AudiobookdbError::Network(_) => true,
            AudiobookdbError::Connection(_) => true,
            AudiobookdbError::RateLimited => true,
            AudiobookdbError::Timeout => true,
            AudiobookdbError::ApiError { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// Returns true if the identifier looks like an Audible ASIN
    /// (exactly 10 alphanumeric characters, mirroring `AudibleClient::validate_id`).
    fn looks_like_asin(id: &str) -> bool {
        id.len() == 10 && id.chars().all(|c| c.is_ascii_alphanumeric())
    }

    /// Map a non-200 response to an `AudiobookdbError`. `not_found_id` gives the 404
    /// a meaningful id (the book/release id, or the bare word "request" for `post_json`).
    fn http_error(status: StatusCode, body: String, not_found_id: &str) -> AudiobookdbError {
        match status {
            StatusCode::NOT_FOUND => AudiobookdbError::NotFound(not_found_id.to_string()),
            // 408 is transient like the Audible client treats it: retry with backoff.
            StatusCode::REQUEST_TIMEOUT => AudiobookdbError::Timeout,
            StatusCode::TOO_MANY_REQUESTS => AudiobookdbError::RateLimited,
            _ => AudiobookdbError::ApiError { status: status.as_u16(), message: body },
        }
    }

    /// Read a `Retry-After` header from a 429 response, if present.
    fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
        resp.headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(Self::parse_retry_after)
    }

    /// Parse a `Retry-After` header value (RFC 9110: delta-seconds or HTTP-date).
    ///
    /// Delta-seconds are truncated to whole seconds. HTTP-dates are converted to a
    /// wait relative to now; past dates (negative waits) yield zero, so the caller
    /// sleeps only its 100 ms jitter minimum. Waits are clamped to
    /// `RETRY_AFTER_MAX_SECS` to bound how long a malformed or hostile header can
    /// stall the client. Fractional seconds are not supported and yield `None`.
    fn parse_retry_after(value: &str) -> Option<Duration> {
        let value = value.trim();
        if let Ok(secs) = value.parse::<u64>() {
            return Some(Duration::from_secs(secs.min(RETRY_AFTER_MAX_SECS)));
        }
        if let Ok(date) = httpdate::parse_http_date(value) {
            let now = SystemTime::now();
            // A past date yields zero so the caller sleeps only its jitter minimum.
            let wait = date
                .duration_since(now)
                .map(|d| d.min(Duration::from_secs(RETRY_AFTER_MAX_SECS)))
                .unwrap_or_default();
            return Some(wait);
        }
        None
    }

    /// Execute one HTTP attempt: send the request, parse a 200 body into `T`, and map
    /// anything else to an error. The 429 `Retry-After` delay (when the server
    /// provided one) is returned alongside the error so the retry loop can honor it.
    async fn attempt<T: for<'de> Deserialize<'de>>(
        &self,
        request: reqwest::RequestBuilder,
        not_found_id: &str,
    ) -> Result<T, (AudiobookdbError, Option<Duration>)> {
        let resp = match request.send().await {
            Ok(resp) => resp,
            // Network failures (timeouts, DNS, connection resets) are transient.
            Err(e) => return Err((AudiobookdbError::Network(e), None)),
        };
        let status = resp.status();
        let retry_after =
            (status == StatusCode::TOO_MANY_REQUESTS).then(|| Self::retry_after(&resp)).flatten();
        if status == StatusCode::OK {
            let value = resp.json().await.map_err(|e| (AudiobookdbError::Network(e), None))?;
            return Ok(value);
        }
        let body = resp.text().await.unwrap_or_default();
        Err((Self::http_error(status, body, not_found_id), retry_after))
    }

    /// Run `run_attempt` once, then retry transient errors up to `MAX_RETRIES` times
    /// (`1 + MAX_RETRIES` total requests, matching the Audible client), with linear
    /// backoff (1s, 2s, 3s; honoring `Retry-After` on 429s when present).
    async fn with_retries<'a, T, F, Fut>(&'a self, run_attempt: F) -> Result<T, AudiobookdbError>
    where
        F: FnMut() -> Fut + 'a,
        Fut: Future<Output = Result<T, (AudiobookdbError, Option<Duration>)>> + 'a,
    {
        let mut run_attempt = run_attempt;
        for attempt in 0..=MAX_RETRIES {
            match run_attempt().await {
                Ok(value) => return Ok(value),
                Err((error, retry_after)) => {
                    if !Self::is_transient_error(&error) || attempt >= MAX_RETRIES {
                        return Err(error);
                    }
                    let fallback =
                        Duration::from_millis(((attempt + 1) as u64 * BACKOFF_BASE_MS).min(8192));
                    let delay = retry_after
                        .map(|d| d + jitter(Duration::from_millis(100)))
                        .unwrap_or_else(|| fallback + jitter(Duration::from_millis(100)));
                    sleep(delay).await;
                }
            }
        }
        // The loop always returns: the last iteration bails on any error.
        unreachable!()
    }

    /// GET `url` and deserialize the 200 body into `T`.
    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        not_found_id: &str,
    ) -> Result<T, AudiobookdbError> {
        self.with_retries(|| async {
            self.attempt(self.client.get(url).header("Accept", "application/json"), not_found_id)
                .await
        })
        .await
    }

    /// POST a JSON body to `url` and deserialize the 200 body into `T`.
    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<T, AudiobookdbError> {
        self.with_retries(|| async {
            self.attempt(
                self.client.post(url).header("Accept", "application/json").json(body),
                "request",
            )
            .await
        })
        .await
    }

    /// Resolve an Audible ASIN via the external lookup endpoint. Returns the full
    /// book; `matchedReleaseId` (when present) links the ASIN to a specific release.
    async fn resolve_external(&self, asin: &str) -> Result<AudiobookdbBook, AudiobookdbError> {
        let url = format!("{}/audiobooks/external/audible/{}", self.base_url, asin.trim());
        self.get_json::<AudiobookdbBook>(&url, asin).await
    }

    /// Look up a book by internal AudiobookDB ID.
    async fn get_book(&self, id: &str) -> Result<AudiobookdbBook, AudiobookdbError> {
        let url = format!("{}/books/{}?include={}", self.base_url, id, BOOK_INCLUDE);
        self.get_json::<AudiobookdbBook>(&url, id).await
    }

    /// Look up a release by ID. The v1 API always includes every relation, so no
    /// `include` parameter is sent.
    async fn get_release(&self, id: &str) -> Result<AudiobookdbRelease, AudiobookdbError> {
        let url = format!("{}/releases/{}", self.base_url, id);
        self.get_json::<AudiobookdbRelease>(&url, id).await
    }

    /// Search the books collection. The v1 API returns a bare JSON array (no
    /// envelope, no pagination); anonymous responses omit `external` fields, so hits
    /// are only used to recover an internal book id.
    async fn search_books(&self, query: &str) -> Result<Vec<SearchDocumentBook>, AudiobookdbError> {
        let url = format!("{}/search", self.base_url);
        let body = serde_json::json!({ "q": query, "type": "books" });
        self.post_json::<Vec<SearchDocumentBook>>(&url, &body).await
    }

    /// Pick the first endpoint for `id`: ASIN-shaped identifiers go through the
    /// external lookup, everything else through the internal book endpoint.
    fn resolve_endpoint(id: &str) -> BookEndpoint {
        if Self::looks_like_asin(id) {
            BookEndpoint::ExternalAsin
        } else {
            BookEndpoint::InternalBook
        }
    }

    /// Decide the fallback after the initial lookup 404s: an ASIN-shaped id is
    /// retried as an internal id, a searchable id falls back to search, and an
    /// id shorter than the API's 3-character search minimum is a dead end.
    fn not_found_fallback(id: &str, initial: BookEndpoint) -> NotFoundFallback {
        match initial {
            BookEndpoint::ExternalAsin => NotFoundFallback::InternalBook,
            BookEndpoint::InternalBook => {
                if id.len() < 3 {
                    NotFoundFallback::GiveUp
                } else {
                    NotFoundFallback::Search
                }
            }
        }
    }

    /// Pick the release to fetch chapter data for: the ASIN-linked matched release
    /// wins, then the book's first release, then `None`.
    fn release_id_for(book: &AudiobookdbBook, matched_release_id: Option<&str>) -> Option<String> {
        matched_release_id
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .or_else(|| book.releases.first().map(|r| r.id.clone()))
    }

    /// Pick the search hit for a lookup id: an exact `id` match only. Returns
    /// `None` when no hit matches exactly — silently taking an unrelated hit would
    /// attach a different book's metadata to the input.
    fn select_search_hit<'a>(
        id: &str,
        hits: &'a [SearchDocumentBook],
    ) -> Option<&'a SearchDocumentBook> {
        hits.iter().find(|h| h.id == id)
    }

    /// Look up book metadata by identifier.
    ///
    /// ASIN-shaped identifiers (10 alphanumeric characters) are resolved via
    /// `GET /audiobooks/external/audible/{asin}`; if that 404s, the identifier is
    /// retried as an internal ID. Other identifiers use `GET /books/{id}` first and
    /// fall back to `POST /search` when the book is not found directly — only a
    /// search hit whose `id` exactly matches the input is used, otherwise the
    /// lookup errors with `IdNotFound`.
    pub async fn fetch_book(&self, book_id: &str) -> Result<BookMetadata, AudiobookdbError> {
        let endpoint = Self::resolve_endpoint(book_id);
        let first = match endpoint {
            BookEndpoint::ExternalAsin => self.resolve_external(book_id).await,
            BookEndpoint::InternalBook => self.get_book(book_id).await,
        };

        let book = match first {
            Ok(b) => b,
            Err(AudiobookdbError::NotFound(_)) => match Self::not_found_fallback(book_id, endpoint)
            {
                NotFoundFallback::InternalBook => {
                    // ASIN not in the catalog; the 10-char id might be an internal ID.
                    self.get_book(book_id).await?
                }
                NotFoundFallback::Search => {
                    let hits = self.search_books(book_id).await?;
                    // Exact match only: an id no hit carries is an error, not a
                    // silent fallback to an unrelated book.
                    let hit = Self::select_search_hit(book_id, &hits)
                        .ok_or_else(|| AudiobookdbError::IdNotFound(book_id.to_string()))?;
                    self.get_book(&hit.id).await?
                }
                NotFoundFallback::GiveUp => {
                    // Shorter-than-3-char identifiers cannot be searched (API minimum).
                    return Err(AudiobookdbError::IdNotFound(book_id.to_string()));
                }
            },
            Err(e) => return Err(e),
        };

        // Fetch release data for chapter information. The matched release
        // (ASIN-linked) is preferred; otherwise fall back to the book's first
        // release. Missing release data is not fatal because many books simply
        // lack chapter metadata.
        let release_id = Self::release_id_for(&book, book.matched_release_id.as_deref());
        let release_data =
            if let Some(rid) = release_id { self.get_release(&rid).await.ok() } else { None };

        Ok(Self::map_book(book_id, &book, release_data.as_ref()))
    }

    /// Map a book (and optional release) to metadata: authors/narrators from `people`,
    /// year from `copyright` (falling back to `originallyPublishedAt`), subtitle from
    /// `subtitle` (falling back to `disambiguation`), series position from
    /// `position.value`/`label`, cover from `coverImage`/`images`, chapters from the
    /// release's `chapterDetail` (sorted by ordinal, negative offsets skipped).
    ///
    /// `caller_id` is the identifier the caller supplied; when the resolved book
    /// carries its own canonical id it wins for `metadata_id` (e.g. an ASIN that
    /// resolved to an internal book), falling back to `caller_id` when absent.
    fn map_book(
        caller_id: &str,
        book: &AudiobookdbBook,
        release_data: Option<&AudiobookdbRelease>,
    ) -> BookMetadata {
        let has_role = |role: &str, target: &str| role.eq_ignore_ascii_case(target);

        let authors: Vec<String> = book
            .people
            .iter()
            .filter(|p| has_role(&p.role.name, "author"))
            .map(|p| p.person.name.clone())
            .collect();

        // Narrators live on the release, not the book, in most records — take them
        // from the release first, then the book, deduplicating by name.
        let mut narrators: Vec<String> = release_data
            .iter()
            .flat_map(|r| r.people.iter())
            .filter(|p| has_role(&p.role.name, "narrator"))
            .map(|p| p.person.name.clone())
            .collect();
        for p in book.people.iter().filter(|p| has_role(&p.role.name, "narrator")) {
            if !narrators.contains(&p.person.name) {
                narrators.push(p.person.name.clone());
            }
        }

        let first_series = book.series.first();
        let series_name = first_series.map(|s| s.series.title.clone());
        let series_position = first_series.and_then(|s| {
            let pos = s.position.value.clone().unwrap_or(s.position.label.clone());
            (!pos.is_empty()).then_some(pos)
        });

        let year = book.copyright.map(|c| c as u32).or_else(|| {
            book.originally_published_at
                .as_ref()
                .and_then(|d| d.split('-').next())
                .and_then(|y| y.parse().ok())
        });

        let genres: Vec<String> = book.genres.iter().map(|g| g.title.clone()).collect();

        // Full-resolution original (`sourceUrl`) is preferred for embedded covers;
        // fall back to the documented 768px derivative when only the base key exists.
        let cover_url = book.cover_image.as_ref().or_else(|| book.images.first()).map(cover_url_of);

        let subtitle = book
            .subtitle
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| book.disambiguation.clone().filter(|s| !s.is_empty()));

        // Skip chapters with negative offsets or lengths rather than wrapping: they
        // are data errors, and dropping them is the safer fallback. Server order is
        // not guaranteed, so sort by ordinal before mapping.
        let chapters: Vec<Chapter> = release_data
            .and_then(|r| r.chapter_detail.as_ref())
            .map(|cd| {
                let mut chapters: Vec<&AudiobookdbChapter> = cd
                    .chapters
                    .iter()
                    .filter(|ch| ch.start_offset_ms >= 0 && ch.length_ms >= 0)
                    .collect();
                chapters.sort_by_key(|ch| ch.ordinal);
                chapters
                    .into_iter()
                    .map(|ch| Chapter {
                        title: ch.title.clone(),
                        start_time: Duration::from_millis(ch.start_offset_ms as u64),
                        duration: Duration::from_millis(ch.length_ms as u64),
                    })
                    .collect()
            })
            .unwrap_or_default();

        BookMetadata {
            // Prefer the resolved book's canonical id over the caller's identifier.
            metadata_id: book
                .id
                .as_deref()
                .filter(|id| !id.is_empty())
                .unwrap_or(caller_id)
                .to_string(),
            title: book.title.clone(),
            subtitle,
            authors,
            narrators,
            series_name,
            series_position,
            description: book.description.clone().unwrap_or_default(),
            genres,
            year,
            cover_url,
            chapters,
        }
    }
}

/// The first endpoint to try for an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BookEndpoint {
    /// `GET /audiobooks/external/audible/{asin}` — ASIN-shaped identifiers.
    ExternalAsin,
    /// `GET /books/{id}` — everything else.
    InternalBook,
}

/// What to do when the initial lookup 404s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotFoundFallback {
    /// Retry the id as an internal book id.
    InternalBook,
    /// Search for it and use an exact-`id` hit, if any.
    Search,
    /// Give up with `IdNotFound` (id too short to search).
    GiveUp,
}

/// Cover URL for an image: the full-resolution original when available, otherwise
/// the documented 768px derivative of the storage key.
fn cover_url_of(img: &AudiobookdbImage) -> String {
    img.source_url
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/large.jpg", img.url))
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
struct SearchDocumentBook {
    id: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbBook {
    /// Canonical internal AudiobookDB id (present on the book endpoints and on the
    /// external-ASIN response's linked book).
    #[serde(default)]
    id: Option<String>,
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    disambiguation: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    copyright: Option<i32>,
    #[serde(rename = "originallyPublishedAt", default)]
    originally_published_at: Option<String>,
    #[serde(default)]
    images: Vec<AudiobookdbImage>,
    #[serde(rename = "coverImage", default)]
    cover_image: Option<AudiobookdbImage>,
    #[serde(default)]
    people: Vec<AudiobookdbPersonRelation>,
    #[serde(default)]
    releases: Vec<AudiobookdbReleaseRef>,
    #[serde(default)]
    series: Vec<AudiobookdbBookInSeries>,
    #[serde(default)]
    genres: Vec<AudiobookdbIdTitle>,
    #[serde(rename = "matchedReleaseId", default)]
    matched_release_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbReleaseRef {
    id: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbRelease {
    #[serde(rename = "chapterDetail", default)]
    chapter_detail: Option<AudiobookdbChapterDetail>,
    #[serde(default)]
    people: Vec<AudiobookdbPersonRelation>,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbChapterDetail {
    chapters: Vec<AudiobookdbChapter>,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbChapter {
    title: String,
    #[serde(default)]
    ordinal: i32,
    #[serde(rename = "startOffsetMs", default)]
    start_offset_ms: i64,
    #[serde(rename = "lengthMs", default)]
    length_ms: i64,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbImage {
    url: String,
    #[serde(default, rename = "sourceUrl")]
    source_url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbPersonRelation {
    role: AudiobookdbRoleRef,
    person: AudiobookdbPersonRef,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbRoleRef {
    name: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbPersonRef {
    name: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbIdTitle {
    title: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbBookInSeries {
    position: AudiobookdbSeriesPosition,
    series: AudiobookdbSeriesRef,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbSeriesPosition {
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    label: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbSeriesRef {
    title: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[tokio::test]
    async fn test_is_transient_error_matrix() {
        // A refused loopback connection yields a genuine reqwest::Error for the
        // Network arm (port 9 mirrors the other retry tests; nothing listens there).
        let network = reqwest::get("http://127.0.0.1:9").await.unwrap_err();
        let cases = [
            (AudiobookdbError::Network(network), true),
            (AudiobookdbError::Connection("refused".to_string()), true),
            (AudiobookdbError::RateLimited, true),
            (AudiobookdbError::Timeout, true),
            (AudiobookdbError::ApiError { status: 499, message: "gone".into() }, false),
            (AudiobookdbError::ApiError { status: 500, message: "boom".into() }, true),
            (AudiobookdbError::ApiError { status: 503, message: "boom".into() }, true),
            (AudiobookdbError::NotFound("id1".to_string()), false),
            (AudiobookdbError::IdNotFound("id1".to_string()), false),
            (
                AudiobookdbError::Serialization(serde_json::from_str::<i32>("nope").unwrap_err()),
                false,
            ),
        ];
        for (error, expected) in &cases {
            assert_eq!(AudiobookdbClient::is_transient_error(error), *expected, "{error:?}");
        }
    }

    #[test]
    fn test_http_error_mapping_table() {
        let cases = [
            (StatusCode::NOT_FOUND, "id1", Some("id1")),
            (StatusCode::REQUEST_TIMEOUT, "took too long", None),
            (StatusCode::TOO_MANY_REQUESTS, "slow down", None),
            (StatusCode::BAD_GATEWAY, "boom", Some("boom")),
            (StatusCode::SERVICE_UNAVAILABLE, "", Some("")),
        ];
        for (status, body, expected_message) in cases {
            let error = AudiobookdbClient::http_error(status, body.to_string(), "id1");
            match error {
                AudiobookdbError::Timeout => {
                    assert_eq!(status, StatusCode::REQUEST_TIMEOUT, "expected Timeout");
                }
                AudiobookdbError::RateLimited => {
                    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "expected RateLimited");
                }
                AudiobookdbError::NotFound(id) => {
                    assert_eq!(status, StatusCode::NOT_FOUND, "expected NotFound");
                    assert_eq!(id, "id1");
                }
                AudiobookdbError::ApiError { status: got, message } => {
                    assert_eq!(got, status.as_u16(), "expected ApiError({status})");
                    assert_eq!(message, expected_message.as_deref().unwrap(), "message payload");
                }
                other => panic!("unexpected error variant: {other:?}"),
            }
        }
    }

    #[test]
    fn test_looks_like_asin_table() {
        // Mirrors `AudibleClient::validate_id`: any 10-char ASCII alphanumeric
        // string (B-prefixed or not, any case) is ASIN-shaped.
        let cases = [
            ("B08XYZ1234", true),
            ("b08xyz1234", true),
            ("A08XYZ1234", true),
            ("1234567890", true),
            ("ABCDEFGHIJ", true),
            ("B08XYZ123", false),
            ("B08XYZ12345", false),
            ("B08-XYZ123", false),
            ("", false),
        ];
        for (id, expected) in cases {
            assert_eq!(AudiobookdbClient::looks_like_asin(id), expected, "id {id:?}");
        }
    }

    #[test]
    fn test_parse_retry_after_table() {
        // HTTP-dates beyond the cap clamp to exactly RETRY_AFTER_MAX_SECS, so every
        // case here is deterministic.
        let future_date = SystemTime::now() + Duration::from_secs(120);
        let past_date = SystemTime::now() - Duration::from_secs(120);
        let future = httpdate::fmt_http_date(future_date);
        let past = httpdate::fmt_http_date(past_date);
        let cap = Duration::from_secs(RETRY_AFTER_MAX_SECS);
        let cases = [
            // (input, expected)
            ("3", Some(Duration::from_secs(3))),
            ("0", Some(Duration::ZERO)),
            ("  5  ", Some(Duration::from_secs(5))),
            ("300", Some(cap)),
            (&future, Some(cap)),
            (&past, Some(Duration::ZERO)),
            // Fractional seconds are not supported (documented in parse_retry_after).
            ("12.5", None),
            ("abc", None),
            ("", None),
            ("not a date", None),
        ];
        for (input, expected) in cases {
            assert_eq!(AudiobookdbClient::parse_retry_after(input), expected, "input {input:?}");
        }
    }

    #[tokio::test]
    async fn test_with_retries_attempt_count() {
        let client =
            AudiobookdbClient::with_base_url("http://127.0.0.1:9").expect("client should build");
        let mut calls = 0;
        let result: Result<u32, _> = client
            .with_retries(|| {
                calls += 1;
                async {
                    Err((
                        AudiobookdbError::ApiError { status: 503, message: "unavailable".into() },
                        None,
                    ))
                }
            })
            .await;
        assert!(matches!(result, Err(AudiobookdbError::ApiError { status: 503, .. })));
        // 1 initial attempt + MAX_RETRIES retries.
        assert_eq!(calls, 1 + MAX_RETRIES);
    }

    #[tokio::test]
    async fn test_with_retries_honors_retry_after() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let client =
            AudiobookdbClient::with_base_url("http://127.0.0.1:9").expect("client should build");
        let calls = AtomicUsize::new(0);
        let start = Instant::now();
        let result: Result<u32, _> = client
            .with_retries(|| {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    if n <= 2 {
                        // A present (zero) Retry-After takes the honored branch:
                        // delay = 0 + jitter(100ms) instead of the linear 1s/2s.
                        Err((AudiobookdbError::RateLimited, Some(Duration::ZERO)))
                    } else {
                        Ok(42u32)
                    }
                }
            })
            .await;
        assert!(matches!(result, Ok(v) if v == 42));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // The linear fallback would sleep at least 1s + 2s; a zero Retry-After
        // yields only two jitter sleeps, so this bound separates the branches.
        assert!(start.elapsed() < Duration::from_millis(2500), "took {:?}", start.elapsed());
    }

    #[test]
    fn test_parse_retry_after_date_future_within_cap() {
        // A future date inside the cap yields roughly the remaining wait.
        let date = SystemTime::now() + Duration::from_secs(30);
        let parsed = AudiobookdbClient::parse_retry_after(&httpdate::fmt_http_date(date)).unwrap();
        assert!(parsed.as_millis() <= 30_000, "got {parsed:?}");
        assert!(parsed.as_millis() >= 29_000, "got {parsed:?}");
    }

    #[test]
    fn test_external_asin_with_matched_release_id() {
        let book: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "Dune",
                "matchedReleaseId": "rel123abc456"
            }"#,
        ))
        .unwrap();
        assert_eq!(book.matched_release_id.as_deref(), Some("rel123abc456"));
    }

    #[test]
    fn test_external_asin_without_matched_release_id() {
        // matchedReleaseId is absent (not null) when the ASIN is not release-linked.
        let book: AudiobookdbBook =
            serde_json::from_value(json(r#"{"id": "abc123def456", "title": "Dune"}"#)).unwrap();
        assert_eq!(book.matched_release_id, None);
    }

    #[test]
    fn test_narrators_merged_from_release_and_book() {
        // Most records carry narrators only on the release, not the book.
        let book: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "people": [{"role": {"name": "Author"}, "person": {"id": "p1", "name": "A. Author"}}]
            }"#,
        )).unwrap();
        let release: AudiobookdbRelease = serde_json::from_value(json(
            r#"{
                "id": "rel123abc456",
                "title": "R",
                "people": [{"role": {"name": "Narrator"}, "person": {"id": "p2", "name": "N. Narrator"}}]
            }"#,
        )).unwrap();
        let meta = AudiobookdbClient::map_book("abc123def456", &book, Some(&release));
        assert_eq!(meta.narrators, vec!["N. Narrator".to_string()]);
        assert_eq!(meta.authors, vec!["A. Author".to_string()]);

        // Book-level narrator duplicates are dropped.
        let release2: AudiobookdbRelease = serde_json::from_value(json(
            r#"{
                "id": "rel123abc456",
                "title": "R",
                "people": [
                    {"role": {"name": "Narrator"}, "person": {"id": "p2", "name": "N. Narrator"}},
                    {"role": {"name": "Author"}, "person": {"id": "p1", "name": "A. Author"}}
                ]
            }"#,
        ))
        .unwrap();
        let book2: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "people": [
                    {"role": {"name": "Narrator"}, "person": {"id": "p2", "name": "N. Narrator"}},
                    {"role": {"name": "Narrator"}, "person": {"id": "p3", "name": "M. Second"}}
                ]
            }"#,
        ))
        .unwrap();
        let meta2 = AudiobookdbClient::map_book("abc123def456", &book2, Some(&release2));
        assert_eq!(meta2.narrators, vec!["N. Narrator".to_string(), "M. Second".to_string()]);
        // Release authors are not treated as authors.
        assert!(meta2.authors.is_empty());
    }

    #[test]
    fn test_book_null_subtitle_copyright_year() {
        let book: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "The Name of the Wind",
                "subtitle": null,
                "disambiguation": "Robin Hobb novel",
                "copyright": 2015,
                "originallyPublishedAt": null,
                "description": "A story.",
                "people": [
                    {"role": {"name": "Author"}, "person": {"id": "p1", "name": "Patrick Rothfuss"}},
                    {"role": {"name": "narrator"}, "person": {"id": "p2", "name": "Scott Brick"}}
                ],
                "series": [
                    {
                        "ordinal": "1",
                        "position": {"value": "1", "label": "Book 1", "sortKey": 1, "kind": "point", "status": "complete", "spans": []},
                        "series": {"id": "s1", "title": "The Kingdoms"}
                    }
                ],
                "genres": [{"id": "g1", "title": "Fantasy"}],
                "coverImage": {"url": "img/base", "sourceUrl": "https://cdn.example/source.jpg:book"}
            }"#,
        )).unwrap();
        let meta = AudiobookdbClient::map_book("B000000000", &book, None);
        // The resolved book's canonical id wins over the caller's identifier.
        assert_eq!(meta.metadata_id, "abc123def456");
        assert_eq!(meta.title, "The Name of the Wind");
        // null subtitle falls back to disambiguation
        assert_eq!(meta.subtitle.as_deref(), Some("Robin Hobb novel"));
        assert_eq!(meta.year, Some(2015));
        assert_eq!(meta.authors, vec!["Patrick Rothfuss".to_string()]);
        assert_eq!(meta.narrators, vec!["Scott Brick".to_string()]);
        assert_eq!(meta.series_name.as_deref(), Some("The Kingdoms"));
        assert_eq!(meta.series_position.as_deref(), Some("1"));
        assert_eq!(meta.genres, vec!["Fantasy".to_string()]);
        assert_eq!(meta.description, "A story.");
        // sourceUrl wins for the cover
        assert_eq!(meta.cover_url.as_deref(), Some("https://cdn.example/source.jpg:book"));
    }

    #[test]
    fn test_year_falls_back_to_originally_published_at() {
        let book: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "copyright": null,
                "originallyPublishedAt": "1993-04-05T00:00:00.000Z"
            }"#,
        ))
        .unwrap();
        let meta = AudiobookdbClient::map_book("abc123def456", &book, None);
        assert_eq!(meta.year, Some(1993));
    }

    #[test]
    fn test_series_position_value_vs_label() {
        let with_value: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "series": [
                    {
                        "ordinal": "0.5",
                        "position": {"value": "1.5", "label": "Book 1.5", "sortKey": 1.5, "kind": "point", "status": "complete", "spans": []},
                        "series": {"id": "s1", "title": "Series"}
                    }
                ]
            }"#,
        )).unwrap();
        assert_eq!(
            AudiobookdbClient::map_book("abc123def456", &with_value, None)
                .series_position
                .as_deref(),
            Some("1.5")
        );

        // value null → label is the display source
        let null_value: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "series": [
                    {
                        "ordinal": null,
                        "position": {"value": null, "label": "Collection", "sortKey": null, "kind": "label", "status": "partial", "spans": []},
                        "series": {"id": "s1", "title": "Series"}
                    }
                ]
            }"#,
        )).unwrap();
        assert_eq!(
            AudiobookdbClient::map_book("abc123def456", &null_value, None)
                .series_position
                .as_deref(),
            Some("Collection")
        );

        // empty label (status "absent") → no position at all
        let absent: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "series": [
                    {
                        "ordinal": null,
                        "position": {"value": null, "label": "", "sortKey": null, "kind": "unknown", "status": "absent", "spans": []},
                        "series": {"id": "s1", "title": "Series"}
                    }
                ]
            }"#,
        )).unwrap();
        assert_eq!(
            AudiobookdbClient::map_book("abc123def456", &absent, None).series_position,
            None
        );
    }

    #[test]
    fn test_empty_string_fallback_boundaries() {
        // An empty subtitle is treated as absent: it falls back to a non-empty
        // disambiguation, and yields None when that is empty too.
        let cases = [
            (
                r#""subtitle": "", "disambiguation": "The Deluxe Edition""#,
                Some("The Deluxe Edition"),
            ),
            (r#""subtitle": "", "disambiguation": """#, None),
        ];
        for (fields, expected) in cases {
            let book: AudiobookdbBook = serde_json::from_value(json(&format!(
                r#"{{"id": "abc123def456", "title": "T", {fields}}}"#
            )))
            .unwrap();
            assert_eq!(
                AudiobookdbClient::map_book("abc123def456", &book, None).subtitle.as_deref(),
                expected,
                "{fields}"
            );
        }
    }

    #[test]
    fn test_release_chapters_sorted_by_ordinal_and_filtered() {
        let release: AudiobookdbRelease = serde_json::from_value(json(
            r#"{
                "id": "rel123abc456",
                "title": "Release",
                "chapterDetail": {
                    "chapters": [
                        {"title": "Chapter 3", "ordinal": 3, "startOffsetMs": 300000, "lengthMs": 100000},
                        {"title": "Chapter 1", "ordinal": 1, "startOffsetMs": 0, "lengthMs": 100000},
                        {"title": "Bad chapter", "ordinal": 4, "startOffsetMs": -1, "lengthMs": 1000},
                        {"title": "Bad length", "ordinal": 5, "startOffsetMs": 100000, "lengthMs": -1},
                        {"title": "Chapter 2", "ordinal": 2, "startOffsetMs": 100000, "lengthMs": 100000}
                    ]
                }
            }"#,
        )).unwrap();
        let meta = AudiobookdbClient::map_book(
            "abc123def456",
            &serde_json::from_value(json(r#"{"id": "abc123def456", "title": "T"}"#)).unwrap(),
            Some(&release),
        );
        // Out-of-order ordinals are sorted; the negative offset and negative
        // length are dropped.
        assert_eq!(
            meta.chapters.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["Chapter 1", "Chapter 2", "Chapter 3"]
        );
        assert_eq!(meta.chapters[0].start_time, Duration::ZERO);
        assert_eq!(meta.chapters[1].start_time, Duration::from_millis(100_000));
        assert_eq!(meta.chapters[2].start_time, Duration::from_millis(300_000));
        assert_eq!(meta.chapters[2].duration, Duration::from_millis(100_000));
    }

    #[test]
    fn test_cover_url_source_url_present_vs_absent() {
        let with_source: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "coverImage": {"url": "img/base", "sourceUrl": "https://cdn.example/source.jpg:book"}
            }"#,
        )).unwrap();
        assert_eq!(
            AudiobookdbClient::map_book("abc123def456", &with_source, None).cover_url.as_deref(),
            Some("https://cdn.example/source.jpg:book")
        );

        let without_source: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "coverImage": null,
                "images": [{"url": "img/base", "width": 1200, "height": 1200}]
            }"#,
        ))
        .unwrap();
        // No coverImage and no sourceUrl → documented 768px derivative of the key.
        assert_eq!(
            AudiobookdbClient::map_book("abc123def456", &without_source, None).cover_url.as_deref(),
            Some("img/base/large.jpg")
        );

        let no_images: AudiobookdbBook =
            serde_json::from_value(json(r#"{"id": "abc123def456", "title": "T"}"#)).unwrap();
        assert_eq!(AudiobookdbClient::map_book("abc123def456", &no_images, None).cover_url, None);

        // An empty sourceUrl string is treated as absent and falls back to the
        // 768px derivative of the base key.
        let empty_source: AudiobookdbBook = serde_json::from_value(json(
            r#"{
                "id": "abc123def456",
                "title": "T",
                "coverImage": {"url": "img/base", "sourceUrl": ""}
            }"#,
        ))
        .unwrap();
        assert_eq!(
            AudiobookdbClient::map_book("abc123def456", &empty_source, None).cover_url.as_deref(),
            Some("img/base/large.jpg")
        );
    }

    #[test]
    fn test_search_documents_deserialize() {
        let hits: Vec<SearchDocumentBook> = serde_json::from_value(json(
            r#"[
                {"id": "abc123def456", "title": "Dune"},
                {"id": "def456abc789", "title": "Dune Messiah", "genres": ["Science Fiction"]}
            ]"#,
        ))
        .unwrap();
        assert_eq!(hits[0].id, "abc123def456");
        assert_eq!(hits[1].id, "def456abc789");
    }

    #[test]
    fn test_resolve_endpoint_table() {
        let cases = [
            // ASIN-shaped (10 ASCII alphanumeric, any prefix/case) → external lookup.
            ("B08XYZ1234", BookEndpoint::ExternalAsin),
            ("1234567890", BookEndpoint::ExternalAsin),
            ("b08xyz1234", BookEndpoint::ExternalAsin),
            // Internal AudiobookDB ids and other identifiers → book endpoint.
            ("abc123def456", BookEndpoint::InternalBook),
            ("dune", BookEndpoint::InternalBook),
            ("123456789", BookEndpoint::InternalBook), // 9 chars: not ASIN-shaped
        ];
        for (id, expected) in cases {
            assert_eq!(AudiobookdbClient::resolve_endpoint(id), expected, "id {id:?}");
        }
    }

    #[test]
    fn test_not_found_fallback_table() {
        let external = BookEndpoint::ExternalAsin;
        let internal = BookEndpoint::InternalBook;
        let cases = [
            // ASIN 404 on the external endpoint → retry as internal id.
            ("B08XYZ1234", external, NotFoundFallback::InternalBook),
            ("1234567890", external, NotFoundFallback::InternalBook),
            // Searchable ids (>= 3 chars) 404 on the book endpoint → search.
            ("abc123def456", internal, NotFoundFallback::Search),
            ("dune", internal, NotFoundFallback::Search),
            // Below the API's 3-char search minimum → give up.
            ("ab", internal, NotFoundFallback::GiveUp),
            ("", internal, NotFoundFallback::GiveUp),
        ];
        for (id, endpoint, expected) in cases {
            assert_eq!(
                AudiobookdbClient::not_found_fallback(id, endpoint),
                expected,
                "id {id:?} after {endpoint:?}"
            );
        }
    }

    #[test]
    fn test_select_search_hit_exact_match_only() {
        let hits: Vec<SearchDocumentBook> = vec![
            SearchDocumentBook { id: "def456abc789".into() },
            SearchDocumentBook { id: "abc123def456".into() },
        ];
        // Exact id match wins regardless of position.
        assert_eq!(
            AudiobookdbClient::select_search_hit("abc123def456", &hits).map(|h| h.id.as_str()),
            Some("abc123def456")
        );
        // No exact match: no lenient first-hit fallback (F5).
        assert_eq!(AudiobookdbClient::select_search_hit("dune", &hits), None);
        // Empty hit list: none.
        let empty: Vec<SearchDocumentBook> = vec![];
        assert_eq!(AudiobookdbClient::select_search_hit("abc123def456", &empty), None);
    }

    #[test]
    fn test_metadata_id_falls_back_to_caller_id_when_book_id_absent() {
        // A book response without an id keeps the caller's identifier.
        let no_id: AudiobookdbBook =
            serde_json::from_value(json(r#"{"title": "No ID Book"}"#)).unwrap();
        let meta = AudiobookdbClient::map_book("B000000000", &no_id, None);
        assert_eq!(meta.metadata_id, "B000000000");

        // An empty-string id is treated as absent too.
        let empty_id: AudiobookdbBook =
            serde_json::from_value(json(r#"{"id": "", "title": "Empty ID Book"}"#)).unwrap();
        let meta = AudiobookdbClient::map_book("caller-1", &empty_id, None);
        assert_eq!(meta.metadata_id, "caller-1");
    }

    #[test]
    fn test_release_id_for_preferred_order() {
        let book: AudiobookdbBook = serde_json::from_value(json(
            r#"{"id": "b1", "title": "T", "releases": [{"id": "rel1"}]}"#,
        ))
        .unwrap();
        // Matched release wins over the book's first release.
        assert_eq!(
            AudiobookdbClient::release_id_for(&book, Some("rel-matched")),
            Some("rel-matched".to_string())
        );
        // Empty matched release ID is treated as absent → book's first release.
        assert_eq!(AudiobookdbClient::release_id_for(&book, Some("")), Some("rel1".to_string()));
        // No matched release → first release.
        assert_eq!(AudiobookdbClient::release_id_for(&book, None), Some("rel1".to_string()));
        // No matched release and no releases → None (chapter fetch skipped).
        let bare: AudiobookdbBook =
            serde_json::from_value(json(r#"{"id": "b2", "title": "T"}"#)).unwrap();
        assert_eq!(AudiobookdbClient::release_id_for(&bare, None), None);
    }
}
