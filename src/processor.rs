use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::api::{MetadataError, MetadataSource};
use crate::audio::ffmpeg::{FFmpeg, FFmpegError};
use crate::config::Config;
use crate::discovery::{AudioFile, AudioGroup, DiscoveryError};
use crate::merge::{MergeError, MergeJob, Merger};
use crate::metadata::BookMetadata;
use crate::tagging::{Tagger, TaggingError, describe_out_of_range_chapters, format_duration};

/// Static metadata ID regex compiled once at startup
static METADATA_ID_REGEX: OnceLock<regex::Regex> = OnceLock::new();

/// Returns the metadata ID regex. Matches exactly the two ID shapes the API
/// clients accept: an Audible ASIN (10 alphanumeric characters; the 10-char
/// alternative is case-insensitive to match `AudibleClient::validate_id` and
/// `AudiobookdbClient::looks_like_asin`) or an AudiobookDB internal id (12
/// alphanumeric characters). A bracketed word whose content is neither 10 nor
/// 12 alphanumeric characters (e.g. `[Unabridged 2020]`) yields no lookup.
/// The pattern cannot tell a descriptive marker from an ID, so callers must
/// collect every match and try them in order.
fn get_metadata_id_regex() -> &'static regex::Regex {
    METADATA_ID_REGEX.get_or_init(|| {
        regex::Regex::new(r"\[([A-Za-z0-9]{10}|[A-Za-z0-9]{12})\]")
            .expect("Invalid metadata ID regex pattern")
    })
}

/// Errors that can occur during processing
#[derive(Error, Debug)]
pub enum ProcessorError {
    #[error("Discovery error: {0}")]
    Discovery(#[from] DiscoveryError),

    #[error("Merge error: {0}")]
    Merge(#[from] MergeError),

    #[error("API error: {0}")]
    Api(#[from] MetadataError),

    #[error("Tagging error: {0}")]
    Tagging(#[from] TaggingError),

    #[error("No input files provided")]
    NoInputs,

    #[error("Job input error: {0}")]
    Job(#[from] crate::job::JobError),

    #[error("No output directory specified")]
    NoOutputDir,

    #[error("Failed to create output directory: {0}")]
    OutputDirCreation(String),

    #[error("FFmpeg not found: {0}")]
    FFmpegNotFound(String),

    #[error("Processing failed for {path}: {reason}")]
    ProcessingFailed { path: PathBuf, reason: String },

    #[error(
        "Refusing to embed chapters for {path}: {count} chapter(s) start at or past the {playlength} playlength ({details})"
    )]
    ChaptersOutOfRange { path: PathBuf, count: usize, playlength: String, details: String },

    #[error("Could not determine duration of {path}; refusing to tag chapters: {source}")]
    ProbeFailed {
        path: PathBuf,
        #[source]
        source: FFmpegError,
    },

    #[error("Cleanup failed: {0}")]
    CleanupFailed(String),

    #[error("Group processing failed: {group}")]
    GroupProcessingFailed { group: String, source: Box<dyn std::error::Error + Send + Sync> },

    #[error("Partial processing complete: {detail}")]
    PartialFailure { results: Vec<ProcessingResult>, detail: String },
}

/// Result type for processor operations
pub type Result<T> = std::result::Result<T, ProcessorError>;

/// Processing result for a single audiobook
#[derive(Debug, Clone)]
pub struct ProcessingResult {
    /// Input files that were processed
    pub input_files: Vec<PathBuf>,
    /// Output file path
    pub output_file: PathBuf,
    /// Whether metadata was applied from API
    pub metadata_applied: bool,
    /// Whether source files were moved to completed directory
    pub files_moved: bool,
}

/// Progress information during processing
#[derive(Debug, Clone)]
pub struct ProcessingProgress {
    /// Current stage
    pub stage: ProcessingStage,
    /// Current file being processed (if applicable)
    pub current_file: Option<PathBuf>,
    /// Total files to process
    pub total_files: usize,
    /// Files processed so far
    pub completed_files: usize,
    /// Current message
    pub message: String,
}

/// Processing stages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingStage {
    Discovery,
    ApiLookup,
    Merging,
    Tagging,
    MovingFiles,
    Complete,
    Error,
}

impl std::fmt::Display for ProcessingStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessingStage::Discovery => write!(f, "Discovery"),
            ProcessingStage::ApiLookup => write!(f, "API Lookup"),
            ProcessingStage::Merging => write!(f, "Merging"),
            ProcessingStage::Tagging => write!(f, "Tagging"),
            ProcessingStage::MovingFiles => write!(f, "Moving Files"),
            ProcessingStage::Complete => write!(f, "Complete"),
            ProcessingStage::Error => write!(f, "Error"),
        }
    }
}

/// Trait for handling processing progress callbacks
pub trait ProgressHandler: Send + Sync {
    /// Called when progress is updated
    fn on_progress(&self, progress: ProcessingProgress);
}

/// Simple progress handler that logs progress
pub struct LoggingProgressHandler;

impl ProgressHandler for LoggingProgressHandler {
    fn on_progress(&self, progress: ProcessingProgress) {
        info!(
            "[{}] {}/{} - {}",
            progress.stage, progress.completed_files, progress.total_files, progress.message
        );
    }
}

/// No-op progress handler
pub struct NoOpProgressHandler;

impl ProgressHandler for NoOpProgressHandler {
    fn on_progress(&self, _progress: ProcessingProgress) {}
}

/// Main processor that orchestrates all operations
pub struct Processor {
    config: Config,
    ffmpeg: Arc<FFmpeg>,
    api_client: Option<MetadataSource>,
    merger: Merger,
    tagger: Tagger,
    /// Structured job input (authoritative metadata base, overrides, and
    /// per-field write policies); `None` for the legacy standalone CLI.
    job: Option<crate::job::Job>,
    /// Test-only substitute for [`Tagger::embed_chapters`], so the
    /// chapter-embed failure path can be exercised without a real file.
    #[cfg(test)]
    embed_hook: Option<
        Arc<
            dyn Fn(&std::path::Path, &[crate::metadata::Chapter]) -> crate::tagging::Result<()>
                + Send
                + Sync,
        >,
    >,
}

impl Processor {
    /// Create a new Processor with the given configuration
    pub fn new(config: Config) -> Result<Self> {
        // Discover FFmpeg
        let ffmpeg = Arc::new(
            FFmpeg::discover().map_err(|e| ProcessorError::FFmpegNotFound(e.to_string()))?,
        );

        // Create API client based on configured metadata source
        let api_client = match MetadataSource::new_with_region(
            config.metadata_source,
            config.api_url.as_deref(),
            config.region,
        ) {
            Ok(src) => Some(src),
            Err(e) => {
                warn!("Failed to create metadata source: {}", e);
                eprintln!(
                    "Warning: failed to create metadata source ({}); metadata lookup will be disabled.",
                    e
                );
                None
            }
        };

        let merger = Merger::new(ffmpeg.clone());
        let tagger = Tagger::new();
        #[cfg(test)]
        let embed_hook = None;

        Ok(Self {
            config,
            ffmpeg,
            api_client,
            merger,
            tagger,
            job: None,
            #[cfg(test)]
            embed_hook,
        })
    }

