//! Versioned, file-based JSON job input.
//!
//! A job describes exactly one processing run: the metadata base (a provider
//! reference OR supplied metadata, never both), an optional override layer,
//! and the output root/template. Per-field write policies, and cover/chapter
//! handling, arrive in follow-up PRs against the same schema.
//!
//! See the README "Job Input" section for the schema.
//!
//! Semantics that are contractual with callers:
//! - Supplied mode is authoritative: it never rematches by ASIN or filename.
//! - The selected release is preserved: an explicit release id must not
//!   silently become the book's first release.

use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use serde::de::Error as _;

use crate::api::MetadataSource;
use crate::metadata::BookMetadata;

/// Schema version accepted by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Job input errors.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("Job file I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Job parse error: {0}")]
    Parse(String),

    #[error("Invalid job: {0}")]
    Invalid(String),

    #[error("Metadata lookup failed: {0}")]
    Metadata(#[from] crate::api::MetadataError),
}

pub type Result<T> = std::result::Result<T, JobError>;

/// Per-field policy for a scalar metadata field.
///
/// `Write` writes the effective value, `Clear` removes the atom, and
/// `Preserve` leaves the file's existing value untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldPolicy {
    #[default]
    Preserve,
    Write,
    Clear,
}

/// Title-component policy. `preserve` (the default) leaves both existing
/// title components untouched; `clear` removes them; `write` writes the
/// effective metadata title (a blank effective title is a diagnosed failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitlePolicy {
    #[default]
    Preserve,
    Clear,
    Write,
}

/// Subtitle-component policy. `preserve` (the default) leaves the existing
/// subtitle component untouched; `clear` removes it; `write` writes the
/// effective metadata subtitle (a blank effective subtitle is a diagnosed
/// failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitlePolicy {
    #[default]
    Preserve,
    Clear,
    Write,
}

/// Scalar atom write policies, shared by the processor and the tagger.
///
/// The title/subtitle `write` variants carry no value: a written component
/// comes from the effective metadata, the same values that name the output
/// path. A policy value that diverged from the effective metadata would put
/// the file under one name while tagging it another, so the policy selects
/// the operation and the metadata owns the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePolicies {
    pub title: TitlePolicy,
    pub subtitle: SubtitlePolicy,
    pub authors: FieldPolicy,
    pub narrators: FieldPolicy,
    pub series: FieldPolicy,
    pub description: FieldPolicy,
    pub genres: FieldPolicy,
    pub year: FieldPolicy,
}

/// Cover policy. `write_url` (the default) embeds the provider cover URL
/// when the resolved metadata carries one — the legacy baseline; an explicit
/// `{"write": "path"}` embeds from a validated local file, `{"write_url":
/// "https://…"}` overrides the URL (under the existing SSRF guards), and
/// `clear` removes the cover. `preserve` (single input only) keeps the
/// source M4B's embedded cover without a download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverPolicy {
    Preserve,
    Write(PathBuf),
    WriteUrl(Option<String>),
    Clear,
}

impl Default for CoverPolicy {
    fn default() -> Self {
        Self::WriteUrl(None)
    }
}

impl<'de> Deserialize<'de> for CoverPolicy {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // `"preserve" | "clear" | "write" | "write_url"`, `{"write":
        // "path"}`, or `{"write_url": "https://…"}`. A bare `write_url` is
        // the documented default (use the provider cover); it cannot be
        // expressed by the derived newtype form, so the policy is parsed by
        // hand. A misspelled object field is rejected, never silently dropped.
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(word) => match word.as_str() {
                "preserve" => Ok(Self::Preserve),
                "clear" => Ok(Self::Clear),
                "write_url" => Ok(Self::WriteUrl(None)),
                "write" => Err(D::Error::invalid_value(
                    serde::de::Unexpected::Str("write"),
                    &"a cover file needs a path: {\"write\": \"/absolute/path\"}",
                )),
                other => Err(D::Error::invalid_value(
                    serde::de::Unexpected::Str(other),
                    &"\"preserve\", \"clear\", \"write\", or \"write_url\"",
                )),
            },
            serde_json::Value::Object(map) => {
                let mut entries = map.into_iter();
                let Some((key, val)) = entries.next() else {
                    return Ok(Self::default());
                };
                if entries.next().is_some() {
                    return Err(D::Error::unknown_field(&key, &["write", "write_url"]));
                }
                match key.as_str() {
                    "write" => {
                        let path = PathBuf::deserialize(val).map_err(D::Error::custom)?;
                        Ok(Self::Write(path))
                    }
                    "write_url" => match val {
                        serde_json::Value::Null => Ok(Self::WriteUrl(None)),
                        serde_json::Value::String(url) => Ok(Self::WriteUrl(Some(url))),
                        _ => Err(D::Error::invalid_value(
                            serde::de::Unexpected::Other("write_url value"),
                            &"a URL string",
                        )),
                    },
                    other => Err(D::Error::unknown_field(other, &["write", "write_url"])),
                }
            }
            _ => Err(D::Error::invalid_value(
                serde::de::Unexpected::Other("cover policy"),
                &"a word or a {\"write\": \"path\"} / {\"write_url\": \"https://…\"} object",
            )),
        }
    }
}

