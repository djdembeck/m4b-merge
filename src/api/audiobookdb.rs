use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use tokio_retry::RetryIf;
use tokio_retry::strategy::{ExponentialBackoff, jitter};

use crate::metadata::{BookMetadata, Chapter};

pub const DEFAULT_API_URL: &str = "https://audiobookdb.org/api";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_RETRIES: usize = 3;

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
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct AudiobookdbClient {
    client: Client,
    base_url: String,
}

impl Default for AudiobookdbClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AudiobookdbClient {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_API_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { client, base_url: base_url.into() }
    }

    fn is_transient_error(error: &AudiobookdbError) -> bool {
        match error {
            AudiobookdbError::Network(_) => true,
            AudiobookdbError::RateLimited => true,
            AudiobookdbError::Timeout => true,
            AudiobookdbError::ApiError { status, .. } => *status >= 500,
            _ => false,
        }
    }

    async fn search_books(&self, query: &str) -> Result<Vec<SearchHit>, AudiobookdbError> {
        let url = format!("{}/search", self.base_url);
        let body = serde_json::json!({
            "query": query,
            "types": ["books"],
            "skip": 0,
            "take": 20
        });

        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);
        let client = self.client.clone();
        let url = url.clone();
        let body = body.clone();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let url = url.clone();
                let body = body.clone();
                async move {
                    let resp = client
                        .post(&url)
                        .header("Accept", "application/json")
                        .json(&body)
                        .send()
                        .await?;

                    let status = resp.status();
                    match status {
                        StatusCode::OK => {
                            let search_resp: SearchResponse = resp.json().await?;
                            Ok(search_resp.results)
                        }
                        StatusCode::TOO_MANY_REQUESTS => Err(AudiobookdbError::RateLimited),
                        _ => {
                            let msg = resp.text().await.unwrap_or_default();
                            Err(AudiobookdbError::ApiError {
                                status: status.as_u16(),
                                message: msg,
                            })
                        }
                    }
                }
            },
            Self::is_transient_error,
        )
        .await
    }

    async fn get_book(&self, id: &str, include: &str) -> Result<AudiobookdbBook, AudiobookdbError> {
        let url = format!("{}/books/{}?include={}", self.base_url, id, include);

        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);
        let client = self.client.clone();
        let url = url.clone();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let url = url.clone();
                async move {
                    let resp = client.get(&url).header("Accept", "application/json").send().await?;

                    let status = resp.status();
                    match status {
                        StatusCode::OK => Ok(resp.json().await?),
                        StatusCode::NOT_FOUND => Err(AudiobookdbError::NotFound(id.to_string())),
                        StatusCode::TOO_MANY_REQUESTS => Err(AudiobookdbError::RateLimited),
                        _ => {
                            let msg = resp.text().await.unwrap_or_default();
                            Err(AudiobookdbError::ApiError {
                                status: status.as_u16(),
                                message: msg,
                            })
                        }
                    }
                }
            },
            Self::is_transient_error,
        )
        .await
    }

    async fn get_release(
        &self,
        id: &str,
        include: &str,
    ) -> Result<AudiobookdbRelease, AudiobookdbError> {
        let url = format!("{}/releases/{}?include={}", self.base_url, id, include);

        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);
        let client = self.client.clone();
        let url = url.clone();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let url = url.clone();
                async move {
                    let resp = client.get(&url).header("Accept", "application/json").send().await?;

                    let status = resp.status();
                    match status {
                        StatusCode::OK => Ok(resp.json().await?),
                        StatusCode::NOT_FOUND => Err(AudiobookdbError::NotFound(id.to_string())),
                        StatusCode::TOO_MANY_REQUESTS => Err(AudiobookdbError::RateLimited),
                        _ => {
                            let msg = resp.text().await.unwrap_or_default();
                            Err(AudiobookdbError::ApiError {
                                status: status.as_u16(),
                                message: msg,
                            })
                        }
                    }
                }
            },
            Self::is_transient_error,
        )
        .await
    }

    pub async fn fetch_book(&self, id: &str) -> Result<BookMetadata, AudiobookdbError> {
        // Try direct ID lookup first (handles AudiobookDB internal IDs like jq3wT8UKmC7R)
        let book_result = self
            .get_book(id, "external,genres,people,releases,series,tags,images")
            .await;

        let book = match book_result {
            Ok(b) => b,
            Err(AudiobookdbError::NotFound(_)) => {
                // Fall back to ASIN search via external reference
                let results = self.search_books(id).await?;

                let book_id = results
                    .iter()
                    .filter(|h| h.r#type == "books" || h.r#type == "book")
                    .find(|h| {
                        let b: AudiobookdbBook =
                            serde_json::from_value(h.data.clone()).unwrap_or_default();
                        b.external.iter().any(|e| e.r#type == "Audible" && e.id == id)
                    })
                    .map(|h| h.id.clone());

                let book_id = match book_id {
                    Some(id) => id,
                    None => return Err(AudiobookdbError::IdNotFound(id.to_string())),
                };

                self.get_book(&book_id, "external,genres,people,releases,series,tags,images")
                    .await?
            }
            Err(e) => return Err(e),
        };

        let release = book.releases.first().map(|r| {
            let release_id = r.id.clone();
            async move {
                self.get_release(
                    &release_id,
                    "chapterDetail,external,images,language,people,publisher",
                )
                .await
            }
        });

        let release_data = if let Some(fut) = release { fut.await.ok() } else { None };

        let authors: Vec<String> = book
            .people
            .iter()
            .filter(|p| p.role.name == "Author" || p.role.name == "author")
            .map(|p| p.person.name.clone())
            .collect();

        let narrators: Vec<String> = book
            .people
            .iter()
            .filter(|p| p.role.name == "Narrator" || p.role.name == "narrator")
            .map(|p| p.person.name.clone())
            .collect();

        let series_name = book.series.first().map(|s| s.series.title.clone());
        let series_position = book.series.first().map(|s| s.position.clone());

        let year = book
            .originally_published_at
            .as_ref()
            .and_then(|d| d.split('-').next())
            .and_then(|y| y.parse().ok());

        let genres: Vec<String> = book.genres.iter().map(|g| g.title.clone()).collect();

        let cover_url = book.images.first().map(|i| i.url.clone());

        let chapters = release_data
            .as_ref()
            .map(|r| {
                r.chapter_detail
                    .as_ref()
                    .map(|cd| {
                        cd.chapters
                            .iter()
                            .map(|ch| Chapter {
                                title: ch.title.clone(),
                                start_time: Duration::from_millis(ch.start_offset_ms as u64),
                                duration: Duration::from_millis(ch.length_ms as u64),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        Ok(BookMetadata {
            metadata_id: id.to_string(),
            title: book.title.clone(),
            subtitle: book.disambiguation.clone(),
            authors,
            narrators,
            series_name,
            series_position,
            description: book
                .description
                .as_ref()
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            genres,
            year,
            cover_url,
            chapters,
        })
    }

    pub async fn download_cover(&self, cover_url: &str) -> Result<Vec<u8>, AudiobookdbError> {
        let retry_strategy = ExponentialBackoff::from_millis(1000).map(jitter).take(MAX_RETRIES);
        let client = self.client.clone();
        let url = cover_url.to_string();

        RetryIf::start(
            retry_strategy,
            move || {
                let client = client.clone();
                let url = url.clone();
                async move {
                    let resp = client.get(&url).send().await?;
                    let status = resp.status();
                    match status {
                        StatusCode::OK => Ok(resp.bytes().await?.to_vec()),
                        StatusCode::NOT_FOUND => {
                            Err(AudiobookdbError::NotFound("cover".to_string()))
                        }
                        StatusCode::TOO_MANY_REQUESTS => Err(AudiobookdbError::RateLimited),
                        status if status.is_server_error() => Err(AudiobookdbError::ApiError {
                            status: status.as_u16(),
                            message: "Server error".to_string(),
                        }),
                        _ => Err(AudiobookdbError::ApiError {
                            status: status.as_u16(),
                            message: "Failed to download cover".to_string(),
                        }),
                    }
                }
            },
            Self::is_transient_error,
        )
        .await
    }
}

// --- audiobookdb.org response types ---

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<SearchHit>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SearchHit {
    id: String,
    #[serde(rename = "type")]
    r#type: String,
    data: serde_json::Value,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone, Default)]
struct AudiobookdbBook {
    id: String,
    title: String,
    #[serde(default)]
    description: Option<serde_json::Value>,
    #[serde(default)]
    disambiguation: Option<String>,
    #[serde(rename = "originallyPublishedAt", default)]
    originally_published_at: Option<String>,
    #[serde(default)]
    images: Vec<AudiobookdbImage>,
    #[serde(default)]
    people: Vec<AudiobookdbPersonRelation>,
    #[serde(default)]
    releases: Vec<AudiobookdbIdTitle>,
    #[serde(default)]
    series: Vec<AudiobookdbBookInSeries>,
    #[serde(default)]
    external: Vec<AudiobookdbExternal>,
    #[serde(default)]
    genres: Vec<AudiobookdbIdTitle>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbRelease {
    #[serde(default)]
    chapter_detail: Option<AudiobookdbChapterDetail>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbChapterDetail {
    chapters: Vec<AudiobookdbChapter>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbChapter {
    title: String,
    start_offset_ms: i64,
    length_ms: i64,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbImage {
    id: String,
    url: String,
    #[serde(default)]
    width: Option<i32>,
    #[serde(default)]
    height: Option<i32>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbPersonRelation {
    role: AudiobookdbRoleRef,
    person: AudiobookdbPersonRef,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbRoleRef {
    name: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbPersonRef {
    name: String,
    id: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbIdTitle {
    id: String,
    title: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbBookInSeries {
    #[serde(rename = "ordinal")]
    position: String,
    series: AudiobookdbSeriesRef,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbSeriesRef {
    id: String,
    title: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
struct AudiobookdbExternal {
    #[serde(rename = "type")]
    r#type: String,
    id: String,
}