    /// Create a processor for a structured job file.
    ///
    /// The job supplies its own metadata base and output; its inputs drive
    /// discovery. The CLI's `--metadata-source`/`--api-url`/`--region` flags
    /// are ignored in favor of the job's own `source` selection (provider
    /// mode) or the absence of a client (supplied mode), so a supplied job
    /// runs fully offline.
    pub fn for_job(job: crate::job::Job, config: Config) -> Result<Self> {
        // Only a provider job constructs a metadata client. A supplied job is
        // fully offline: a `source` object, if present, is irrelevant and must
        // not build a client.
        let source = match (&job.input_provider, job.source.as_ref()) {
            (Some(_), Some(src)) => Some(
                MetadataSource::new_with_region(
                    src.source,
                    src.api_url.as_deref(),
                    src.region.unwrap_or(crate::api::MetadataRegion::Us),
                )
                .map_err(|e| ProcessorError::Job(crate::job::JobError::Metadata(e)))?,
            ),
            (Some(_), None) => Some(
                MetadataSource::new_with_region(
                    config.metadata_source,
                    config.api_url.as_deref(),
                    config.region,
                )
                .map_err(|e| ProcessorError::Job(crate::job::JobError::Metadata(e)))?,
            ),
            // Supplied input never needs a client, regardless of `source`.
            (None, _) => None,
        };

        let ffmpeg = Arc::new(
            FFmpeg::discover().map_err(|e| ProcessorError::FFmpegNotFound(e.to_string()))?,
        );
        let merger = Merger::new(ffmpeg.clone());
        let tagger = Tagger::new();
        #[cfg(test)]
        let embed_hook = None;

        Ok(Self {
            config,
            ffmpeg,
            api_client: source,
            merger,
            tagger,
            job: Some(job),
            #[cfg(test)]
            embed_hook,
        })
    }

    /// Create a new Processor with explicit components (for testing)
    #[allow(dead_code)]
    pub fn with_components(
        config: Config,
        ffmpeg: Arc<FFmpeg>,
        api_client: Option<MetadataSource>,
    ) -> Self {
        let merger = Merger::new(ffmpeg.clone());
        let tagger = Tagger::new();
        #[cfg(test)]
        let embed_hook = None;

        Self {
            config,
            ffmpeg,
            api_client,
            job: None,
            merger,
            tagger,
            #[cfg(test)]
            embed_hook,
        }
    }

    /// Process all input paths
    pub async fn process(&self, inputs: Vec<PathBuf>) -> Result<Vec<ProcessingResult>> {
        self.process_with_progress(inputs, &NoOpProgressHandler).await
    }