/// Chapters policy. `write` (the default) embeds chapters and writes the
/// `chapters.txt` sidecar: the explicit list when given
/// (`{"write": {"chapters": [ … ]}}` with integer-millisecond times),
/// otherwise the resolved chapters (provider chapters in provider mode,
/// input chapters, then generated per-track chapters — the documented
/// default behavior). `preserve` (single input only) skips all chapter
/// mutation, so the source chapters survive the native merge. `clear`
/// removes existing chapters from the output and skips the sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaptersPolicy {
    Preserve,
    /// Resolved chapters (`explicit: None`) or the explicit list.
    Write {
        explicit: Option<Vec<JobChapter>>,
    },
    Clear,
}

impl Default for ChaptersPolicy {
    fn default() -> Self {
        Self::Write { explicit: None }
    }
}

struct ChaptersWriteList {
    chapters: Vec<JobChapter>,
}

impl<'de> Deserialize<'de> for ChaptersWriteList {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            chapters: Vec<JobChapter>,
        }
        Ok(Self { chapters: Fields::deserialize(deserializer)?.chapters })
    }
}

impl<'de> Deserialize<'de> for ChaptersPolicy {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // `"preserve" | "clear" | "write"`, `{"write": null}` (resolved
        // chapters), or `{"write": {"chapters": [ … ]}}` (explicit list).
        // A misspelled object field is rejected, never silently dropped.
        use serde::de::Error;
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(word) => match word.as_str() {
                "preserve" => Ok(Self::Preserve),
                "clear" => Ok(Self::Clear),
                "write" => Ok(Self::Write { explicit: None }),
                other => Err(D::Error::invalid_value(
                    serde::de::Unexpected::Str(other),
                    &"\"preserve\", \"clear\", or \"write\"",
                )),
            },
            serde_json::Value::Object(map) => {
                let mut entries = map.into_iter();
                let Some((key, write_value)) = entries.next() else {
                    return Ok(Self::default());
                };
                if entries.next().is_some() {
                    return Err(D::Error::unknown_field(&key, &["write"]));
                }
                match key.as_str() {
                    "write" => Ok(match write_value {
                        serde_json::Value::Null => Self::Write { explicit: None },
                        other => Self::Write {
                            explicit: Some(
                                ChaptersWriteList::deserialize(other)
                                    .map_err(D::Error::custom)?
                                    .chapters,
                            ),
                        },
                    }),
                    other => Err(D::Error::unknown_field(other, &["write"])),
                }
            }
            _ => Err(D::Error::invalid_value(
                serde::de::Unexpected::Other("chapter policy"),
                &"a word or a {\"write\": …} object",
            )),
        }
    }
}

/// One explicit chapter; all times are integer milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobChapter {
    pub title: String,
    /// Chapter start time in milliseconds.
    pub start_millis: u64,
    /// Chapter duration in milliseconds.
    pub duration_millis: u64,
}

impl WritePolicies {
    /// The value a title `write` writes: the effective metadata's base title,
    /// the same `{title}` token that names the output path. The writer combines
    /// it with the subtitle component into `©nam`, so the written `©alb` (the
    /// base title) cannot diverge from the file name. A blank value is a
    /// diagnosed failure, not a silent clear.
    pub fn effective_title(&self, metadata: &crate::metadata::BookMetadata) -> Option<String> {
        let title = metadata.title.trim();
        (!title.is_empty()).then(|| title.to_string())
    }

    /// The value a subtitle `write` writes: the effective metadata subtitle.
    pub fn effective_subtitle(&self, metadata: &crate::metadata::BookMetadata) -> Option<String> {
        metadata.subtitle.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
    }
}

/// Provider base: an explicit book id with an optional exact release id.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderInput {
    pub book_id: String,
    /// Exact release id (AudiobookDB); it must not silently fall back to the
    /// book's first release.
    pub release_id: Option<String>,
}

/// Supplied base: complete or partial supported metadata values.
///
/// Only `title` is mandatory; omitted fields mean "nothing supplied".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppliedInput {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub narrators: Vec<String>,
    #[serde(default)]
    pub series_name: Option<String>,
    #[serde(default)]
    pub series_position: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub year: Option<u32>,
}

