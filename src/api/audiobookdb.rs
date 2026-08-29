//! AudiobookDB client (public audiobookdb.org v1.0.0 API).
//!
//! Resolution flow: Audible ASINs are resolved via `GET /audiobooks/external/audible/{asin}`,
//! internal AudiobookDB IDs via `GET /books/{id}`, and any other identifier falls back to
//! `POST /search`. Chapter data comes from `GET /releases/{id}` (the matched release when the
//! ASIN endpoint links one, otherwise the book's first release).
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use tokio_retry::strategy::jitter;

use crate::metadata::{BookMetadata, Chapter};

pub const DEFAULT_API_URL: &str = "https://audiobookdb.org/api";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const CONNECT_TIMEOUT_SECS: u64 = 10;
const POOL_IDLE_TIMEOUT_SECS: u64 = 30;
const USER_AGENT: &str = "m4b-merge/0.1.0 (https://github.com/djdembeck/m4b-merge)";
const MAX_RETRIES: usize = 3;
const BACKOFF_BASE_MS: u64 = 1000;
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
    /// (exactly 10 alphanumeric characters starting with B).
    fn looks_like_asin(id: &str) -> bool {
        id.len() == 10 && id.starts_with('B') && id.chars().all(|c| c.is_ascii_alphanumeric())
    }

    /// Map a non-200 response to an `AudiobookdbError`. `not_found_id` gives the 404
    /// a meaningful id (a bare word for endpoints that have none, e.g. covers).
    fn http_error(status: StatusCode, body: String, not_found_id: &str) -> AudiobookdbError {
        match status {
            StatusCode::NOT_FOUND => AudiobookdbError::NotFound(not_found_id.to_string()),
            StatusCode::TOO_MANY_REQUESTS => AudiobookdbError::RateLimited,
            _ => AudiobookdbError::ApiError { status: status.as_u16(), message: body },
        }
    }

    /// Read a `Retry-After` header (seconds) from a 429 response, if present.
    fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
        resp.headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
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

    /// Run `run_attempt` up to `MAX_RETRIES` times, retrying transient errors with
    /// exponential backoff (honoring `Retry-After` on 429s when present).
    async fn with_retries<'a, T, F, Fut>(&'a self, run_attempt: F) -> Result<T, AudiobookdbError>
    where
        F: FnMut() -> Fut + 'a,
        Fut: Future<Output = Result<T, (AudiobookdbError, Option<Duration>)>> + 'a,
    {
        let mut run_attempt = run_attempt;
        for attempt in 0..MAX_RETRIES {
            match run_attempt().await {
                Ok(value) => return Ok(value),
                Err((error, retry_after)) => {
                    if !Self::is_transient_error(&error) || attempt + 1 >= MAX_RETRIES {
                        return Err(error);
                    }
                    let fallback =
                        Duration::from_millis((attempt + 1) as u64 * BACKOFF_BASE_MS.min(8192));
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

    /// Look up book metadata by identifier.
    ///
    /// Audible ASINs (10 alphanumeric characters starting with B) are resolved via
    /// `GET /audiobooks/external/audible/{asin}`; if that 404s, the identifier is
    /// retried as an internal ID. Other identifiers use `GET /books/{id}` first and
    /// fall back to `POST /search` (preferring a hit whose `id` equals the input,
    /// otherwise the first hit) when the book is not found directly.
    pub async fn fetch_book(&self, book_id: &str) -> Result<BookMetadata, AudiobookdbError> {
        // (book, release id for chapter data)
        let (book, matched_release_id) = if Self::looks_like_asin(book_id) {
            match self.resolve_external(book_id).await {
                Ok(b) => {
                    let matched = b.matched_release_id.clone();
                    (b, matched)
                }
                Err(AudiobookdbError::NotFound(_)) => {
                    // ASIN not in the catalog; the 10-char id might be an internal ID.
                    (self.get_book(book_id).await?, None)
                }
                Err(e) => return Err(e),
            }
        } else {
            match self.get_book(book_id).await {
                Ok(b) => (b, None),
                Err(AudiobookdbError::NotFound(_)) => {
                    // Shorter-than-3-char identifiers cannot be searched (API minimum).
                    if book_id.len() < 3 {
                        return Err(AudiobookdbError::IdNotFound(book_id.to_string()));
                    }
                    let hits = self.search_books(book_id).await?;
                    // Prefer a hit whose id matches exactly (case-sensitive), else
                    // keep the lenient behavior of taking the first hit.
                    let hit = hits
                        .iter()
                        .find(|h| h.id == book_id)
                        .or_else(|| hits.first())
                        .ok_or_else(|| AudiobookdbError::IdNotFound(book_id.to_string()))?;
                    (self.get_book(&hit.id).await?, None)
                }
                Err(e) => return Err(e),
            }
        };

        // Fetch release data for chapter information. The matched release
        // (ASIN-linked) is preferred; otherwise fall back to the book's first
        // release. Missing release data is not fatal because many books simply
        // lack chapter metadata.
        let release_id = matched_release_id.or_else(|| book.releases.first().map(|r| r.id.clone()));
        let release_data =
            if let Some(rid) = release_id { self.get_release(&rid).await.ok() } else { None };

        Ok(Self::map_book(book_id, &book, release_data.as_ref()))
    }

    pub async fn download_cover(&self, cover_url: &str) -> Result<Vec<u8>, AudiobookdbError> {
        self.with_retries(|| Box::pin(async { self.attempt_download(cover_url).await })).await
    }

    async fn attempt_download(
        &self,
        url: &str,
    ) -> Result<Vec<u8>, (AudiobookdbError, Option<Duration>)> {
        let resp = match self.client.get(url).send().await {
            Ok(resp) => resp,
            Err(e) => return Err((AudiobookdbError::Network(e), None)),
        };
        let status = resp.status();
        let retry_after =
            (status == StatusCode::TOO_MANY_REQUESTS).then(|| Self::retry_after(&resp)).flatten();
        if status == StatusCode::OK {
            let bytes = resp.bytes().await.map_err(|e| (AudiobookdbError::Network(e), None))?;
            return Ok(bytes.to_vec());
        }
        let body = resp.text().await.unwrap_or_default();
        Err((Self::http_error(status, body, "cover"), retry_after))
    }

    /// Map a book (and optional release) to metadata: authors/narrators from `people`,
    /// year from `copyright` (falling back to `originallyPublishedAt`), subtitle from
    /// `subtitle` (falling back to `disambiguation`), series position from
    /// `position.value`/`label`, cover from `coverImage`/`images`, chapters from the
    /// release's `chapterDetail` (sorted by ordinal, negative offsets skipped).
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
            metadata_id: caller_id.to_string(),
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

/// Cover URL for an image: the full-resolution original when available, otherwise
/// the documented 768px derivative of the storage key.
fn cover_url_of(img: &AudiobookdbImage) -> String {
    img.source_url
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{}/large.jpg", img.url))
}

#[derive(Debug, Deserialize, Clone)]
struct SearchDocumentBook {
    id: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbBook {
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

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
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
        assert_eq!(meta.metadata_id, "B000000000");
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
        // Out-of-order ordinals are sorted; the negative offset is dropped.
        assert_eq!(
            meta.chapters.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["Chapter 1", "Chapter 2", "Chapter 3"]
        );
        assert_eq!(meta.chapters[0].start_time, Duration::ZERO);
        assert_eq!(meta.chapters[1].start_time, Duration::from_millis(100_000));
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
}