    /// Process all input paths with progress reporting
    pub async fn process_with_progress(
        &self,
        inputs: Vec<PathBuf>,
        progress_handler: &dyn ProgressHandler,
    ) -> Result<Vec<ProcessingResult>> {
        if inputs.is_empty() {
            return Err(ProcessorError::NoInputs);
        }

        // A structured job is one book. Resolve it exactly once, before
        // discovery and before the output root is created: an unresolvable
        // provider id (or a disabled client) fails before any directory,
        // merge, or source move. In provider mode this is the only catalog
        // fetch of the run; in supplied mode no client exists at all. Dry-run
        // never resolves, so it stays network-free.
        let resolved_job: Option<crate::job::EffectiveJob> = match &self.job {
            Some(job) if !self.config.dry_run => Some(job.resolve(self.api_client.as_ref()).await?),
            _ => None,
        };
        if resolved_job.is_some() {
            info!(
                "Job resolved; processing '{}' as one book",
                self.job.as_ref().unwrap().output_root.display()
            );
        }

        // Stage 1: Discovery
        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Discovery,
            current_file: None,
            total_files: inputs.len(),
            completed_files: 0,
            message: "Discovering audio files...".to_string(),
        });

        let job_mode = self.job.is_some();
        let groups = if job_mode {
            Self::group_job_inputs(&inputs, self.config.dry_run)?
        } else {
            if !self.config.dry_run {
                if let Some(output) = &self.config.output {
                    std::fs::create_dir_all(output)
                        .map_err(|e| ProcessorError::OutputDirCreation(e.to_string()))?;
                }
            }
            // Dry-run is tolerant of discovery errors: they become an empty
            // group list (and a warning) rather than failing the run.
            match crate::discovery::discover_and_group(&inputs) {
                Ok(groups) => groups,
                Err(e) if self.config.dry_run => {
                    warn!("Discovery error in dry-run mode: {e}");
                    Vec::new()
                }
                Err(e) => return Err(e.into()),
            }
        };
        info!("Discovered {} audio groups to process", groups.len());

        if groups.is_empty() {
            if self.config.dry_run {
                println!("Dry run: No audio files discovered in input paths. Inputs: {:?}", inputs);
                return Ok(Vec::new());
            }
            return Err(ProcessorError::NoInputs);
        }

        // Best-effort: each group is processed independently
        let mut results = Vec::new();
        let mut failed_groups: Vec<(String, String)> = Vec::new();
        let total_groups = groups.len();

        for (idx, group) in groups.iter().enumerate() {
            progress_handler.on_progress(ProcessingProgress {
                stage: ProcessingStage::Merging,
                current_file: Some(group.files[0].path.clone()),
                total_files: total_groups,
                completed_files: idx,
                message: format!("Processing group '{}'...", group.name),
            });

            match self.process_group(group, progress_handler, resolved_job.as_ref()).await {
                Ok(result) => {
                    results.push(result);
                }
                Err(e) => {
                    warn!("Group '{}' failed: {}", group.name, e);
                    failed_groups.push((group.name.clone(), e.to_string()));
                    progress_handler.on_progress(ProcessingProgress {
                        stage: ProcessingStage::Error,
                        current_file: Some(group.files[0].path.clone()),
                        total_files: total_groups,
                        completed_files: idx,
                        message: format!("Failed to process group '{}': {}", group.name, e),
                    });
                    continue;
                }
            }
        }

        if results.is_empty() {
            return Err(ProcessorError::GroupProcessingFailed {
                group: failed_groups.first().map(|(g, _)| g.clone()).unwrap_or_default(),
                source: Box::new(std::io::Error::other("All groups failed to process")),
            });
        }

        if !failed_groups.is_empty() {
            let detail = format!(
                "{} of {} groups succeeded; failed: {}",
                results.len(),
                total_groups,
                failed_groups
                    .iter()
                    .map(|(g, e)| format!("{}: {}", g, e))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            return Err(ProcessorError::PartialFailure { results, detail });
        }

        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Complete,
            current_file: None,
            total_files: total_groups,
            completed_files: results.len(),
            message: format!("Completed processing {} audiobooks", results.len()),
        });

        Ok(results)
    }

    /// Process a single audio group (audiobook)
    async fn process_group(
        &self,
        group: &AudioGroup,
        progress_handler: &dyn ProgressHandler,
        resolved_job: Option<&crate::job::EffectiveJob>,
    ) -> Result<ProcessingResult> {
        if let Some(job) = &self.job {
            return self.process_group_job(group, job, resolved_job, progress_handler).await;
        }

        let input_paths: Vec<PathBuf> = group.files.iter().map(|f| f.path.clone()).collect();

        debug!("Processing group '{}' with {} files", group.name, input_paths.len());

        // Stage 2: API Lookup (optional - only if metadata ID is provided or can be inferred)
        // Skipped in dry-run mode to avoid unnecessary network calls
        let mut metadata = if self.config.dry_run {
            None
        } else if let Some(client) = &self.api_client {
            let candidate_ids = self.extract_metadata_ids(group);

            // An explicit --metadata-id wins; otherwise try every bracketed
            // candidate in name order. A descriptive marker such as
            // `[Unabridged]` is ASIN-shaped, so a failure falls through to the
            // next token instead of dropping a real ASIN listed after it.
            let ids: Vec<&str> = match self.config.metadata_id.as_deref() {
                Some(id) => vec![id],
                None => candidate_ids.iter().map(String::as_str).collect(),
            };

            let mut metadata = None;
            for (index, id) in ids.iter().enumerate() {
                progress_handler.on_progress(ProcessingProgress {
                    stage: ProcessingStage::ApiLookup,
                    current_file: Some(input_paths[0].clone()),
                    total_files: 1,
                    completed_files: 0,
                    message: format!("Fetching metadata for ID: {}...", id),
                });

                match client.fetch_book(id).await {
                    Ok(book_metadata) => {
                        info!("Successfully fetched metadata for: {}", book_metadata.title);
                        metadata = Some(book_metadata);
                        break;
                    }
                    Err(e) => {
                        if index + 1 < ids.len() {
                            debug!(
                                "Metadata ID '{}' did not resolve ({}); trying next candidate",
                                id, e
                            );
                        } else {
                            warn!("Failed to fetch metadata for ID {}: {}", id, e);
                        }
                    }
                }
            }
            metadata
        } else {
            None
        };

        // Determine output path (after API lookup so we have metadata for path formatting)
        let output_path = self.determine_output_path(&group.files, metadata.as_ref())?;
        debug!("Output path: {}", output_path.display());

        // Dry-run: print what would happen without doing it
        if self.config.dry_run {
            info!(
                "DRY RUN: Would merge {} file(s) into: {}",
                group.files.len(),
                output_path.display()
            );

            // Show what input files would be merged
            for file in &group.files {
                info!("DRY RUN:   - {}", file.path.display());
            }

            // Show what metadata operations would happen
            if metadata.is_some() {
                info!("DRY RUN: Would apply metadata tags");
            }

            // Show what file move would happen
            if let Some(completed_dir) = &self.config.completed_directory {
                info!("DRY RUN: Would move source files to: {}", completed_dir.display());
            }

            return Ok(ProcessingResult {
                input_files: input_paths,
                output_file: output_path,
                metadata_applied: false,
                files_moved: false,
            });
        }

        // Resolve chapters: provider chapters when present, otherwise chapters read
        // from the inputs, and finally one generated chapter per input track for
        // chapterless groups so the merged output always carries chapter markers.
        let chapters = if let Some(meta) = metadata.as_ref().filter(|m| !m.chapters.is_empty()) {
            meta.chapters.clone()
        } else {
            let resolved = self.input_chapters(group);
            // Keep provider metadata in sync so chapters.txt matches the embedding.
            if let Some(meta) = metadata.as_mut() {
                meta.chapters.clone_from(&resolved);
            }
            resolved
        };

        // Stage 3: Merge
        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Merging,
            current_file: Some(input_paths[0].clone()),
            total_files: 1,
            completed_files: 0,
            message: "Merging audio files...".to_string(),
        });

        let merge_job = MergeJob::new(group.files.clone(), output_path.clone())
            .with_threads(self.config.num_cpus);
        let merged_path = self.merger.merge(&merge_job).map_err(ProcessorError::Merge)?;

        info!("Successfully merged to: {}", merged_path.display());

        // Reject chapters that start at or beyond the merged file's duration.
        // mp4ameta writes start times verbatim, so embedding them would create
        // unplayable entries; dropping them would silently shorten the chapter
        // list, so fail the book and name every offending chapter. The merged
        // output is removed first: a failed book must not look complete to
        // anything scanning the output directory.
        if !chapters.is_empty() {
            let duration = match self.ffmpeg.get_duration(&merged_path) {
                Ok(duration) => duration,
                Err(e) => {
                    warn!(
                        "Could not determine duration of {}; refusing to tag chapters: {}",
                        merged_path.display(),
                        e
                    );
                    Self::discard_output(&merged_path);
                    return Err(ProcessorError::ProbeFailed { path: merged_path, source: e });
                }
            };

            if let Some((count, details)) = describe_out_of_range_chapters(&chapters, duration) {
                Self::discard_output(&merged_path);
                return Err(ProcessorError::ChaptersOutOfRange {
                    path: merged_path,
                    count,
                    playlength: format_duration(duration),
                    details,
                });
            }
        }

        // Stage 4: Tagging
        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Tagging,
            current_file: Some(merged_path.clone()),
            total_files: 1,
            completed_files: 0,
            message: "Writing metadata and cover art...".to_string(),
        });

        let metadata_applied = if let Some(meta) = &metadata {
            // Write metadata tags
            if let Err(e) = self.tagger.write_metadata(&merged_path, meta) {
                warn!("Failed to write metadata: {}", e);
                false
            } else {
                // Download and embed cover art
                if let Some(cover_url) = &meta.cover_url {
                    if let Err(e) = self.tagger.embed_cover(&merged_path, cover_url).await {
                        warn!("Failed to embed cover art: {}", e);
                    }
                }

                true
            }
        } else {
            false
        };

        // Embed chapters into the M4B file. These are provider chapters, chapters
        // read from the inputs, or chapters generated from the input tracks, so
        // they are applied even when no provider metadata was available.
        if !chapters.is_empty() {
            #[cfg(test)]
            let embed_result = match &self.embed_hook {
                Some(hook) => hook(&merged_path, &chapters),
                None => self.tagger.embed_chapters(&merged_path, &chapters),
            };
            #[cfg(not(test))]
            let embed_result = self.tagger.embed_chapters(&merged_path, &chapters);
            if let Err(e) = embed_result {
                // The pre-merge guard probes the ffprobe duration, but mp4ameta
                // validates chapters against the container's mvhd duration, which
                // can be shorter. A failed embed would otherwise ship a
                // "successful" book with no chapters, so discard the merged
                // output and fail the book.
                Self::discard_output(&merged_path);
                return Err(ProcessorError::Tagging(e));
            } else {
                info!("Successfully embedded {} chapters", chapters.len());
            }
        }

        // Write chapters.txt next to the output file. Generated chapters have no
        // provider metadata to source headers from, so fall back to the output
        // stem as the album name.
        let chapters_txt_path = merged_path.with_extension("chapters.txt");
        if let Some(meta) = &metadata {
            if let Err(e) = self.tagger.write_chapters_txt(&chapters_txt_path, meta) {
                warn!("Failed to write chapters.txt: {}", e);
            }
        } else if !chapters.is_empty() {
            let album = merged_path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Err(e) =
                self.tagger.write_chapters_txt_with(&chapters_txt_path, &album, None, &chapters)
            {
                warn!("Failed to write chapters.txt: {}", e);
            }
        }

        // Stage 5: Move source files
        let files_moved = if let Some(completed_dir) = &self.config.completed_directory {
            progress_handler.on_progress(ProcessingProgress {
                stage: ProcessingStage::MovingFiles,
                current_file: Some(input_paths[0].clone()),
                total_files: input_paths.len(),
                completed_files: 0,
                message: "Moving source files to completed directory...".to_string(),
            });

            match self.tagger.move_completed_files(&input_paths, completed_dir) {
                Ok(moved) => {
                    info!("Moved {} files to completed directory", moved.len());
                    true
                }
                Err(e) => {
                    warn!("Failed to move source files: {}", e);
                    false
                }
            }
        } else {
            false
        };

        Ok(ProcessingResult {
            input_files: input_paths,
            output_file: merged_path,
            metadata_applied,
            files_moved,
        })
    }

    /// Process one audio group under a structured job.
    ///
    /// Differs from the legacy path deliberately:
    /// - Metadata comes from the job's base + overrides. Resolution failure
    ///   (unknown id, release the book does not carry, missing client) fails
    ///   the group before any merge or source move.
    /// - Tag writes follow per-field policies; a `write` without a value and
    ///   an unresolvable preserved title/subtitle component are group
    ///   failures, not warnings.
    /// - `preserve` chapters on a multi-file group is refused: a merge cannot
    ///   byte-preserve per-file chapters, so the strict promise is only valid
    ///   for a single input.
    /// - The source files move only after every write succeeded.
    async fn process_group_job(
        &self,
        group: &AudioGroup,
        job: &crate::job::Job,
        resolved_job: Option<&crate::job::EffectiveJob>,
        progress_handler: &dyn ProgressHandler,
    ) -> Result<ProcessingResult> {
        let input_paths: Vec<PathBuf> = group.files.iter().map(|f| f.path.clone()).collect();
        let is_single_input = group.files.len() == 1;

        // Output path from the job's root and template. Resolving it needs no
        // metadata in dry-run; naming uses the effective metadata below.
        if self.config.dry_run {
            info!(
                "DRY RUN (job): would process {} file(s) from '{}'",
                group.files.len(),
                group.name
            );
            return Ok(ProcessingResult {
                input_files: input_paths,
                output_file: PathBuf::new(),
                metadata_applied: false,
                files_moved: false,
            });
        }

        // The job was resolved exactly once in `process_with_progress`, before
        // discovery and output creation; that EffectiveJob is authoritative
        // here. No catalog touch happens in this function. Dry-run returned
        // above, so the job is resolved by this point.
        let effective = resolved_job.expect("job resolved before the dry-run return");

        // Output path from the job's root and template.
        let output_path = self.determine_job_output_path(&job.output_root, effective)?;

        // PR1 baseline for chapters (the documented default): resolved
        // chapters (provider → input → generated per-track), embedded, plus
        // the `chapters.txt` sidecar. A per-field chapter policy arrives in a
        // follow-up; PR1 has no `chapters` job field. Range validation is
        // owned by `Tagger::embed_chapters` against the merged container's
        // actual duration: a chapter-derived bound would reject valid
        // zero-length final markers, and it would not catch a chapter past the
        // real playlength.
        let mut chapters = effective.metadata.chapters.clone();
        if chapters.is_empty() {
            chapters = self.input_chapters(group);
        }
        // Normalize once by start time so the embedded chapters and the
        // `chapters.txt` sidecar list the same order; `embed_chapters` sorts a
        // copy internally, but the sidecar writer preserves input order.
        chapters.sort_by_key(|c| c.start_time);

        // Stage 3: Merge
        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Merging,
            current_file: Some(input_paths[0].clone()),
            total_files: 1,
            completed_files: 0,
            message: "Merging audio files...".to_string(),
        });

        let merge_job = MergeJob::new(group.files.clone(), output_path.clone())
            .with_threads(self.config.num_cpus);
        let merged_path = self.merger.merge(&merge_job).map_err(ProcessorError::Merge)?;
        info!("Successfully merged to: {}", merged_path.display());

        // Stage 4: Policy-driven writes. Any failure here discards the merged
        // output and fails the group before the sources move.
        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Tagging,
            current_file: Some(merged_path.clone()),
            total_files: 1,
            completed_files: 0,
            message: "Applying write policies...".to_string(),
        });

        // Tag writes: the documented baseline — every supported scalar field
        // is written from the effective metadata (the legacy all-fields
        // behavior). Per-field preserve/write/clear policies arrive in a
        // follow-up.
        self.tagger.write_metadata(&merged_path, &effective.metadata).map_err(|e| {
            Self::discard_output(&merged_path);
            ProcessorError::Tagging(e)
        })?;

        // Cover: PR1 keeps the legacy provider-URL behavior (a supplied job
        // carries no cover URL, so nothing is downloaded). A cover policy
        // (preserve / local file / URL / clear) arrives in a follow-up.
        if let Some(cover_url) = effective.metadata.cover_url.clone() {
            if is_single_input {
                let source_sidecar = input_paths[0].with_extension("cover.txt");
                if source_sidecar.is_file() {
                    let dest = merged_path.with_extension("cover.txt");
                    if let Err(e) = std::fs::copy(&source_sidecar, &dest) {
                        warn!("Failed to carry source cover sidecar: {e}");
                    }
                }
            }
            if let Err(e) = self.tagger.embed_cover(&merged_path, &cover_url).await {
                Self::discard_output(&merged_path);
                return Err(ProcessorError::Tagging(e));
            }
        }

        // Chapters: PR1 baseline — embed the resolved chapters and write the
        // sidecar (the documented default). A chapter policy (preserve /
        // explicit write / clear) arrives in a follow-up. The sidecar is
        // written from the same resolved vector that was embedded — never from
        // `effective.metadata.chapters`, which stays empty whenever the
        // chapters came from the inputs (always for supplied jobs).
        if !chapters.is_empty() {
            if let Err(e) = self.tagger.embed_chapters(&merged_path, &chapters) {
                Self::discard_output(&merged_path);
                return Err(ProcessorError::Tagging(e));
            }
            let artist = if effective.metadata.authors.is_empty() {
                None
            } else {
                Some(effective.metadata.authors.join(", "))
            };
            let sidecar = merged_path.with_extension("chapters.txt");
            if let Err(e) = self.tagger.write_chapters_txt_with(
                &sidecar,
                &effective.metadata.title,
                artist.as_deref(),
                &chapters,
            ) {
                warn!("Failed to write chapters.txt: {e}");
            }
        }

        // Stage 5: Move source files only after every write succeeded.
        let completed_dir =
            effective.completed_directory.as_ref().or(self.config.completed_directory.as_ref());
        let files_moved = if let Some(completed_dir) = completed_dir {
            progress_handler.on_progress(ProcessingProgress {
                stage: ProcessingStage::MovingFiles,
                current_file: Some(input_paths[0].clone()),
                total_files: 1,
                completed_files: 0,
                message: "Moving source files to completed directory...".to_string(),
            });
            match self.tagger.move_completed_files(&input_paths, completed_dir) {
                Ok(moved) => {
                    info!("Moved {} files to completed directory", moved.len());
                    true
                }
                Err(e) => {
                    warn!("Failed to move source files: {e}");
                    false
                }
            }
        } else {
            false
        };

        Ok(ProcessingResult {
            input_files: input_paths,
            output_file: merged_path,
            metadata_applied: true,
            files_moved,
        })
    }

    /// Output path for a job: the job's root plus its template (default
    /// `{author}/{title}`), sanitized per component like the legacy path.
    fn determine_job_output_path(
        &self,
        output_root: &std::path::Path,
        effective: &crate::job::EffectiveJob,
    ) -> Result<PathBuf> {
        let template = if effective.path_format.is_empty() {
            "{author}/{title}".to_string()
        } else {
            effective.path_format.clone()
        };
        let formatted = self.format_path(&template, &effective.metadata);

        let path_components: Vec<String> = formatted
            .split('/')
            .map(sanitize_filename::sanitize)
            .filter(|s| !s.is_empty())
            .collect();

        let mut output_path = output_root.to_path_buf();
        for component in &path_components[..path_components.len().saturating_sub(1)] {
            output_path = output_path.join(component);
        }
        let filename = path_components
            .last()
            .map(|s| format!("{}.m4b", s))
            .unwrap_or_else(|| "output.m4b".to_string());
        Ok(output_path.join(filename))
    }

    /// Remove a merged output that failed post-merge validation.
    ///
    /// The output keeps its final name once merged, so leaving it behind makes a
    /// rejected book look like a finished one. Source files are never touched.
    fn discard_output(path: &std::path::Path) {
        if let Err(e) = std::fs::remove_file(path) {
            warn!("Failed to remove rejected output {}: {}", path.display(), e);
        }
    }

    /// Discover a job's inputs as a single group.
    ///
    /// A structured job is one book, so every discovered file is combined into
    /// one group in the discovery's stable natural (numeric) order; the group
    /// is never split by parent directory into multiple groups that would
    /// share a single output path. Empty discovery is a dry-run no-op or a
    /// `NoInputs` error.
    fn group_job_inputs(inputs: &[PathBuf], dry_run: bool) -> Result<Vec<AudioGroup>> {
        let files = crate::discovery::discover_files(inputs)?;
        if files.is_empty() {
            if dry_run {
                println!("Dry run: No audio files discovered in input paths. Inputs: {:?}", inputs);
                return Ok(Vec::new());
            }
            return Err(ProcessorError::NoInputs);
        }
        Ok(vec![AudioGroup { name: "job".to_string(), files, disc_number: None }])
    }

    /// Resolve chapters for a group from its input files.
    ///
    /// Prefers chapters embedded in the first input, which keeps a single M4B's
    /// own chapters. When the group carries no chapters at all — the usual case
    /// for split MP3s — a chapter is generated per input track, matching the old
    /// m4b-tool `ChaptersFromFileTracks` behavior, so the merged M4B still has
    /// chapter markers. Reading chapters from every input is tracked separately.
    fn input_chapters(&self, group: &AudioGroup) -> Vec<crate::metadata::Chapter> {
        if let Some(first) = group.files.first() {
            match crate::chapters::read_chapters(&first.path, self.ffmpeg.ffprobe_path().to_str()) {
                Ok(file_chapters) if !file_chapters.is_empty() => {
                    info!("Read {} chapters from input file", file_chapters.len());
                    return file_chapters
                        .into_iter()
                        .map(|ch| {
                            crate::metadata::Chapter::new(
                                ch.title,
                                std::time::Duration::from_millis(ch.start_time),
                                std::time::Duration::from_millis(ch.duration),
                            )
                        })
                        .collect();
                }
                Ok(_) => {}
                Err(e) => {
                    debug!("No chapters read from {}: {}", first.path.display(), e);
                }
            }
        }

        // Every track duration is needed: a missing one would shift all later
        // chapter start times, so skip generation instead of emitting wrong times.
        let mut durations = Vec::with_capacity(group.files.len());
        for file in &group.files {
            match self.ffmpeg.get_duration(&file.path) {
                Ok(duration) => durations.push(duration.as_millis() as u64),
                Err(e) => {
                    warn!(
                        "Cannot generate chapters, failed to read duration of {}: {}",
                        file.path.display(),
                        e
                    );
                    return Vec::new();
                }
            }
        }

        let generated = crate::chapters::generate_chapters_from_durations(&durations);
        if !generated.is_empty() {
            info!("Generated {} chapters from {} input tracks", generated.len(), durations.len());
        }

        generated
            .into_iter()
            .map(|ch| {
                crate::metadata::Chapter::new(
                    ch.title,
                    std::time::Duration::from_millis(ch.start_time),
                    std::time::Duration::from_millis(ch.duration),
                )
            })
            .collect()
    }

    /// Determine output path for a group of files
    fn determine_output_path(
        &self,
        files: &[AudioFile],
        metadata: Option<&BookMetadata>,
    ) -> Result<PathBuf> {
        let output_dir = self.config.output.clone().ok_or(ProcessorError::NoOutputDir)?;

        // Get the first file for fallback
        let first_file = files.first().ok_or_else(|| ProcessorError::NoInputs)?;

        // Format the path based on metadata and path_format template
        let formatted_path = if let Some(meta) = metadata {
            self.format_path(&self.config.path_format, meta)
        } else {
            // Fallback: use parent directory name
            first_file
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "output".to_string())
        };

        // Split by '/' to create subdirectories and sanitize each component
        let path_components: Vec<String> = formatted_path
            .split('/')
            .map(sanitize_filename::sanitize)
            .filter(|s| !s.is_empty())
            .collect();

        // Join components with output directory
        let mut output_path = output_dir;
        for component in &path_components[..path_components.len().saturating_sub(1)] {
            output_path = output_path.join(component);
        }

        // Add the filename with .m4b extension
        let filename = path_components
            .last()
            .map(|s| format!("{}.m4b", s))
            .unwrap_or_else(|| "output.m4b".to_string());
        output_path = output_path.join(filename);

        Ok(output_path)
    }

    /// Format path using template with metadata replacement
    fn format_path(&self, template: &str, metadata: &BookMetadata) -> String {
        let mut result = template.to_string();

        // Replace {author} with first author (trimmed)
        let author = metadata.authors.first().map(|s| s.trim()).unwrap_or("Unknown");
        result = result.replace("{author}", author);

        // Replace {authors} with all authors trimmed, whitespace-only filtered, joined by ", "
        let authors: Vec<&str> =
            metadata.authors.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        let authors_value =
            if authors.is_empty() { "Unknown".to_string() } else { authors.join(", ") };
        result = result.replace("{authors}", &authors_value);

        // Replace {narrator} with first narrator (trimmed)
        let narrator = metadata.narrators.first().map(|s| s.trim()).unwrap_or("Unknown");
        result = result.replace("{narrator}", narrator);

        // Replace {narrators} with all narrators trimmed, whitespace-only filtered, joined by ", "
        let narrators: Vec<&str> =
            metadata.narrators.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        let narrators_value =
            if narrators.is_empty() { "Unknown".to_string() } else { narrators.join(", ") };
        result = result.replace("{narrators}", &narrators_value);

        // Replace {title} (trimmed)
        result = result.replace("{title}", metadata.title.trim());

        // Replace {genre} with first genre, or remove if none (trimmed)
        if let Some(genre) = &metadata.genres.first().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            result = result.replace("{genre}", genre);
        } else {
            result = result.replace("{genre}", "");
        }

        // Replace {subtitle} or remove if none (trimmed)
        if let Some(subtitle) = &metadata.subtitle {
            result = result.replace("{subtitle}", subtitle.trim());
        } else {
            result = result.replace("{subtitle}", "");
        }

        // Replace {series_name} or remove if none (trimmed)
        if let Some(series) = &metadata.series_name {
            result = result.replace("{series_name}", series.trim());
        } else {
            result = result.replace("{series_name}", "");
        }

        // Replace {series_position} or remove if none (trimmed)
        if let Some(pos) = &metadata.series_position {
            result = result.replace("{series_position}", pos.trim());
        } else {
            result = result.replace("{series_position}", "");
        }

        // Replace {year} or remove if none (trimmed)
        if let Some(year) = metadata.year {
            result = result.replace("{year}", year.to_string().trim());
        } else {
            result = result.replace("{year}", "");
        }

        // Replace {metadata_id} (the looked-up ASIN/AudiobookDB id)
        result = result.replace("{metadata_id}", &metadata.metadata_id);

        // Clean up: remove empty path segments and normalize slashes
        result = result
            .split('/')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");

        result
    }

    /// Extract metadata ID candidates from the audio group name (folder name),
    /// in the order they appear. Every bracketed token of an accepted shape is
    /// returned: the regex cannot distinguish a descriptive marker such as
    /// `[Unabridged]` (10 alphanumeric characters) from an ASIN, so the caller
    /// tries them until one resolves.
    fn extract_metadata_ids(&self, group: &AudioGroup) -> Vec<String> {
        let re = get_metadata_id_regex();
        let mut ids: Vec<String> = Vec::new();
        for captures in re.captures_iter(&group.name) {
            if let Some(capture) = captures.get(1) {
                let id = capture.as_str();
                if !ids.iter().any(|existing| existing == id) {
                    ids.push(id.to_string());
                }
            }
        }
        ids
    }

    /// Get the FFmpeg instance
    pub fn ffmpeg(&self) -> &FFmpeg {
        &self.ffmpeg
    }

    /// Get the API client
    pub fn api_client(&self) -> Option<&MetadataSource> {
        self.api_client.as_ref()
    }

    /// Get the merger
    pub fn merger(&self) -> &Merger {
        &self.merger
    }

    /// Get the tagger
    pub fn tagger(&self) -> &Tagger {
        &self.tagger
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{MetadataRegion, MetadataSourceKind};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use tempfile::TempDir;

    /// Build a test `Config` with the standard dry-run defaults
    /// (AudiobookDB source, no API URL override).
    fn test_config(
        output: Option<PathBuf>,
        completed: Option<PathBuf>,
        dry_run: bool,
        metadata_id: Option<String>,
    ) -> Config {
        Config::new(
            vec![],
            output,
            None,
            MetadataSourceKind::Audiobookdb,
            MetadataRegion::Us,
            completed,
            1,
            "info".to_string(),
            "{author}/{title}".to_string(),
            dry_run,
            metadata_id,
        )
    }

    fn create_test_config(temp_dir: &TempDir) -> Config {
        test_config(
            Some(temp_dir.path().join("output")),
            Some(temp_dir.path().join("completed")),
            false,
            None,
        )
    }

    fn create_test_audio_file(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content).unwrap();
        path
    }

    /// Build a non-dry-run `Config` whose metadata source is a local stub API.
    fn api_test_config(temp_dir: &TempDir, api_url: &str) -> Config {
        Config::new(
            vec![],
            Some(temp_dir.path().join("output")),
            Some(api_url.to_string()),
            MetadataSourceKind::Audiobookdb,
            MetadataRegion::Us,
            Some(temp_dir.path().join("completed")),
            1,
            "info".to_string(),
            "{author}/{title}".to_string(),
            false,
            None,
        )
    }

    /// Create a real M4B of `seconds` duration using FFmpeg.
    fn create_test_m4b(dir: &std::path::Path, seconds: u32) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("audio.m4b");
        let status = std::process::Command::new("ffmpeg")
            .args(["-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-t", &seconds.to_string()])
            .args(["-c:a", "aac", "-b:a", "64k", "-y"])
            .arg(&path)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg failed to create test audio");
        path
    }

    /// Spawn a local AudiobookDB stub.
    ///
    /// Books with id `abc123def456` return a chapter past the audio duration;
    /// every other id returns a single in-range chapter.
    fn spawn_metadata_api() -> String {
        fn respond(stream: &mut std::net::TcpStream, body: serde_json::Value) {
            let body = body.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                }

                let path = request_line.split_whitespace().nth(1).unwrap_or_default();
                let id = path
                    .strip_prefix("/books/")
                    .or_else(|| path.strip_prefix("/releases/rel-"))
                    .and_then(|rest| rest.split(['?', '/']).next());

                let Some(id) = id else { continue };
                let book = || {
                    serde_json::json!({
                        "id": id,
                        "title": format!("Book {id}"),
                        "matchedReleaseId": format!("rel-{id}"),
                        "people": [{"role": {"name": "author"}, "person": {"name": "Test Author"}}],
                    })
                };
                let first = serde_json::json!({
                    "title": "Intro", "ordinal": 1, "startOffsetMs": 0, "lengthMs": 10_000
                });
                let body = if path.starts_with("/books/") {
                    book()
                } else if id == "abc123def456" {
                    serde_json::json!({
                        "id": format!("rel-{id}"),
                        "chapterDetail": {"chapters": [
                            first,
                            {"title": "End Credits", "ordinal": 2, "startOffsetMs": 20_000, "lengthMs": 30_000},
                        ]},
                    })
                } else {
                    serde_json::json!({
                        "id": format!("rel-{id}"),
                        "chapterDetail": {"chapters": [first]},
                    })
                };
                respond(&mut stream, body);
            }
        });

        url
    }

    #[test]
    fn test_processor_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);

        let processor = Processor::new(config);
        assert!(processor.is_ok());
    }

    #[test]
    fn test_determine_output_path() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let processor = Processor::new(config).unwrap();

        // Create test audio file
        let test_dir = temp_dir.path().join("My Audiobook");
        std::fs::create_dir(&test_dir).unwrap();
        let file_path = create_test_audio_file(&temp_dir, "My Audiobook/chapter1.mp3", b"dummy");

        let audio_file = AudioFile::new(file_path).unwrap();
        let output_path = processor.determine_output_path(&[audio_file], None).unwrap();

        assert!(output_path.to_string_lossy().contains("My Audiobook"));
        assert!(output_path.extension().unwrap() == "m4b");
    }

    #[test]
    fn test_format_path_authors_narrators_all_values() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let processor = Processor::new(config).unwrap();

        let meta = BookMetadata::new("TESTID1234", "Title", "desc")
            .add_author(" Alice ")
            .add_author("Bob")
            .add_narrator("N1")
            .add_narrator("N2")
            .add_narrator("N3");

        assert_eq!(
            processor.format_path("{authors} - {narrators}", &meta),
            "Alice, Bob - N1, N2, N3"
        );
    }

    #[test]
    fn test_format_path_authors_narrators_empty_fallback() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let processor = Processor::new(config).unwrap();

        let meta = BookMetadata::new("TESTID1234", "Title", "desc");

        assert_eq!(processor.format_path("{authors}/{narrators}", &meta), "Unknown/Unknown");
    }

    #[test]
    fn test_format_path_authors_narrators_single_backward_compat() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let processor = Processor::new(config).unwrap();

        let meta = BookMetadata::new("TESTID1234", "Title", "desc")
            .add_author("Solo")
            .add_narrator("Narr");

        assert_eq!(
            processor.format_path("{author}/{narrator}/{authors}/{narrators}", &meta),
            "Solo/Narr/Solo/Narr"
        );
    }

    #[test]
    fn test_format_path_authors_whitespace_only_filtered() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let processor = Processor::new(config).unwrap();

        let meta =
            BookMetadata::new("TESTID1234", "Title", "desc").add_author("   ").add_author("Dave");

        assert_eq!(processor.format_path("{authors}", &meta), "Dave");
    }

    #[test]
    fn test_determine_output_path_authors_single_segment() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(
            Some(temp_dir.path().join("output")),
            Some(temp_dir.path().join("completed")),
            false,
            None,
        );
        let mut config = config;
        config.path_format = "{authors}/{title}".to_string();
        let processor = Processor::new(config).unwrap();

        let audio_file_path = create_test_audio_file(&temp_dir, "chapter1.mp3", b"dummy");
        let audio_file = AudioFile::new(audio_file_path).unwrap();

        let meta = BookMetadata::new("TESTID1234", "Title", "desc")
            .add_author("A One")
            .add_author("B Two");

        let output_path = processor
            .determine_output_path(std::slice::from_ref(&audio_file), Some(&meta))
            .unwrap();

        // The joined ", " value must survive sanitize_filename::sanitize as one segment
        assert_eq!(
            output_path.parent().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("A One, B Two"))
        );
        assert_eq!(output_path.file_name(), Some(std::ffi::OsStr::new("Title.m4b")));
    }

    #[test]
    fn test_processing_stage_display() {
        assert_eq!(ProcessingStage::Discovery.to_string(), "Discovery");
        assert_eq!(ProcessingStage::Merging.to_string(), "Merging");
        assert_eq!(ProcessingStage::Complete.to_string(), "Complete");
    }

    #[test]
    fn test_no_inputs_returns_empty_groups() {
        // Empty inputs returns empty groups (not an error at discovery stage)
        let result = crate::discovery::discover_files(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn job_inputs_across_directories_form_one_group_in_natural_order() {
        // A job's inputs are one book: files spread across several parent
        // directories must combine into a single group (never one group per
        // directory sharing a single output path), in stable natural order.
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        std::fs::create_dir_all(root.join("disc1")).unwrap();
        std::fs::create_dir_all(root.join("disc2")).unwrap();
        for name in ["part1.mp3", "part2.mp3"] {
            std::fs::write(root.join("disc1").join(name), b"").unwrap();
        }
        for name in ["part3.mp3", "part10.mp3"] {
            std::fs::write(root.join("disc2").join(name), b"").unwrap();
        }

        let groups = Processor::group_job_inputs(&[root.to_path_buf()], false).unwrap();
        assert_eq!(groups.len(), 1, "a job is one book: exactly one group");
        let files = &groups[0].files;
        assert_eq!(files.len(), 4);
        // Natural (numeric) order: part1, part2, part3, part10.
        let names: Vec<&str> =
            files.iter().map(|f| f.path.file_name().unwrap().to_str().unwrap()).collect();
        assert_eq!(names, ["part1.mp3", "part2.mp3", "part3.mp3", "part10.mp3"]);
    }

    #[test]
    fn job_inputs_empty_list_errors_no_inputs() {
        // An empty input list is a `NoInputs` error (not a discovery success
        // with zero groups), so the job fails before any merge.
        let err = Processor::group_job_inputs(&[], false).unwrap_err();
        assert!(matches!(err, ProcessorError::NoInputs));
    }

    #[tokio::test]
    async fn test_process_empty_inputs() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let processor = Processor::new(config).unwrap();

        let result = processor.process(vec![]).await;
        assert!(matches!(result, Err(ProcessorError::NoInputs)));
    }

    #[tokio::test]
    async fn test_process_group_dry_run_no_metadata() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(Some(temp_dir.path().join("output")), None, true, None);
        let processor = Processor::new(config).unwrap();

        // Create a test group with a dummy file
        let test_dir = temp_dir.path().join("Test Book");
        std::fs::create_dir(&test_dir).unwrap();
        let _file_path = create_test_audio_file(&temp_dir, "Test Book/chapter1.mp3", b"dummy");
        let audio_file = AudioFile::new(test_dir.join("chapter1.mp3")).unwrap();
        let group = AudioGroup {
            name: "Test Book".to_string(),
            files: vec![audio_file],
            disc_number: None,
        };

        let result = processor.process_group(&group, &NoOpProgressHandler, None).await;
        assert!(result.is_ok());
    }

    // Dry-run skips metadata validation/fetching entirely, so an invalid ID
    // (INVALID_ID) does not cause the group to fail; only the dry-run path is exercised.
    #[tokio::test]
    async fn test_process_group_dry_run_with_invalid_id() {
        let temp_dir = TempDir::new().unwrap();
        let config = test_config(
            Some(temp_dir.path().join("output")),
            None,
            true,
            Some("INVALID_ID".to_string()),
        );
        let processor = Processor::new(config).unwrap();

        let test_dir = temp_dir.path().join("Test Book [INVALID_ID]");
        std::fs::create_dir(&test_dir).unwrap();
        let _file_path =
            create_test_audio_file(&temp_dir, "Test Book [INVALID_ID]/chapter1.mp3", b"dummy");
        let audio_file = AudioFile::new(test_dir.join("chapter1.mp3")).unwrap();
        let group = AudioGroup {
            name: "Test Book [INVALID_ID]".to_string(),
            files: vec![audio_file],
            disc_number: None,
        };

        let result = processor.process_group(&group, &NoOpProgressHandler, None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_group_dry_run_output_path_resolution() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("output");
        let config = test_config(Some(output_dir.clone()), None, true, None);
        let processor = Processor::new(config).unwrap();

        // Create two test groups
        for name in &["Book One", "Book Two"] {
            let test_dir = temp_dir.path().join(name);
            std::fs::create_dir(&test_dir).unwrap();
            let _file_path =
                create_test_audio_file(&temp_dir, &format!("{}/audio.mp3", name), b"dummy");
            let audio_file = AudioFile::new(test_dir.join("audio.mp3")).unwrap();
            let group =
                AudioGroup { name: name.to_string(), files: vec![audio_file], disc_number: None };

            let result = processor.process_group(&group, &NoOpProgressHandler, None).await;
            assert!(result.is_ok());
            let result = result.unwrap();
            assert!(result.output_file.starts_with(&output_dir));
            assert_eq!(result.output_file.extension().unwrap(), "m4b");
        }
    }

    // A job dry-run must not panic: the job is not resolved in dry-run (it
    // stays network-free), so the `EffectiveJob` must not be required before
    // the dry-run early return. Regression for a `resolved_job.expect` that
    // fired before that return.
    #[tokio::test]
    async fn job_dry_run_with_input_file_does_not_panic() {
        let temp_dir = TempDir::new().unwrap();
        // An existing input file so discovery produces a group that reaches the
        // job dry-run return.
        let input_path = temp_dir.path().join("book");
        std::fs::create_dir(&input_path).unwrap();
        let _ = create_test_audio_file(&temp_dir, "book/chapter1.mp3", b"dummy");
        let output_dir = temp_dir.path().join("output");

        let job_path = temp_dir.path().join("job.json");
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],
                  "input_supplied": {{ "title": "T", "authors": ["A"] }},
                  "output_root": {:?}, "path_format": "{{author}}/{{title}}" }}"#,
            input_path.join("chapter1.mp3"),
            output_dir
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();

        let config = test_config(Some(output_dir), None, true, None);
        let processor = Processor::for_job(job, config).unwrap();

        let result = processor
            .process(vec![input_path.join("chapter1.mp3")])
            .await
            .expect("a job dry-run must not panic or fail");
        // The dry-run group is reported as processed without touching metadata.
        assert_eq!(result.len(), 1);
        assert!(!result[0].metadata_applied);
    }

    fn test_metadata_with_genres() -> BookMetadata {
        BookMetadata::new("B08XYZ1234", "Test Book", "")
            .add_author("Test Author")
            .add_genre("Mystery")
            .add_genre("Thriller")
    }

    #[test]
    fn test_format_path_new_tokens_populated() {
        let temp_dir = TempDir::new().unwrap();
        let processor = Processor::new(create_test_config(&temp_dir)).unwrap();
        let meta = test_metadata_with_genres();

        let result = processor.format_path("{author}/{genre}/{metadata_id}/{title}", &meta);
        assert_eq!(result, "Test Author/Mystery/B08XYZ1234/Test Book");
    }

    #[test]
    fn test_format_path_first_genre_with_comma() {
        let temp_dir = TempDir::new().unwrap();
        let processor = Processor::new(create_test_config(&temp_dir)).unwrap();
        let meta = BookMetadata::new("B08XYZ1234", "Test Book", "")
            .add_genre("Mystery, Thriller & Suspense");

        let result = processor.format_path("{genre}", &meta);
        assert_eq!(result, "Mystery, Thriller & Suspense");
    }

    #[test]
    fn test_format_path_empty_genres_strips_segment() {
        let temp_dir = TempDir::new().unwrap();
        let processor = Processor::new(create_test_config(&temp_dir)).unwrap();

        let no_genres = BookMetadata::new("B08XYZ1234", "Test Book", "").add_author("Test Author");
        let result = processor.format_path("{author}/{genre}/{metadata_id}/{title}", &no_genres);
        assert_eq!(result, "Test Author/B08XYZ1234/Test Book");

        let whitespace_only = no_genres.clone().add_genre(" ");
        let result =
            processor.format_path("{author}/{genre}/{metadata_id}/{title}", &whitespace_only);
        assert_eq!(result, "Test Author/B08XYZ1234/Test Book");
    }

    #[test]
    fn test_format_path_metadata_id_inline() {
        let temp_dir = TempDir::new().unwrap();
        let processor = Processor::new(create_test_config(&temp_dir)).unwrap();
        let meta = BookMetadata::new("B08XYZ1234", "Test Book", "");

        let result = processor.format_path("{metadata_id}-{title}", &meta);
        assert_eq!(result, "B08XYZ1234-Test Book");
    }

    #[test]
    fn test_determine_output_path_all_unset_tokens() {
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path().join("output");
        let config = Config::new(
            vec![],
            Some(output_dir.clone()),
            None,
            MetadataSourceKind::Audiobookdb,
            MetadataRegion::Us,
            Some(temp_dir.path().join("completed")),
            1,
            "info".to_string(),
            "{genre}/{subtitle}".to_string(),
            false,
            None,
        );
        let processor = Processor::new(config).unwrap();

        let file_path = create_test_audio_file(&temp_dir, "chapter1.mp3", b"dummy");
        let audio_file = AudioFile::new(file_path).unwrap();
        let meta = BookMetadata::new("B08XYZ1234", "Test Book", "");

        let output_path = processor.determine_output_path(&[audio_file], Some(&meta)).unwrap();
        assert_eq!(output_path, output_dir.join("output.m4b"));
    }

    #[test]
    fn test_extract_metadata_ids() {
        let temp_dir = TempDir::new().unwrap();
        let processor = Processor::new(create_test_config(&temp_dir)).unwrap();

        // Exactly two shapes are extracted: 10 alphanumeric (ASIN, any case,
        // mirroring the API clients' validators) and 12 alphanumeric
        // (AudiobookDB internal id). Other bracketed words are not. All
        // accepted matches are returned in name order, because a descriptive
        // marker can be ASIN-shaped and must not mask a later real ID.
        let cases: [(&str, &[&str]); 13] = [
            ("Book [B08XYZ1234]", &["B08XYZ1234"]), // ASIN: 10 uppercase alphanumeric
            ("Book [b08xyz1234]", &["b08xyz1234"]), // lowercase 10-char: accepted like the clients
            ("Book [abc123def456]", &["abc123def456"]), // 12-char AudiobookDB internal id
            // "Unabridged" is 10 ASCII alphanumeric characters, so the
            // case-insensitive 10-char alternative extracts it as a candidate;
            // the lookup then falls through to the real ASIN beside it.
            ("Book [Unabridged]", &["Unabridged"]),
            ("Book [Unabridged] [B08XYZ1234]", &["Unabridged", "B08XYZ1234"]),
            ("Book [B08XYZ1234] [Unabridged]", &["B08XYZ1234", "Unabridged"]),
            ("Book [B08XYZ1234] [b08xyz1234]", &["B08XYZ1234", "b08xyz1234"]),
            ("Book [B08XYZ1234] [B08XYZ1234]", &["B08XYZ1234"]), // duplicates collapse
            ("Book [Complete2024Set]", &[]),                     // 15-char word
            ("Book [ABCDEF12345]", &[]),                         // 11 chars: neither shape
            ("Book [abcdefghijk12]", &[]),                       // 13 chars: neither shape
            ("Book [B08XYZ123]", &[]),                           // 9 chars
            ("Book [ABCD_12345]", &[]),
        ];
        for (name, expected) in cases {
            let group = AudioGroup { name: name.to_string(), files: vec![], disc_number: None };
            assert_eq!(processor.extract_metadata_ids(&group), expected, "name {name:?}");
        }
        let group =
            AudioGroup { name: "Book No Tag".to_string(), files: vec![], disc_number: None };
        assert!(processor.extract_metadata_ids(&group).is_empty());
    }

    #[tokio::test]
    async fn test_process_group_rejects_out_of_range_chapters_before_tagging_or_move() {
        let temp_dir = TempDir::new().unwrap();
        let api_url = spawn_metadata_api();
        let processor = Processor::new(api_test_config(&temp_dir, &api_url)).unwrap();

        let book_dir = temp_dir.path().join("Bad Book [abc123def456]");
        let input = create_test_m4b(&book_dir, 10);
        let group = AudioGroup {
            name: "Bad Book [abc123def456]".to_string(),
            files: vec![AudioFile::new(input.clone()).unwrap()],
            disc_number: None,
        };

        let err = processor.process_group(&group, &NoOpProgressHandler, None).await.unwrap_err();

        match &err {
            ProcessorError::ChaptersOutOfRange { count, playlength, details, .. } => {
                assert_eq!(*count, 1);
                assert_eq!(playlength, "00:00:10.000");
                assert!(details.contains("End Credits"), "{details}");
            }
            other => panic!("expected ChaptersOutOfRange, got {other:?}"),
        }

        // The check runs before tagging, so nothing was tagged or written, and the
        // rejected merge is removed rather than left looking like a finished book.
        let output = temp_dir.path().join("output/Test Author/Book abc123def456.m4b");
        assert!(!output.exists(), "rejected output must not be left behind");
        assert!(!output.with_extension("chapters.txt").exists());

        // And before the source files are moved.
        assert!(input.exists(), "source file must not be moved after a failed check");
        let completed = temp_dir.path().join("completed");
        let moved = std::fs::read_dir(&completed).map(|entries| entries.count()).unwrap_or(0);
        assert_eq!(moved, 0);
    }

    #[tokio::test]
    async fn test_process_group_rejects_out_of_range_source_chapters_without_metadata() {
        let temp_dir = TempDir::new().unwrap();
        // No API lookup: chapters come from the input file itself.
        let processor = Processor::new(create_test_config(&temp_dir)).unwrap();

        let book_dir = temp_dir.path().join("Source Book");
        let input = create_test_m4b(&book_dir, 10);
        let mut tag = mp4ameta::Tag::read_from_path(&input).unwrap();
        tag.chapter_list_mut().extend([
            mp4ameta::Chapter::new(std::time::Duration::ZERO, "Intro"),
            mp4ameta::Chapter::new(std::time::Duration::from_secs(20), "End Credits"),
        ]);
        tag.write_to_path(&input).unwrap();

        let group = AudioGroup {
            name: "Source Book".to_string(),
            files: vec![AudioFile::new(input).unwrap()],
            disc_number: None,
        };

        let err = processor.process_group(&group, &NoOpProgressHandler, None).await.unwrap_err();

        match &err {
            ProcessorError::ChaptersOutOfRange { details, .. } => {
                assert!(details.contains("End Credits"), "{details}");
            }
            other => panic!("expected ChaptersOutOfRange, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_process_group_fails_when_duration_probe_fails() {
        let temp_dir = TempDir::new().unwrap();
        let api_url = spawn_metadata_api();
        let processor = Processor::new(api_test_config(&temp_dir, &api_url)).unwrap();

        // Garbage M4B: the native single-file copy succeeds, but ffprobe cannot
        // read the merged output, so the range check must fail closed.
        let book_dir = temp_dir.path().join("Bad Probe [abc123def456]");
        std::fs::create_dir_all(&book_dir).unwrap();
        let input = book_dir.join("audio.m4b");
        std::fs::write(&input, b"not audio").unwrap();
        let group = AudioGroup {
            name: "Bad Probe [abc123def456]".to_string(),
            files: vec![AudioFile::new(input).unwrap()],
            disc_number: None,
        };

        let err = processor.process_group(&group, &NoOpProgressHandler, None).await.unwrap_err();
        let message = err.to_string();

        match &err {
            ProcessorError::ProbeFailed { path, .. } => {
                assert!(path.to_string_lossy().ends_with("Book abc123def456.m4b"));
            }
            other => panic!("expected ProbeFailed, got {other:?}"),
        }
        assert!(message.contains("refusing to tag chapters"), "{message}");

        // The unprobeable merge must not be left behind either.
        let output = temp_dir.path().join("output/Test Author/Book abc123def456.m4b");
        assert!(!output.exists(), "unprobeable output must not be left behind");
    }

    #[tokio::test]
    async fn test_batch_continues_after_a_group_fails_chapter_validation() {
        let temp_dir = TempDir::new().unwrap();
        let api_url = spawn_metadata_api();
        let processor = Processor::new(api_test_config(&temp_dir, &api_url)).unwrap();

        let bad_dir = temp_dir.path().join("Bad Book [abc123def456]");
        create_test_m4b(&bad_dir, 10);
        let good_dir = temp_dir.path().join("Good Book [xyz987uvw654]");
        create_test_m4b(&good_dir, 10);

        let result =
            processor.process_with_progress(vec![bad_dir, good_dir], &NoOpProgressHandler).await;

        match result {
            Err(ProcessorError::PartialFailure { results, detail }) => {
                assert_eq!(results.len(), 1, "only the valid group should complete");
                assert!(detail.contains("Refusing to embed chapters"), "{detail}");
                let tag = mp4ameta::Tag::read_from_path(&results[0].output_file).unwrap();
                assert_eq!(tag.chapter_list()[0].title, "Intro");
            }
            other => panic!("expected PartialFailure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_process_group_embed_failure_discards_output() {
        let temp_dir = TempDir::new().unwrap();
        let api_url = spawn_metadata_api();
        let config = api_test_config(&temp_dir, &api_url);
        let ffmpeg = Arc::new(FFmpeg::discover().unwrap());

        // Test hook standing in for `Tagger::embed_chapters`: records the path
        // it is handed (which must be the merged output), then fails, so a
        // chapter-embed failure can be tested without a real file.
        let recorded = Arc::new(std::sync::Mutex::new(None));
        let recorded_in_hook = recorded.clone();
        let processor = {
            let mut processor = Processor::with_components(config, ffmpeg, None);
            processor.embed_hook =
                Some(Arc::new(move |file_path: &Path, _: &[crate::metadata::Chapter]| {
                    *recorded_in_hook.lock().unwrap() = Some(file_path.to_path_buf());
                    Err(TaggingError::Mp4Meta("mock chapter embed failure".to_string()))
                }));
            processor
        };

        let book_dir = temp_dir.path().join("Book [xyz987uvw654]");
        let input = create_test_m4b(&book_dir, 10);
        let group = AudioGroup {
            name: "Book [xyz987uvw654]".to_string(),
            files: vec![AudioFile::new(input.clone()).unwrap()],
            disc_number: None,
        };

        // Chapters are generated from the input tracks (no API client), and
        // in range, so the ffprobe guard passes — but the hook makes the
        // actual embed fail.
        let err = processor.process_group(&group, &NoOpProgressHandler, None).await.unwrap_err();

        assert!(
            matches!(err, ProcessorError::Tagging(TaggingError::Mp4Meta(_))),
            "a failed chapter embed must fail the book, got {err:?}",
        );

        // The merged output must be discarded, not shipped as a "successful"
        // book with no chapters.
        let expected = temp_dir.path().join("output/Book [xyz987uvw654].m4b");
        assert_eq!(
            recorded.lock().unwrap().as_ref(),
            Some(&expected),
            "embed must have targeted the merged output"
        );
        assert!(!expected.exists(), "embed failure must not leave the output behind");
        assert!(input.exists(), "source file must not be moved after a failed embed");
    }
}