/// Manual override layer; legal on both bases.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverrideFields {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub authors: Option<Vec<String>>,
    pub narrators: Option<Vec<String>>,
    pub series_name: Option<String>,
    pub series_position: Option<String>,
    pub description: Option<String>,
    pub genres: Option<Vec<String>>,
    pub year: Option<u32>,
}

/// Metadata-source selection; only used in provider mode.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobSource {
    pub source: crate::api::MetadataSourceKind,
    pub api_url: Option<String>,
    pub region: Option<crate::api::MetadataRegion>,
}

/// Top-level job input.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub schema_version: u32,
    /// Input files or directories to process.
    pub inputs: Vec<PathBuf>,
    pub input_provider: Option<ProviderInput>,
    pub input_supplied: Option<SuppliedInput>,
    pub overrides: Option<OverrideFields>,
    pub output_root: PathBuf,
    #[serde(default)]
    pub path_format: String,
    #[serde(default)]
    pub completed_directory: Option<PathBuf>,
    #[serde(default)]
    pub source: Option<JobSource>,
    #[serde(default)]
    pub title: TitlePolicy,
    #[serde(default)]
    pub subtitle: SubtitlePolicy,
    #[serde(default)]
    pub authors: FieldPolicy,
    #[serde(default)]
    pub narrators: FieldPolicy,
    #[serde(default)]
    pub series: FieldPolicy,
    #[serde(default)]
    pub description: FieldPolicy,
    #[serde(default)]
    pub genres: FieldPolicy,
    #[serde(default)]
    pub year: FieldPolicy,
    #[serde(default)]
    pub cover: CoverPolicy,
    #[serde(default)]
    pub chapters: ChaptersPolicy,
}

