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
/// title components untouched; `clear` removes them; `{"write": "Title"}`
/// writes the title.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitlePolicy {
    #[default]
    Preserve,
    Clear,
    Write(String),
}

/// Subtitle-component policy. `preserve` (the default) leaves the existing
/// subtitle component untouched; `clear` removes it; `{"write": "Sub"}`
/// writes the subtitle (a `null` value is rejected at validation).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitlePolicy {
    #[default]
    Preserve,
    Clear,
    Write(Option<String>),
}

/// Scalar atom write policies, shared by the processor and the tagger.
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

impl WritePolicies {
    /// The standalone CLI's documented behavior: write every supported field.
    pub fn write_all(title: String, subtitle: Option<String>) -> Self {
        Self {
            title: TitlePolicy::Write(title),
            subtitle: SubtitlePolicy::Write(subtitle),
            authors: FieldPolicy::Write,
            narrators: FieldPolicy::Write,
            series: FieldPolicy::Write,
            description: FieldPolicy::Write,
            genres: FieldPolicy::Write,
            year: FieldPolicy::Write,
        }
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
        if let TitlePolicy::Write(title) = &self.title {
            if title.trim().is_empty() {
                return Err(JobError::Invalid(
                    "title policy 'write' requires a title value".into(),
                ));
            }
        }
        if let SubtitlePolicy::Write(None) = &self.subtitle {
            return Err(JobError::Invalid(
                "subtitle policy 'write' requires a subtitle value".into(),
            ));
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
                // filename rematch, and no provider-dependent metadata_id.
                return self.effective_from_supplied(supplied, self.overrides.as_ref());
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
            path_format: self.path_format.clone(),
            completed_directory: self.completed_directory.clone(),
        })
    }

    /// The job's per-atom write policies, independent of the metadata base.
    fn policies_from(&self) -> WritePolicies {
        WritePolicies {
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            authors: self.authors,
            narrators: self.narrators,
            series: self.series,
            description: self.description,
            genres: self.genres,
            year: self.year,
        }
    }

    /// Build the effective job for a supplied (authoritative) base, applying
    /// the override layer exactly once. No catalog lookup and no provider
    /// dependency: a supplied job resolves fully offline.
    fn effective_from_supplied(
        &self,
        supplied: &SuppliedInput,
        overrides: Option<&OverrideFields>,
    ) -> Result<EffectiveJob> {
        let mut effective = BookMetadata {
            metadata_id: String::new(),
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
        };
        apply_overrides(&mut effective, overrides);
        if effective.title.trim().is_empty() {
            return Err(JobError::Invalid("effective title is empty".into()));
        }
        Ok(EffectiveJob {
            metadata: effective,
            policies: self.policies_from(),
            path_format: self.path_format.clone(),
            completed_directory: self.completed_directory.clone(),
        })
    }
}

/// A job resolved into effective metadata plus policies.
#[derive(Debug, Clone)]
pub struct EffectiveJob {
    /// Effective (base + overrides) metadata; feeds naming and `Write` values.
    pub metadata: BookMetadata,
    /// Per-atom write policies for the scalar metadata fields.
    pub policies: WritePolicies,
    pub path_format: String,
    pub completed_directory: Option<PathBuf>,
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
            "title": { "write": "The Test" }
        }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        assert_eq!(job.schema_version, 1);
        assert!(job.input_supplied.is_some());
        assert!(matches!(job.title, TitlePolicy::Write(_)));
        assert!(matches!(job.authors, FieldPolicy::Preserve));
        job.validate().unwrap();
    }

    #[test]
    fn rejects_write_without_value() {
        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "title": { "write": "  " } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        let err = job.validate().unwrap_err().to_string();
        assert!(err.contains("title"), "{err}");

        let raw = r#"{ "schema_version": 1, "inputs": ["in.m4b"], "input_supplied": {"title": "T"},
            "output_root": "/o", "subtitle": { "write": null } }"#;
        let job: Job = serde_json::from_str(raw).unwrap();
        let err = job.validate().unwrap_err().to_string();
        assert!(err.contains("subtitle"), "{err}");
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
}