impl Job {
    /// Read and parse a job file, then validate it.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let job: Self = serde_json::from_str(&raw)
            .map_err(|e| JobError::Parse(format!("{}: {e}", path.display())))?;
        job.validate()?;
        Ok(job)
    }

    /// Validate the job contract.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(JobError::Invalid(format!(
                "schema_version must be {SCHEMA_VERSION}, got {}",
                self.schema_version
            )));
        }
        match (&self.input_provider, &self.input_supplied) {
            (Some(_), Some(_)) => {
                return Err(JobError::Invalid(
                    "input_provider and input_supplied are mutually exclusive".into(),
                ));
            }
            (None, None) => {
                return Err(JobError::Invalid(
                    "exactly one of input_provider or input_supplied is required".into(),
                ));
            }
            _ => {}
        }
        if self.output_root.as_os_str().is_empty() {
            return Err(JobError::Invalid("output_root is required".into()));
        }
        if self.inputs.is_empty() {
            return Err(JobError::Invalid("inputs must not be empty".into()));
        }
        // Supplied mode: the effective series value (base, with an override
        // winning) is known now, so reject a blank write here, mirroring
        // title/subtitle. Provider mode's value is only known after the fetch,
        // so the tagger's pre-mutation check covers it.
        if self.series == FieldPolicy::Write {
            if let Some(supplied) = &self.input_supplied {
                // A non-blank override wins over the base; a blank override is
                // treated as absent, matching the override layer's semantics.
                let effective = self
                    .overrides
                    .as_ref()
                    .and_then(|o| o.series_name.as_deref())
                    .filter(|s| !s.trim().is_empty())
                    .or(supplied.series_name.as_deref());
                if effective.map(str::trim).unwrap_or_default().is_empty() {
                    return Err(JobError::Invalid(
                        "series policy 'write' requires a series name".into(),
                    ));
                }
            }
        }
        if let CoverPolicy::Write(file) = &self.cover {
            if !file.is_absolute() {
                return Err(JobError::Invalid(format!(
                    "cover file must be an absolute path, got {}",
                    file.display()
                )));
            }
        }
        Ok(())
    }

    /// Resolve the job against the metadata source (provider mode only) and
    /// apply the override layer.
    ///
    /// Failures here — an unresolvable book id, an explicit release the book
    /// does not carry, a client that cannot be built — are diagnosed before any
    /// merge or source move.
    pub async fn resolve(&self, source: Option<&MetadataSource>) -> Result<EffectiveJob> {
        let base = match (&self.input_provider, &self.input_supplied) {
            (Some(provider), None) => {
                let source = source.ok_or_else(|| {
                    JobError::Invalid("provider input requires a metadata source".into())
                })?;
                if provider.release_id.is_some() && source.as_audiobookdb().is_none() {
                    return Err(JobError::Invalid(
                        "release_id is only supported by the audiobookdb source".into(),
                    ));
                }
                // A blank explicit pin is an invalid selection, not an absent
                // one: it must fail, not silently fall back to the matched or
                // first release. Only `None` selects the fallback.
                if let Some(rid) = provider.release_id.as_deref().filter(|r| r.trim().is_empty()) {
                    return Err(JobError::Invalid(format!(
                        "release_id must be a non-blank release id, got {rid:?}"
                    )));
                }
                match provider.release_id.as_deref() {
                    Some(release_id) => {
                        source.fetch_book_with_release(&provider.book_id, Some(release_id)).await?
                    }
                    None => source.fetch_book(&provider.book_id).await?,
                }
            }
            (None, Some(supplied)) => {
                // Supplied mode is authoritative: no catalog lookup, no ASIN or
                // filename rematch, and no provider-dependent metadata_id. The
                // override layer is applied exactly once below, on the base.
                self.base_from_supplied(supplied)?
            }
            (Some(_), Some(_)) => unreachable!("validate() rejects both bases"),
            (None, None) => unreachable!("validate() requires exactly one base"),
        };

        let mut effective = base;
        apply_overrides(&mut effective, self.overrides.as_ref());

        // The title must be non-blank after overrides for naming.
        if effective.title.trim().is_empty() {
            return Err(JobError::Invalid("effective title is empty".into()));
        }

        Ok(EffectiveJob {
            metadata: effective,
            policies: self.policies_from(),
            cover: self.cover.clone(),
            chapters: self.chapters.clone(),
            path_format: self.path_format.clone(),
            completed_directory: self.completed_directory.clone(),
        })
    }

    /// The job's per-atom write policies, independent of the metadata base.
    fn policies_from(&self) -> WritePolicies {
        WritePolicies {
            title: self.title,
            subtitle: self.subtitle,
            authors: self.authors,
            narrators: self.narrators,
            series: self.series,
            description: self.description,
            genres: self.genres,
            year: self.year,
        }
    }

    /// The metadata source provider mode builds (`None` in supplied mode,
    /// which performs no catalog request).
    pub fn metadata_source(
        &self,
        config: &crate::config::Config,
    ) -> Result<Option<MetadataSource>> {
        let source = match (&self.input_provider, self.source.as_ref()) {
            (Some(_), Some(src)) => Some(
                MetadataSource::new_with_region(
                    src.source,
                    src.api_url.as_deref(),
                    src.region.unwrap_or(crate::api::MetadataRegion::Us),
                )
                .map_err(JobError::Metadata)?,
            ),
            (Some(_), None) => Some(
                MetadataSource::new_with_region(
                    config.metadata_source,
                    config.api_url.as_deref(),
                    config.region,
                )
                .map_err(JobError::Metadata)?,
            ),
            // Supplied input never needs a client, regardless of `source`.
            (None, _) => None,
        };
        Ok(source)
    }

    /// Non-mutating preview: resolve the effective metadata exactly as
    /// execution does and the same parsed template with the same resolver,
    /// returning the exact sanitized relative path plus the effective values
    /// for diagnostics. No directories, media, or name reservations happen,
    /// and no execution success is promised.
    pub async fn preview(&self, config: &crate::config::Config) -> Result<JobPreview> {
        let source = self.metadata_source(config)?;
        let effective = self.resolve(source.as_ref()).await?;
        let relative_path = crate::path_resolver::PathResolver::resolve_output(
            effective.template(),
            &effective.metadata,
        )
        .map_err(|e| JobError::Invalid(e.to_string()))?;
        Ok(JobPreview {
            schema_version: self.schema_version,
            resolver_version: crate::path_resolver::RESOLVER_VERSION,
            path_format: effective.template().to_string(),
            relative_path,
            metadata: effective.metadata.clone(),
        })
    }

    /// Build the supplied (authoritative) base metadata. No catalog lookup and
    /// no provider dependency: a supplied job resolves fully offline. The
    /// override layer is applied by [`Self::resolve`], exactly once, so a
    /// non-idempotent policy (e.g. `clear` after `write`) is never applied
    /// twice.
    fn base_from_supplied(&self, supplied: &SuppliedInput) -> Result<BookMetadata> {
        Ok(BookMetadata {
            metadata_id: String::new(),
            // Supplied mode has no canonical catalog book id: the `{book_id}`
            // token contributes no segment, and `metadata_id` stays empty too.
            book_id: String::new(),
            title: supplied.title.clone(),
            subtitle: supplied.subtitle.clone(),
            authors: supplied.authors.clone(),
            narrators: supplied.narrators.clone(),
            series_name: supplied.series_name.clone(),
            series_position: supplied.series_position.clone(),
            description: supplied.description.clone().unwrap_or_default(),
            genres: supplied.genres.clone(),
            year: supplied.year,
            cover_url: None,
            chapters: Vec::new(),
        })
    }
}

/// A non-mutating preview of a job's resolved output path and effective
/// metadata. `relative_path` is the exact sanitized relative path (inside
/// `output_root`) execution produces from the same effective metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobPreview {
    pub schema_version: u32,
    pub resolver_version: &'static str,
    /// The effective template that was resolved.
    pub path_format: String,
    /// The exact sanitized relative output path inside the job's
    /// `output_root`, including the output extension.
    pub relative_path: String,
    /// The effective (base + overrides) metadata the path was resolved from.
    pub metadata: crate::metadata::BookMetadata,
}

/// Machine-readable execution result: every produced output and sidecar,
/// per-group identity and outcome, diagnosed errors. Paths are relative to
/// the job's `output_root`; there is no absolute-path fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobResult {
    pub schema_version: u32,
    /// The resolver version that produced the output paths, matching
    /// `--capabilities` so a caller can verify the identity of a path.
    pub resolver_version: &'static str,
    /// `succeeded` when every group succeeded, `partial_failure` when some
    /// did and some did not, `failed` when none did.
    pub status: &'static str,
    /// The diagnosed error when the run failed before producing any group
    /// outcome (setup, discovery, or an unrecoverable run error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub groups: Vec<GroupResult>,
}

impl JobResult {
    /// `true` when every group succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.status == "succeeded"
    }

    /// A diagnosed failure before any group ran (e.g. processor setup or
    /// discovery), so a machine run always emits a parseable document.
    pub fn setup_failed(error: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            resolver_version: crate::path_resolver::RESOLVER_VERSION,
            status: "failed",
            groups: Vec::new(),
            error: Some(error),
        }
    }

    /// Build the machine result: every group in run order with its artifacts
    /// and diagnosed error; the status never claims success for a failed
    /// group. A CLI job run is one book (one group), so `partial_failure`
    /// arises only for multi-group library consumers.
    pub fn from_run(job_schema_version: u32, run: &crate::processor::RunOutcome) -> Self {
        let groups = run
            .outcomes
            .iter()
            .map(|o| GroupResult {
                group: o.group.clone(),
                outcome: o.outcome,
                input_files: o.input_files.clone(),
                artifacts: o.artifacts.clone(),
                files_moved: o.files_moved,
                error: o.error.clone(),
            })
            .collect();
        let status = if run.complete {
            "succeeded"
        } else if run.results.is_empty() {
            "failed"
        } else {
            "partial_failure"
        };
        Self {
            schema_version: job_schema_version,
            resolver_version: crate::path_resolver::RESOLVER_VERSION,
            status,
            groups,
            error: None,
        }
    }
}

/// The outcome of one processed group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupResult {
    pub group: String,
    pub outcome: GroupOutcome,
    /// The input files that were processed; relative where they sit under a
    /// path the caller owns. Not artifacts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input_files: Vec<PathBuf>,
    /// Every produced output and sidecar, relative to the job's
    /// `output_root` and validated to lie inside it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    /// `true` when the group's source files were moved to the completed
    /// directory.
    pub files_moved: bool,
    /// A diagnosed error when the group failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A single produced file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Artifact {
    /// The artifact's relative path inside the job's `output_root`.
    pub relative_path: String,
    /// `audio` for the merged output, `sidecar` for the chapter list.
    pub kind: &'static str,
    /// True when the file exists on disk at result time.
    pub exists: bool,
}

impl Artifact {
    /// An artifact from an absolute produced path, validated to lie inside
    /// the job's `output_root`. There is no absolute-path fallback: a path
    /// that does not contain `root` yields no artifact (the caller must not
    /// emit it), and a relative input is refused.
    pub fn from_absolute(root: &std::path::Path, path: &std::path::Path) -> Option<Self> {
        if !path.is_absolute() {
            return None;
        }
        let relative = path.strip_prefix(root).ok()?;
        let relative_path = relative.to_string_lossy().replace('\\', "/");
        Some(Self { kind: kind_of(&relative_path), exists: path.exists(), relative_path })
    }
}

fn kind_of(path: &str) -> &'static str {
    match std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("m4b") => "audio",
        Some(_) => "sidecar",
        None => "audio",
    }
}

/// A group's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupOutcome {
    Succeeded,
    Failed,
}

impl GroupOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// A job resolved into effective metadata plus policies.
#[derive(Debug, Clone)]
pub struct EffectiveJob {
    /// Effective (base + overrides) metadata; feeds naming and `Write` values.
    pub metadata: BookMetadata,
    /// Per-atom write policies for the scalar metadata fields.
    pub policies: WritePolicies,
    pub cover: CoverPolicy,
    pub chapters: ChaptersPolicy,
    pub path_format: String,
    pub completed_directory: Option<PathBuf>,
}

impl EffectiveJob {
    /// A merge cannot byte-preserve per-file chapters across inputs; the
    /// strict preserve promise is only valid for a single input file.
    pub fn preserve_chapters_requires_single_input(&self) -> bool {
        matches!(self.chapters, ChaptersPolicy::Preserve)
    }

    /// A strict cover-preservation promise is only valid for a single input.
    pub fn preserve_cover_requires_single_input(&self) -> bool {
        matches!(self.cover, CoverPolicy::Preserve)
    }

    /// The job's effective output template: its `path_format`, or the
    /// documented default when omitted.
    pub fn template(&self) -> &str {
        if self.path_format.is_empty() {
            crate::path_resolver::DEFAULT_TEMPLATE
        } else {
            &self.path_format
        }
    }
}

/// Non-blank, trimmed values in `values`, in order.
fn kept_people(values: &[String]) -> Vec<String> {
    values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()).map(str::to_string).collect()
}

/// Apply the override layer: present non-blank values win over the base.
fn apply_overrides(meta: &mut BookMetadata, overrides: Option<&OverrideFields>) {
    let Some(overrides) = overrides else { return };
    if let Some(title) = overrides.title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        meta.title = title.to_string();
    }
    if let Some(subtitle) = overrides.subtitle.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        meta.subtitle = Some(subtitle.to_string());
    }
    if let Some(authors) = &overrides.authors {
        let kept = kept_people(authors);
        if !kept.is_empty() {
            meta.authors = kept;
        }
    }
    if let Some(narrators) = &overrides.narrators {
        let kept = kept_people(narrators);
        if !kept.is_empty() {
            meta.narrators = kept;
        }
    }
    if let Some(name) = overrides.series_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        meta.series_name = Some(name.to_string());
    }
    if let Some(pos) = overrides.series_position.as_deref().map(str::trim).filter(|s| !s.is_empty())
    {
        meta.series_position = Some(pos.to_string());
    }
    if let Some(desc) = overrides.description.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        meta.description = desc.to_string();
    }
    if let Some(genres) = &overrides.genres {
        let kept = kept_people(genres);
        if !kept.is_empty() {
            meta.genres = kept;
        }
    }
    if let Some(year) = overrides.year {
        meta.year = Some(year);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_supplied_job() {
        let raw = r#"{
            "schema_version": 1,
            "inputs": ["in.m4b"],
            "input_supplied": { "title": "The Test" },
            "output_root": "/tmp/out",
            "path_format": "{author}/{title}",
            "title": "write"
        }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.schema_version, 1);
        assert!(job.input_supplied.is_some());
        assert_eq!(job.title, TitlePolicy::Write);
        assert!(matches!(job.authors, FieldPolicy::Preserve));
        assert!(matches!(job.cover, CoverPolicy::WriteUrl(_)));
        assert!(matches!(job.chapters, ChaptersPolicy::Write { explicit: None }));
        job.validate().unwrap();
    }

    #[tokio::test]
    async fn parses_unit_title_subtitle_write() {
        // Title and subtitle `write` are unit variants: the bare word selects
        // the operation; the value comes from the effective metadata, so the
        // policy carries no string and cannot diverge from the output path.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "title": "write", "subtitle": "write" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.title, TitlePolicy::Write);
        assert_eq!(job.subtitle, SubtitlePolicy::Write);
        job.validate().unwrap();
        // The effective value is the metadata's full title / subtitle.
        let effective = job.resolve(None).await.unwrap();
        assert_eq!(effective.policies.effective_title(&effective.metadata).as_deref(), Some("T"));
        assert_eq!(effective.policies.effective_subtitle(&effective.metadata), None);
    }

    #[test]
    fn rejects_relative_cover_path() {
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "cover": { "write": "cover.jpg" } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        let err = job.validate().unwrap_err().to_string();
        assert!(err.contains("absolute"), "{err}");
    }

    #[test]
    fn rejects_misspelled_chapter_policy_field() {
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "chapters": { "writ": null } }"#;
        let err: serde_json::Error = serde_json::from_str::<Job>(raw).unwrap_err();
        assert!(err.to_string().contains("writ"), "{}", err.to_string());
    }

    #[test]
    fn parses_chapter_write_forms() {
        // The bare word selects the resolved chapters.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "chapters": "write" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(matches!(job.chapters, ChaptersPolicy::Write { explicit: None }));

        // The default (no `chapters` key) is the resolved write, so a
        // multi-input job is not rejected by default.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(matches!(job.chapters, ChaptersPolicy::Write { explicit: None }));
        assert!(matches!(job.cover, CoverPolicy::WriteUrl(_)));

        // An object carries an explicit list.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o",
            "chapters": { "write": { "chapters": [ { "title": "C", "start_millis": 0, "duration_millis": 100 } ] } } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(matches!(job.chapters, ChaptersPolicy::Write { explicit: Some(_) }));
    }

    #[test]
    fn parses_cover_policy_forms() {
        // The documented default is the bare word `write_url`: it selects the
        // provider cover (no explicit URL), and it must parse — a derived
        // newtype form would require `{"write_url": null}` and silently break
        // the documented example.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "cover": "write_url" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.cover, CoverPolicy::WriteUrl(None));

        // The other bare words.
        for (word, expected) in [("preserve", CoverPolicy::Preserve), ("clear", CoverPolicy::Clear)]
        {
            let raw = format!(
                r#"{{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {{"title": "T"}},
                    "output_root": "/o", "cover": "{word}" }}"#
            );
            let job: Job = serde_json::from_str(&raw).unwrap();
            assert_eq!(job.cover, expected, "cover {word:?}");
        }

        // A local-file write carries its path.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "cover": { "write": "/data/book/cover.png" } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.cover, CoverPolicy::Write(PathBuf::from("/data/book/cover.png")));

        // A URL override carries its value; a bare `write_url` (null) does not.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "cover": { "write_url": "https://cdn.example/c.jpg" } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.cover, CoverPolicy::WriteUrl(Some("https://cdn.example/c.jpg".into())));
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "cover": { "write_url": null } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.cover, CoverPolicy::WriteUrl(None));
    }

    #[test]
    fn rejects_invalid_cover_policy_forms() {
        // A bare `write` has no path: it is rejected, not defaulted.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "cover": "write" }"#;
        let err: serde_json::Error = serde_json::from_str::<Job>(raw).unwrap_err();
        assert!(err.to_string().contains("path"), "{}", err.to_string());

        // A misspelled object field is rejected, never silently dropped.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "cover": { "writ": "/data/cover.png" } }"#;
        let err: serde_json::Error = serde_json::from_str::<Job>(raw).unwrap_err();
        assert!(err.to_string().contains("writ"), "{}", err.to_string());
    }

    #[test]
    fn rejects_write_without_value() {
        // A series write with no (or blank) supplied series name is rejected
        // at job validation, not after the merge.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T", "series_name": "  "},
            "output_root": "/o", "series": "write" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        let err = job.validate().unwrap_err().to_string();
        assert!(err.contains("series"), "{err}");

        // A blank base series is valid when a non-blank override supplies the
        // effective value: the override wins, so validation must accept it.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T", "series_name": "  "},
            "output_root": "/o", "series": "write",
            "overrides": { "series_name": "Dune" } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(job.validate().is_ok());
    }

    #[test]
    fn rejects_unknown_fields() {
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "bogus": 1 }"#;
        let err: serde_json::Error = serde_json::from_str::<Job>(raw).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn rejects_wrong_version() {
        let raw = r#"{ "schema_version": 2, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(job.validate().is_err());
    }

    #[test]
    fn rejects_mutually_exclusive_bases() {
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"],
            "input_provider": { "book_id": "B012345678" },
            "input_supplied": { "title": "T" },
            "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        let err = job.validate().unwrap_err().to_string();
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn rejects_missing_base() {
        let raw = r#"{ "schema_version": 1, "inputs": [], "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert!(job.validate().is_err());
    }

    #[test]
    fn explicit_id_kept_distinct() {
        // ASIN / canonical book id / selected release id stay separate fields.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"],
            "input_provider": { "book_id": "B012345678", "release_id": "rel12345678" },
            "source": { "source": "audiobookdb" },
            "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        job.validate().unwrap();
        let provider = job.input_provider.unwrap();
        assert_eq!(provider.book_id, "B012345678");
        assert_eq!(provider.release_id.as_deref(), Some("rel12345678"));
    }

    #[tokio::test]
    async fn rejects_blank_explicit_release_id() {
        // A blank explicit release pin is an invalid selection, not an absent
        // one: it must fail before any catalog fetch, not fall back to the
        // matched or first release.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"],
            "input_provider": { "book_id": "abc123def456", "release_id": "  " },
            "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        job.validate().unwrap();
        let source =
            crate::api::MetadataSource::new(crate::api::MetadataSourceKind::Audiobookdb, None)
                .unwrap();
        let err = job.resolve(Some(&source)).await.unwrap_err().to_string();
        assert!(err.contains("release_id"), "{err}");
    }

    #[tokio::test]
    async fn supplied_resolve_applies_overrides() {
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"],
            "input_supplied": { "title": "Base", "subtitle": "Sub", "authors": ["A"] },
            "overrides": { "title": "Override", "authors": ["B", "C"] },
            "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        let effective = job.resolve(None).await.unwrap();
        assert_eq!(effective.metadata.title, "Override");
        assert_eq!(effective.metadata.subtitle.as_deref(), Some("Sub"));
        assert_eq!(effective.metadata.authors, vec!["B", "C"]);
    }

    #[tokio::test]
    async fn supplied_resolve_rejects_blank_title() {
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"],
            "input_supplied": { "title": "   " },
            "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        let err = job.resolve(None).await.unwrap_err().to_string();
        assert!(err.contains("title"), "{err}");
    }

    #[tokio::test]
    async fn provider_resolve_without_client_fails_diagnosed() {
        // A provider job with no usable metadata source fails resolution with
        // a diagnosed error — before any directory, merge, or source move —
        // rather than proceeding with placeholder metadata.
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"],
            "input_provider": { "book_id": "B012345678" }, "output_root": "/o" }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        job.validate().unwrap();
        let err = job.resolve(None).await.unwrap_err().to_string();
        assert!(err.contains("metadata source"), "{err}");
    }

    #[test]
    fn artifact_validates_containment() {
        let root = std::path::PathBuf::from("/library/book");
        // Inside the root: the relative path is kept.
        let inside = root.join("Author/Book.m4b");
        let art = Artifact::from_absolute(&root, &inside).unwrap();
        assert_eq!(art.relative_path, "Author/Book.m4b");
        assert_eq!(art.kind, "audio");
        // A sidecar is classified as a sidecar.
        let sidecar = root.join("Author/Book.chapters.txt");
        assert_eq!(Artifact::from_absolute(&root, &sidecar).unwrap().kind, "sidecar");
        // Outside the root: refused, no absolute-path fallback.
        let outside = std::path::PathBuf::from("/elsewhere/Book.m4b");
        assert!(Artifact::from_absolute(&root, &outside).is_none());
        // A sibling prefix that merely shares text must not count:
        // /library/book-2 is not inside /library/book.
        let sibling = std::path::PathBuf::from("/library/book-2/Book.m4b");
        assert!(Artifact::from_absolute(&root, &sibling).is_none());
        // Relative input is refused.
        assert!(Artifact::from_absolute(&root, std::path::Path::new("Book.m4b")).is_none());
    }

    #[test]
    fn job_result_statuses_are_truthful() {
        use crate::processor::{GroupOutcomes, RunOutcome};
        let outcome = |group: &str, ok: bool, err: &str| GroupOutcomes {
            group: group.into(),
            artifacts: Vec::new(),
            input_files: Vec::new(),
            files_moved: false,
            outcome: if ok { GroupOutcome::Succeeded } else { GroupOutcome::Failed },
            error: (!ok).then(|| err.to_string()),
        };
        // `results` is empty except for the partial case: the status is
        // `failed` when no group succeeded, so zero results is the failed
        // shape and one is the partial shape.
        let run = |complete: bool, n: usize, outcomes: Vec<GroupOutcomes>| RunOutcome {
            results: (0..n)
                .map(|_| crate::processor::ProcessingResult {
                    input_files: Vec::new(),
                    output_file: "g.m4b".into(),
                    metadata_applied: true,
                    files_moved: false,
                    artifacts: Vec::new(),
                })
                .collect(),
            outcomes,
            complete,
            detail: String::new(),
        };

        let res = JobResult::from_run(
            1,
            &run(true, 2, vec![outcome("g1", true, ""), outcome("g2", true, "")]),
        );
        assert!(res.all_succeeded(), "all groups succeeded");
        assert_eq!(res.status, "succeeded");
        assert_eq!(res.groups.len(), 2);

        let res = JobResult::from_run(
            1,
            &run(false, 1, vec![outcome("g1", true, ""), outcome("g2", false, "boom")]),
        );
        assert!(!res.all_succeeded(), "partial must not claim full success");
        assert_eq!(res.status, "partial_failure");
        assert_eq!(res.groups.len(), 2, "every group is enumerated");
        assert_eq!(res.groups[1].error.as_deref(), Some("boom"));

        let res = JobResult::from_run(1, &run(false, 0, vec![outcome("g1", false, "nope")]));
        assert_eq!(res.status, "failed");
        assert!(!res.all_succeeded());
    }
}
