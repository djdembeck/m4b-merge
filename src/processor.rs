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

    #[error("Partial processing complete: {detail}")]
    PartialFailure { results: Vec<ProcessingResult>, outcomes: Vec<GroupOutcomes>, detail: String },

    /// Every group failed; per-group diagnostics survive on `outcomes`.
    #[error("Group processing failed: {detail}")]
    AllGroupsFailed { outcomes: Vec<GroupOutcomes>, detail: String },

    #[error("Output path already exists; refusing to overwrite: {path}")]
    OutputCollision { path: PathBuf },
}

/// The full outcome of a run: the succeeded results plus every group's
/// recorded outcome (successful and failed), so a machine result can
/// enumerate all groups without guessing.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub results: Vec<ProcessingResult>,
    pub outcomes: Vec<GroupOutcomes>,
    /// `true` when every group succeeded.
    pub complete: bool,
    /// A human-readable summary of the failures.
    pub detail: String,
}

/// A group's outcome: identity, artifacts, inputs, and the diagnosed
/// error, so nothing is ever undisclosed.
#[derive(Debug, Clone)]
pub struct GroupOutcomes {
    pub group: String,
    /// Every produced output and sidecar, relative to the job's
    /// `output_root`; empty when the group failed before producing anything.
    pub artifacts: Vec<crate::job::Artifact>,
    pub input_files: Vec<PathBuf>,
    pub files_moved: bool,
    pub outcome: crate::job::GroupOutcome,
    /// A diagnosed error when the group failed.
    pub error: Option<String>,
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
    /// Every produced output and sidecar (the merged file and each
    /// `chapters.txt`/`cover.txt` carried or written), absolute.
    pub artifacts: Vec<PathBuf>,
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
        // not build a client. The preview path builds the client the same way.
        let source = job.metadata_source(&config)?;

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
        let run = self.run(inputs, progress_handler).await?;
        if !run.complete {
            if !run.results.is_empty() {
                return Err(ProcessorError::PartialFailure {
                    results: run.results,
                    outcomes: run.outcomes,
                    detail: run.detail,
                });
            }
            return Err(ProcessorError::AllGroupsFailed {
                outcomes: run.outcomes,
                detail: run.detail,
            });
        }
        Ok(run.results)
    }

    /// The full run outcome, the single entry point the `--machine` flow
    /// uses; `process*` wrap it.
    pub async fn run(
        &self,
        inputs: Vec<PathBuf>,
        progress_handler: &dyn ProgressHandler,
    ) -> Result<RunOutcome> {
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
                return Ok(RunOutcome {
                    results: Vec::new(),
                    outcomes: Vec::new(),
                    complete: true,
                    detail: String::new(),
                });
            }
            return Err(ProcessorError::NoInputs);
        }

        // Each group is processed independently and its outcome (artifacts,
        // inputs, files_moved, diagnosed error) is recorded so a machine
        // result never claims a failed group succeeded.
        let mut results = Vec::new();
        let mut failed_groups: Vec<(String, String)> = Vec::new();
        let mut outcomes: Vec<GroupOutcomes> = Vec::new();
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
                    outcomes.push(GroupOutcomes {
                        group: group.name.clone(),
                        artifacts: self.artifacts_for_result(&result),
                        input_files: result.input_files.clone(),
                        files_moved: result.files_moved,
                        outcome: crate::job::GroupOutcome::Succeeded,
                        error: None,
                    });
                    results.push(result);
                }
                Err(e) => {
                    warn!("Group '{}' failed: {}", group.name, e);
                    failed_groups.push((group.name.clone(), e.to_string()));
                    // A rejected merged output is discarded before the group
                    // fails; the diagnosed error carries the truth.
                    outcomes.push(GroupOutcomes {
                        group: group.name.clone(),
                        artifacts: Vec::new(),
                        input_files: group.files.iter().map(|f| f.path.clone()).collect(),
                        files_moved: false,
                        outcome: crate::job::GroupOutcome::Failed,
                        error: Some(e.to_string()),
                    });
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
            let detail = outcomes
                .iter()
                .map(|o| format!("{}: {}", o.group, o.error.as_deref().unwrap_or("unknown error")))
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(RunOutcome { results, outcomes, complete: false, detail });
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
            return Ok(RunOutcome { results, outcomes, complete: false, detail });
        }

        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Complete,
            current_file: None,
            total_files: total_groups,
            completed_files: results.len(),
            message: format!("Completed processing {} audiobooks", results.len()),
        });

        Ok(RunOutcome { results, outcomes, complete: true, detail: String::new() })
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
                artifacts: Vec::new(),
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

        // The legacy pipeline writes the sidecar as a warning-level best
        // effort; report exactly what landed so nothing is silently omitted.
        let mut artifacts = vec![merged_path.clone()];
        let sidecar = merged_path.with_extension("chapters.txt");
        if sidecar.is_file() {
            artifacts.push(sidecar);
        }

        Ok(ProcessingResult {
            input_files: input_paths,
            output_file: merged_path,
            metadata_applied,
            files_moved,
            artifacts,
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
                artifacts: Vec::new(),
            });
        }

        // The job was resolved exactly once in `process_with_progress`, before
        // discovery and output creation; that EffectiveJob is authoritative
        // here. No catalog touch happens in this function. Dry-run returned
        // above, so the job is resolved by this point.
        let effective = resolved_job.expect("job resolved before the dry-run return");

        // Output path from the job's root and template.
        let output_path = self.determine_job_output_path(&job.output_root, effective)?;

        // Strict preservation is only honest for a single input: a multi-input
        // merge/transcode cannot byte-preserve per-file chapters or carry a
        // single source's cover into the merged output, so those combinations
        // are an explicit error, never an alleged success.
        if effective.preserve_chapters_requires_single_input() && !is_single_input {
            return Err(ProcessorError::Job(crate::job::JobError::Invalid(format!(
                "chapters policy 'preserve' requires a single input file; group '{}' has {} — a \
                 merge cannot byte-preserve per-file chapters",
                group.name,
                group.files.len()
            ))));
        }
        if effective.preserve_cover_requires_single_input() && !is_single_input {
            return Err(ProcessorError::Job(crate::job::JobError::Invalid(format!(
                "cover policy 'preserve' requires a single input file; group '{}' has {} — a \
                 merge carries no cover, so a strict cover-preservation promise is impossible",
                group.name,
                group.files.len()
            ))));
        }

        // The chapter list a `write` embeds (and writes to the sidecar): the
        // explicit list when given, otherwise provider → input → generated
        // per-track (the documented default). `preserve`/`clear` resolve to an
        // empty list; their arms handle the file themselves.
        //
        // A resolved list that came out empty is left untouched, not cleared:
        // an empty resolve (e.g. a duration-probe failure while the merge
        // succeeds) must not delete the source's chapters — clearing is a
        // deliberate policy (an explicit empty list, or the `clear` policy),
        // never the result of an empty resolve.
        let mut write_chapters: Vec<crate::metadata::Chapter> = match &effective.chapters {
            crate::job::ChaptersPolicy::Write { explicit } => match explicit {
                Some(list) => list
                    .iter()
                    .map(|ch| {
                        crate::metadata::Chapter::new(
                            ch.title.clone(),
                            std::time::Duration::from_millis(ch.start_millis),
                            std::time::Duration::from_millis(ch.duration_millis),
                        )
                    })
                    .collect(),
                None => {
                    let mut resolved = effective.metadata.chapters.clone();
                    if resolved.is_empty() {
                        resolved = self.input_chapters(group);
                    }
                    resolved
                }
            },
            crate::job::ChaptersPolicy::Preserve | crate::job::ChaptersPolicy::Clear => Vec::new(),
        };
        // Normalize once by start time so the embedded chapters and the
        // `chapters.txt` sidecar list the same order; `embed_chapters` sorts a
        // copy internally, but the sidecar writer preserves input order.
        write_chapters.sort_by_key(|c| c.start_time);
        // Out-of-range validation is left to the post-merge `embed_chapters`
        // write-boundary guard, which checks both explicit and resolved lists
        // against the merged container's authoritative (mvhd) duration and
        // discards the output before the sources move. A pre-merge check here
        // would compare against the provider's chapter-derived
        // `total_duration()`, which is not the audio's real playlength and can
        // wrongly reject a valid zero-length final marker that starts at the
        // end of the provider's chapter list but before the longer merged
        // audio ends.

        // Engine-side no-clobber: claim with `O_CREAT|O_EXCL` immediately
        // before the write; a claim failure leaves the existing file intact.
        self.claim_output_name(&output_path)?;

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
        let merged_path = self.merger.merge(&merge_job).map_err(|e| {
            // A failed merge must not leave the claimed placeholder behind:
            // it would block a retry with a false collision.
            Self::discard_output(&output_path);
            ProcessorError::Merge(e)
        })?;
        // Bind the produced output to the validated root (claim containment
        // is pre-write only; the merge writes by path).
        self.verify_output_in_root(&merged_path)?;
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

        // Tag writes: per-field policies. A `write` without a valid value, or
        // an unresolvable preserved title/subtitle component, is a diagnosed
        // failure before any atom is mutated; `preserve` fields keep the
        // file's existing values.
        self.tagger
            .write_metadata_with_policies(&merged_path, &effective.metadata, &effective.policies)
            .map_err(|e| {
                Self::discard_output(&merged_path);
                ProcessorError::Tagging(e)
            })?;

        // Cover policy across the whole path.
        match &effective.cover {
            crate::job::CoverPolicy::Preserve => {
                // Single-input only (enforced above): the merge is a native
                // copy, so the source cover survives; carry the sidecar if the
                // source had one.
                let source_sidecar = input_paths[0].with_extension("cover.txt");
                if source_sidecar.is_file() {
                    let dest = merged_path.with_extension("cover.txt");
                    if let Err(e) = std::fs::copy(&source_sidecar, &dest) {
                        warn!("Failed to carry source cover sidecar: {e}");
                    }
                }
            }
            crate::job::CoverPolicy::Write(file) => {
                if let Err(e) = self.tagger.embed_cover_local(&merged_path, file, &job.output_root)
                {
                    Self::discard_output(&merged_path);
                    return Err(ProcessorError::Tagging(e));
                }
            }
            crate::job::CoverPolicy::WriteUrl(explicit) => {
                // The default policy: embed the provider cover URL when the
                // resolved metadata carries one; an explicit `write_url`
                // overrides it. A supplied job has no provider URL, so nothing
                // is downloaded.
                let url = explicit.clone().or_else(|| effective.metadata.cover_url.clone());
                if let Some(url) = url {
                    if let Err(e) = self.tagger.embed_cover(&merged_path, &url).await {
                        Self::discard_output(&merged_path);
                        return Err(ProcessorError::Tagging(e));
                    }
                }
            }
            crate::job::CoverPolicy::Clear => {
                // Remove the copied/merged artwork so the clear promise holds.
                if let Err(e) = self.tagger.clear_cover(&merged_path) {
                    Self::discard_output(&merged_path);
                    return Err(ProcessorError::Tagging(e));
                }
            }
        }

        // Chapters across the whole path. `preserve` and `clear` are
        // policy-specific; a `write` embeds the `write_chapters` list resolved
        // above. The sidecar is written from the chapters actually embedded
        // (not the effective metadata, which may lack them in supplied mode),
        // and a failed sidecar write fails the group before the sources move.
        match &effective.chapters {
            crate::job::ChaptersPolicy::Write { explicit } => {
                // An explicit empty list is a deliberate clear; a resolved
                // empty list leaves the source's chapters untouched.
                let clear = explicit.as_ref().is_some_and(Vec::is_empty);
                if clear {
                    if let Err(e) = self.tagger.clear_chapters(&merged_path) {
                        Self::discard_output(&merged_path);
                        return Err(ProcessorError::Tagging(e));
                    }
                    let sidecar = merged_path.with_extension("chapters.txt");
                    if sidecar.is_file() {
                        std::fs::remove_file(&sidecar).map_err(|e| {
                            Self::discard_output(&merged_path);
                            ProcessorError::Tagging(TaggingError::Io(std::io::Error::new(
                                e.kind(),
                                format!(
                                    "failed to remove stale chapter sidecar {}: {e}",
                                    sidecar.display()
                                ),
                            )))
                        })?;
                    }
                } else if !write_chapters.is_empty() {
                    if let Err(e) = self.tagger.embed_chapters(&merged_path, &write_chapters) {
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
                        &write_chapters,
                    ) {
                        // A partially written sidecar would be an undisclosed
                        // extra: the group reports no artifacts.
                        Self::discard_output(&sidecar);
                        Self::discard_output(&merged_path);
                        return Err(ProcessorError::Tagging(e));
                    }
                }
                // A resolved empty list leaves the merged copy's chapters and
                // any carried sidecar untouched.
            }
            crate::job::ChaptersPolicy::Clear => {
                if let Err(e) = self.tagger.clear_chapters(&merged_path) {
                    Self::discard_output(&merged_path);
                    return Err(ProcessorError::Tagging(e));
                }
                // A re-run with `clear` must not leave a sidecar that still
                // advertises chapters at the deterministic output path.
                let sidecar = merged_path.with_extension("chapters.txt");
                if sidecar.is_file() {
                    std::fs::remove_file(&sidecar).map_err(|e| {
                        Self::discard_output(&merged_path);
                        ProcessorError::Tagging(TaggingError::Io(std::io::Error::new(
                            e.kind(),
                            format!(
                                "failed to remove stale chapter sidecar {}: {e}",
                                sidecar.display()
                            ),
                        )))
                    })?;
                }
            }
            crate::job::ChaptersPolicy::Preserve => {
                // Single-input only (enforced above): the merge is a native
                // copy, so the source chapters survive; carry the sidecar if
                // the source had one. When the source has no sidecar, a
                // re-run to the deterministic output path must not leave an
                // earlier sidecar describing chapters the source lacks.
                let source_sidecar = input_paths[0].with_extension("chapters.txt");
                let dest = merged_path.with_extension("chapters.txt");
                if source_sidecar.is_file() {
                    std::fs::copy(&source_sidecar, &dest).map_err(|e| {
                        Self::discard_output(&merged_path);
                        ProcessorError::Tagging(TaggingError::Io(std::io::Error::new(
                            e.kind(),
                            format!(
                                "failed to carry source chapter sidecar {} to {}: {e}",
                                source_sidecar.display(),
                                dest.display()
                            ),
                        )))
                    })?;
                } else if dest.is_file() {
                    std::fs::remove_file(&dest).map_err(|e| {
                        Self::discard_output(&merged_path);
                        ProcessorError::Tagging(TaggingError::Io(std::io::Error::new(
                            e.kind(),
                            format!(
                                "failed to remove stale chapter sidecar {}: {e}",
                                dest.display()
                            ),
                        )))
                    })?;
                }
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

        // Enumerate the merged output and any `chapters.txt`/`cover.txt`
        // sidecars so the machine result never silently omits a sidecar.
        let mut artifacts = vec![merged_path.clone()];
        for sidecar in
            [merged_path.with_extension("chapters.txt"), merged_path.with_extension("cover.txt")]
        {
            if sidecar.is_file() {
                artifacts.push(sidecar);
            }
        }

        Ok(ProcessingResult {
            input_files: input_paths,
            output_file: merged_path,
            metadata_applied: true,
            files_moved,
            artifacts,
        })
    }

    /// A job's output path: root + the parsed-template resolver's output,
    /// with root containment enforced before any write.
    fn determine_job_output_path(
        &self,
        output_root: &std::path::Path,
        effective: &crate::job::EffectiveJob,
    ) -> Result<PathBuf> {
        let relative = crate::path_resolver::PathResolver::resolve_output(
            effective.template(),
            &effective.metadata,
        )
        .map_err(|e| crate::job::JobError::Invalid(e.to_string()))?;
        self.job_output_path_for(output_root, &relative)
    }

    /// Join a resolved relative path to the root, keeping the output inside
    /// the job's output area.
    ///
    /// The resolver already guarantees every component is non-empty,
    /// separator-free, and free of leading `.`/`:` characters, so lexical
    /// joining cannot escape. On top of that, any directory component that is
    /// itself a symlink is rejected outright (a dangling symlink included),
    /// and the remaining existing components are canonicalized and required
    /// to stay under the canonical root. This is defense-in-depth against a
    /// resolver bug rather than a structural guarantee: it closes the case
    /// where a link below the root would otherwise let the output be written
    /// outside the area, and a symlinked final entry is rejected so the
    /// output file is never written through a link.
    fn job_output_path_for(
        &self,
        output_root: &std::path::Path,
        relative: &str,
    ) -> Result<PathBuf> {
        let canonical_root = std::fs::canonicalize(output_root).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                std::fs::create_dir_all(output_root)
                    .and_then(|_| std::fs::canonicalize(output_root))
                    .map_err(|e| ProcessorError::OutputDirCreation(e.to_string()))
            } else {
                Err(ProcessorError::OutputDirCreation(e.to_string()))
            }
        })?;
        let components: Vec<&str> = relative.split('/').filter(|c| !c.is_empty()).collect();
        // Walk every directory component; the last is the file name.
        let escape = |component: &str, via: &std::path::Path| {
            ProcessorError::Job(crate::job::JobError::Invalid(format!(
                "resolved output path escapes output root via component {component:?}: {}",
                via.display()
            )))
        };
        let mut dir = canonical_root.clone();
        for component in &components[..components.len().saturating_sub(1)] {
            dir = dir.join(component);
            // Reject a symlinked directory component, including a dangling
            // one: following it would write the output outside the root.
            if let Ok(meta) = std::fs::symlink_metadata(&dir) {
                if meta.file_type().is_symlink() {
                    return Err(escape(component, &dir));
                }
            }
            // Defense-in-depth: any component that exists must resolve under
            // the canonical root.
            if dir.exists() {
                let resolved = std::fs::canonicalize(&dir)
                    .map_err(|e| ProcessorError::OutputDirCreation(e.to_string()))?;
                if !resolved.starts_with(&canonical_root) {
                    return Err(escape(component, &dir));
                }
                dir = resolved;
            }
        }
        let candidate = dir.join(components.last().expect("non-empty relative path"));
        // A symlinked final entry would let the output be written through a
        // link that points outside the root; reject it.
        if let Ok(meta) = std::fs::symlink_metadata(&candidate) {
            if meta.file_type().is_symlink() {
                return Err(escape(components.last().unwrap(), &candidate));
            }
        }
        Ok(candidate)
    }

    /// Claim the output file name before any write: `O_CREAT|O_EXCL`
    /// atomically reserves the exact path, so a pre-existing file or
    /// directory (or a concurrent claim) fails with a diagnosed collision —
    /// nothing is written, renamed, or suffixed. After creating parent
    /// directories, their canonical location is re-validated against the
    /// output root so a swapped-in symlink cannot carry the write outside
    /// the root.
    fn claim_output_name(&self, output_path: &std::path::Path) -> Result<()> {
        // Job-mode only: the claim runs inside a job run.
        let job = self.job.as_ref().ok_or_else(|| {
            ProcessorError::Job(crate::job::JobError::Invalid(
                "output name claim requires a job".into(),
            ))
        })?;
        if output_path.exists() {
            return Err(ProcessorError::OutputCollision { path: output_path.to_path_buf() });
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ProcessorError::OutputDirCreation(format!(
                    "Failed to create output directory {}: {e}",
                    parent.display()
                ))
            })?;
            let root =
                std::fs::canonicalize(&job.output_root).unwrap_or_else(|_| job.output_root.clone());
            let canonical_parent = std::fs::canonicalize(parent).map_err(|e| {
                ProcessorError::OutputDirCreation(format!(
                    "Failed to canonicalize output directory {}: {e}",
                    parent.display()
                ))
            })?;
            if !canonical_parent.starts_with(&root) {
                return Err(ProcessorError::OutputDirCreation(format!(
                    "Output directory {} escapes the job output root after creation; refusing",
                    parent.display()
                )));
            }
        }
        // `create_new` maps to `O_CREAT|O_EXCL`: the create fails if the
        // path exists, closing the exists-then-write race without a rename.
        match std::fs::OpenOptions::new().create_new(true).write(true).open(output_path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(ProcessorError::OutputCollision { path: output_path.to_path_buf() })
            }
            Err(e) => Err(ProcessorError::OutputDirCreation(format!(
                "Failed to claim output path {}: {e}",
                output_path.display()
            ))),
        }
    }

    /// Bind the produced output to the validated root after the merge. The
    /// merge writes by path, so the produced file is re-resolved with
    /// [`std::fs::canonicalize`], which follows any parent swapped for a
    /// symlink; the resolved location must be a regular file inside the
    /// canonical output root and not the untouched zero-byte claim
    /// placeholder — a merge redirected through a swapped parent, even one
    /// restored before this check, leaves the claimed file empty. Any
    /// mismatch fails the group and discards the produced file.
    fn verify_output_in_root(&self, output_path: &std::path::Path) -> Result<()> {
        let job = self.job.as_ref().ok_or_else(|| {
            ProcessorError::Job(crate::job::JobError::Invalid(
                "output verification requires a job".into(),
            ))
        })?;
        let final_meta = std::fs::symlink_metadata(output_path).map_err(|e| {
            ProcessorError::OutputDirCreation(format!("Failed to stat output: {e}"))
        })?;
        if final_meta.file_type().is_symlink() {
            Self::discard_output(output_path);
            return Err(self.output_root_escape(output_path));
        }
        let produced = std::fs::canonicalize(output_path).map_err(|e| {
            ProcessorError::OutputDirCreation(format!(
                "Failed to canonicalize produced output {}: {e}",
                output_path.display()
            ))
        })?;
        let root =
            std::fs::canonicalize(&job.output_root).unwrap_or_else(|_| job.output_root.clone());
        let produced_meta = produced.metadata().map_err(|e| {
            ProcessorError::OutputDirCreation(format!("Failed to stat produced output: {e}"))
        })?;
        let in_root = produced.starts_with(&root) && produced_meta.file_type().is_file();
        // A real merge always writes audio into the claimed file; a
        // redirected merge leaves the zero-byte placeholder behind.
        if !in_root || produced_meta.len() == 0 {
            Self::discard_output(output_path);
            return Err(self.output_root_escape(output_path));
        }
        Ok(())
    }

    /// Diagnoses an output that escaped the job output root.
    fn output_root_escape(&self, output_path: &std::path::Path) -> ProcessorError {
        ProcessorError::OutputDirCreation(format!(
            "Produced output {} is not the claimed file inside the job output root; refusing",
            output_path.display()
        ))
    }

    /// The produced artifacts of a result, relative to the job's
    /// `output_root`. Job mode only: the machine result requires root
    /// containment and has no absolute-path fallback, so a produced path
    /// that is not inside the canonical root is not emitted.
    fn artifacts_for_result(&self, result: &ProcessingResult) -> Vec<crate::job::Artifact> {
        let Some(job) = &self.job else {
            return Vec::new();
        };
        let root =
            std::fs::canonicalize(&job.output_root).unwrap_or_else(|_| job.output_root.clone());
        result
            .artifacts
            .iter()
            .filter_map(|p| crate::job::Artifact::from_absolute(&root, p))
            .collect()
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

    /// A processor for pure path-resolution tests: the (unused) API URL
    /// points nowhere, so construction needs no network.
    fn path_test_processor(temp_dir: &TempDir) -> Processor {
        Processor::new(api_test_config(temp_dir, "http://127.0.0.1:1")).unwrap()
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

    /// Build a non-dry-run config with an explicit completed directory so the
    /// job's completed-directory precedence can be asserted against it.
    fn job_config(temp_dir: &TempDir, completed: &std::path::Path) -> Config {
        Config::new(
            vec![],
            Some(temp_dir.path().join("output")),
            None,
            MetadataSourceKind::Audiobookdb,
            MetadataRegion::Us,
            Some(completed.to_path_buf()),
            1,
            "info".to_string(),
            "{author}/{title}".to_string(),
            false,
            None,
        )
    }

    #[tokio::test]
    async fn supplied_single_track_job_embeds_and_writes_sidecar() {
        let temp_dir = TempDir::new().unwrap();
        let book_dir = temp_dir.path().join("Book");
        let input = create_test_m4b(&book_dir, 10);
        let output_root = temp_dir.path().join("out");
        let job_path = temp_dir.path().join("job.json");
        // All default policies (preserve), supplied base: no catalog, no
        // provider chapters, so the chapter list is generated per input track.
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],
                  "input_supplied": {{ "title": "The Book", "authors": ["The Author"] }},
                  "output_root": {:?}, "path_format": "{{author}}/{{title}}" }}"#,
            input, output_root
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();
        let processor =
            Processor::for_job(job, job_config(&temp_dir, &temp_dir.path().join("cfg-done")))
                .unwrap();
        let result =
            processor.process(vec![input.clone()]).await.expect("supplied job must process");
        assert_eq!(result.len(), 1);
        let output = &result[0].output_file;
        assert!(output.exists(), "merged output missing: {}", output.display());
        assert_eq!(output.as_path(), &output_root.join("The Author").join("The Book.m4b"));
        // Chapters were embedded into the merged container (one generated per
        // input track), not only listed.
        let tag = mp4ameta::Tag::read_from_path(output).unwrap();
        assert!(tag.info.duration > std::time::Duration::ZERO);
        // The sidecar lists the resolved (generated) chapters, not the empty
        // supplied-metadata chapters.
        let sidecar = output.with_extension("chapters.txt");
        let text = std::fs::read_to_string(&sidecar).expect("sidecar written");
        assert!(text.contains("## album: The Book"), "sidecar album header: {text}");
        assert!(text.contains("## artist: The Author"), "sidecar artist header: {text}");
        assert!(text.contains("Chapter 01"), "sidecar generated chapter: {text}");
    }

    #[tokio::test]
    async fn job_preview_and_execution_yield_the_same_path() {
        let temp_dir = TempDir::new().unwrap();
        let book_dir = temp_dir.path().join("Book");
        let input = create_test_m4b(&book_dir, 5);
        let output_root = temp_dir.path().join("out");
        let job_path = temp_dir.path().join("job.json");
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],"input_supplied": {{ "title": "Parity", "authors": ["PA"],"subtitle": "SS", "year": 2019 }},"output_root": {:?},"path_format": "{{author}}/{{title}} - {{subtitle}} ({{year}})" }}"#,
            input, output_root
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();
        let config = job_config(&temp_dir, &temp_dir.path().join("cfg-done"));
        // Preview: non-mutating, resolves the exact relative path.
        let preview = job.preview(&config).await.unwrap();
        assert_eq!(preview.relative_path, "PA/Parity - SS (2019).m4b");
        assert!(!output_root.exists(), "preview created nothing");
        // Execution: the artifact is the previewed relative path, exactly.
        let processor = Processor::for_job(job, config.clone()).unwrap();
        let run = processor.run(vec![input.clone()], &NoOpProgressHandler).await.unwrap();
        assert!(run.complete, "run must complete");
        let outcomes = &run.outcomes[0];
        let art = outcomes.artifacts.iter().find(|a| a.kind == "audio").expect("audio artifact");
        assert_eq!(art.relative_path, preview.relative_path, "preview and execution agree");
        assert!(output_root.join(&art.relative_path).exists(), "artifact on disk");
        // The sidecar is enumerated, not silently omitted.
        assert!(
            outcomes
                .artifacts
                .iter()
                .any(|a| a.kind == "sidecar" && a.relative_path.ends_with(".chapters.txt")),
            "chapters sidecar enumerated: {:?}",
            outcomes.artifacts
        );
    }

    #[tokio::test]
    async fn job_output_collision_refuses_to_clobber() {
        let temp_dir = TempDir::new().unwrap();
        let book_dir = temp_dir.path().join("Book");
        let input = create_test_m4b(&book_dir, 5);
        let output_root = temp_dir.path().join("out");
        let job_path = temp_dir.path().join("job.json");
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],"input_supplied": {{ "title": "Clash", "authors": ["A"] }},"output_root": {:?}, "path_format": "{{author}}/{{title}}" }}"#,
            input, output_root
        );
        std::fs::write(&job_path, job_json).unwrap();
        let config = job_config(&temp_dir, &temp_dir.path().join("cfg-done"));

        // A file that a foreign process placed at the resolved path must
        // survive untouched; no silent suffixing, no rename, no partial
        // "successful" output.
        let existing = output_root.join("A").join("Clash.m4b");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, b"foreign content").unwrap();

        let job = crate::job::Job::load(&job_path).unwrap();
        let processor = Processor::for_job(job, config).unwrap();
        let err = processor
            .run(vec![input.clone()], &NoOpProgressHandler)
            .await
            .expect("run reports the outcome, not a panic")
            .outcomes[0]
            .error
            .clone()
            .expect("group failed");
        assert!(err.contains("refusing to overwrite"), "{err}");
        let content = std::fs::read(&existing).expect("foreign file survived");
        assert_eq!(content, b"foreign content", "existing file untouched");
        // No falsely successful output: the claimed name is the foreign one;
        // nothing else was written under the resolved directory.
        let entries: Vec<_> = std::fs::read_dir(output_root.join("A"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("Clash.m4b")], "{entries:?}");
        // The source input is never destroyed by a collision.
        assert!(input.exists(), "source retained for recovery");
    }

    #[tokio::test]
    async fn failed_merge_removes_claimed_placeholder() {
        // A failed merge must not leave the claimed placeholder behind: it
        // would block a retry with a false collision.
        let temp_dir = TempDir::new().unwrap();
        let book_dir = temp_dir.path().join("Book");
        // An MP3 forces transcode mode (the single-file copy fast path
        // does not invoke FFmpeg), so a corrupt file makes the merge fail.
        let input = book_dir.join("audio.mp3");
        std::fs::create_dir_all(&book_dir).unwrap();
        std::fs::write(&input, b"not a real mp3").unwrap();
        let output_root = temp_dir.path().join("out");
        let job_path = temp_dir.path().join("job.json");
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],"input_supplied": {{ "title": "T", "authors": ["A"] }},"output_root": {:?} }}"#,
            input, output_root
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();
        let processor =
            Processor::for_job(job, job_config(&temp_dir, &temp_dir.path().join("cfg-done")))
                .unwrap();
        let run = processor.run(vec![input.clone()], &NoOpProgressHandler).await.unwrap();
        assert!(!run.complete, "a corrupt input must fail the group");
        // No placeholder survives: the resolved output path does not exist.
        let output = output_root.join("A").join("T.m4b");
        assert!(!output.exists(), "placeholder must be discarded on merge failure");
        // The source input is never destroyed.
        assert!(input.exists(), "source retained for recovery");
    }

    #[test]
    fn machine_setup_failed_document_shape() {
        let res = crate::job::JobResult::setup_failed("ffmpeg missing".into());
        assert_eq!(res.status, "failed");
        assert!(!res.all_succeeded(), "setup failure is not a success");
        assert!(res.groups.is_empty(), "no groups ran");
        assert_eq!(res.error.as_deref(), Some("ffmpeg missing"));
        assert_eq!(res.schema_version, crate::job::SCHEMA_VERSION);
        assert_eq!(res.resolver_version, crate::path_resolver::RESOLVER_VERSION);
    }

    #[tokio::test]
    async fn machine_result_enumerates_cover_sidecar() {
        // A source cover.txt carried into the output root must be enumerated
        // as an artifact, not silently omitted.
        let temp_dir = TempDir::new().unwrap();
        let book_dir = temp_dir.path().join("Book");
        let input = create_test_m4b(&book_dir, 5);
        std::fs::write(book_dir.join("audio.cover.txt"), b"cover").unwrap();
        let output_root = temp_dir.path().join("out");
        let job_path = temp_dir.path().join("job.json");
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],"input_supplied": {{ "title": "Cov", "authors": ["A"] }},"cover": "preserve", "output_root": {:?} }}"#,
            input, output_root
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();
        let processor =
            Processor::for_job(job, job_config(&temp_dir, &temp_dir.path().join("cfg-done")))
                .unwrap();
        let run = processor.run(vec![input.clone()], &NoOpProgressHandler).await.unwrap();
        assert!(run.complete, "run must complete");
        let outcomes = &run.outcomes[0];
        let kinds: Vec<&str> = outcomes.artifacts.iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&"audio"), "audio artifact present: {kinds:?}");
        let cover = outcomes.artifacts.iter().find(|a| {
            a.relative_path.ends_with(".cover.txt") || a.relative_path.ends_with("cover.txt")
        });
        assert!(cover.is_some(), "cover sidecar enumerated: {:?}", outcomes.artifacts);
    }

    #[tokio::test]
    async fn supplied_multi_track_job_is_one_group() {
        let temp_dir = TempDir::new().unwrap();
        let book_dir = temp_dir.path().join("Book");
        std::fs::create_dir_all(&book_dir).unwrap();
        // Two audio files in one directory: discovery collects both, and the
        // job combines them into a single group (never one group per file).
        for (name, secs) in [("01.m4b", 5), ("02.m4b", 5)] {
            let path = book_dir.join(name);
            let status = std::process::Command::new("ffmpeg")
                .args(["-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-t", &secs.to_string()])
                .args(["-c:a", "aac", "-b:a", "64k", "-y"])
                .arg(&path)
                .status()
                .expect("run ffmpeg");
            assert!(status.success(), "ffmpeg failed to create test audio");
        }
        let output_root = temp_dir.path().join("out");
        let job_path = temp_dir.path().join("job.json");
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],
                  "input_supplied": {{ "title": "Multi Track", "authors": ["A"] }},
                  "output_root": {:?}, "path_format": "{{author}}/{{title}}" }}"#,
            book_dir, output_root
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();
        let processor =
            Processor::for_job(job, job_config(&temp_dir, &temp_dir.path().join("cfg-done")))
                .unwrap();
        let result = processor
            .process(vec![book_dir.clone()])
            .await
            .expect("a multi-track job is one book, not one group per directory");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].input_files.len(), 2, "both tracks in the single group");
        assert_eq!(result[0].output_file, output_root.join("A").join("Multi Track.m4b"));
    }

    #[tokio::test]
    async fn job_completed_directory_wins_over_config() {
        let temp_dir = TempDir::new().unwrap();
        let book_dir = temp_dir.path().join("Book");
        let input = create_test_m4b(&book_dir, 5);
        let output_root = temp_dir.path().join("out");
        let job_completed = temp_dir.path().join("job-done");
        let cfg_completed = temp_dir.path().join("cfg-done");
        let job_path = temp_dir.path().join("job.json");
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],
                  "input_supplied": {{ "title": "Done Book", "authors": ["A"] }},
                  "output_root": {:?}, "path_format": "{{author}}/{{title}}",
                  "completed_directory": {:?} }}"#,
            input, output_root, job_completed
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();
        let processor = Processor::for_job(job, job_config(&temp_dir, &cfg_completed)).unwrap();
        let result = processor.process(vec![input.clone()]).await.expect("job must process");
        assert!(result[0].files_moved, "sources moved to the job's completed directory");
        assert!(job_completed.join("audio.m4b").exists(), "source moved to the job's directory");
        assert!(!cfg_completed.join("audio.m4b").exists(), "not the config's directory");
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

    // An explicit chapter list is bounded by the input's real playlength, not
    // the provider's chapter total. The stub provider's chapter ends at 10s,
    // so a pre-merge guard comparing against the provider duration would
    // wrongly reject a valid explicit chapter that sits in 10–30s of a 30s
    // input; the authoritative check is post-merge against the merged
    // container's duration.
    #[tokio::test]
    async fn test_job_explicit_chapters_validated_against_input_not_provider_duration() {
        let temp_dir = TempDir::new().unwrap();
        let api_url = spawn_metadata_api();
        let book_dir = temp_dir.path().join("Job Book");
        let input = create_test_m4b(&book_dir, 30);
        let output_root = temp_dir.path().join("output");

        let job_path = temp_dir.path().join("job.json");
        // The explicit chapter spans 15s–25s: within the 30s input, but past
        // the provider's 10s chapter total (which would reject it pre-merge).
        // A non-`abc123def456` id resolves to the stub's single 0–10s chapter,
        // so the provider `total_duration()` is 10s, not the input's 30s.
        let job_json = format!(
            r#"{{ "schema_version": 1, "inputs": [{:?}],
                  "input_provider": {{"book_id": "jobprovider0001"}},
                  "output_root": {:?}, "path_format": "{{author}}/{{title}}",
                  "chapters": {{"write": {{"chapters": [
                      {{"title": "Ch", "start_millis": 15000, "duration_millis": 10000}}
                  ]}}}} }}"#,
            input, output_root
        );
        std::fs::write(&job_path, job_json).unwrap();
        let job = crate::job::Job::load(&job_path).unwrap();

        let processor = Processor::for_job(job, api_test_config(&temp_dir, &api_url)).unwrap();
        let result = processor.process(vec![input.clone()]).await.expect(
            "a valid explicit chapter within the input duration must not be rejected pre-merge",
        );
        assert!(!result.is_empty());
        assert_eq!(result[0].input_files, vec![input.clone()]);
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
            Err(ProcessorError::PartialFailure { results, outcomes, detail }) => {
                assert_eq!(results.len(), 1, "only the valid group should complete");
                assert_eq!(outcomes.len(), 2, "every group outcome is recorded");
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

    #[tokio::test]
    async fn test_preserve_chapters_removes_stale_sidecar_when_source_has_none() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let output_root = temp_dir.path().join("output");

        // Single-input source with no sidecar of its own.
        let book_dir = temp_dir.path().join("P Book");
        let input = create_test_m4b(&book_dir, 10);

        // The deterministic output path for {author}/{title} with title "P",
        // author "A". A stale sidecar from an earlier run sits there,
        // describing chapters the source does not carry.
        let dest_dir = output_root.join("A");
        std::fs::create_dir_all(&dest_dir).unwrap();
        let dest = dest_dir.join("P.m4b");
        let sidecar = dest_dir.join("P.chapters.txt");
        std::fs::write(&sidecar, "stale\n").unwrap();

        let job: crate::job::Job = serde_json::from_str(&format!(
            r#"{{ "schema_version": 1, "inputs": ["{}"],
                    "input_supplied": {{"title": "P", "authors": ["A"]}},
                    "output_root": "{}", "path_format": "{{author}}/{{title}}",
                    "chapters": "preserve", "cover": "preserve" }}"#,
            input.display(),
            output_root.display()
        ))
        .unwrap();
        // Supplied mode resolves offline, so resolve before the processor takes
        // ownership of the job; the group path requires the resolved job.
        let effective = job.resolve(None).await.unwrap();
        let processor = Processor::for_job(job, config).unwrap();
        let group = AudioGroup {
            name: "P Book".to_string(),
            files: vec![AudioFile::new(input.clone()).unwrap()],
            disc_number: None,
        };

        let result =
            processor.process_group(&group, &NoOpProgressHandler, Some(&effective)).await.unwrap();
        assert_eq!(result.output_file, dest, "output path must be deterministic");

        // Preserve must not leave a sidecar the source lacks: the stale file is
        // removed so it cannot describe chapters absent from the copied source.
        assert!(!sidecar.exists(), "stale chapter sidecar must be removed on preserve");
    }

    /// A minimal 1x1 JPEG, for a local cover asset.
    fn jpeg_bytes() -> Vec<u8> {
        let mut v = Vec::from([0xFF, 0xD8, 0xFF, 0xE0]);
        v.push(1);
        v.push(0xFF);
        v.push(0xD9);
        v
    }

    #[tokio::test]
    async fn test_job_chapters_write_policy_explicit_and_default() {
        // Explicit-empty clears both representations; the default resolved
        // write keeps the source chapters. The two shapes must not be
        // conflated: the provider list is absent here (supplied job), so a
        // default write falls through to the source's own chapters.
        let temp_dir = TempDir::new().unwrap();
        let output_root = temp_dir.path().join("output");
        let config = create_test_config(&temp_dir);

        for (n, (chapters_field, clear)) in [
            (r#""chapters": { "write": { "chapters": [] } }"#, true),
            (r#""chapters": "write""#, false),
        ]
        .into_iter()
        .enumerate()
        {
            // A distinct title per iteration so the two outputs (and sidecars)
            // do not collide at the same deterministic path.
            let title = format!("Ch{n}");
            let book_dir = temp_dir.path().join(format!("Ch Book {n}"));
            let input = create_test_m4b(&book_dir, 10);
            // Give the source a chapter so the two outcomes differ.
            let mut tag = mp4ameta::Tag::read_from_path(&input).unwrap();
            tag.chapter_list_mut().push(mp4ameta::Chapter::new(std::time::Duration::ZERO, "Intro"));
            tag.write_to_path(&input).unwrap();

            let job: crate::job::Job = serde_json::from_str(&format!(
                r#"{{ "schema_version": 1, "inputs": ["{input}"],
                        "input_supplied": {{"title": "{title}", "authors": ["A"]}},
                        "output_root": "{root}", "path_format": "{{author}}/{{title}}",
                        {field} }}"#,
                input = input.display(),
                title = title,
                root = output_root.display(),
                field = chapters_field
            ))
            .unwrap();
            let effective = job.resolve(None).await.unwrap();
            let processor = Processor::for_job(job, config.clone()).unwrap();
            let group = AudioGroup {
                name: "Ch Book".to_string(),
                files: vec![AudioFile::new(input.clone()).unwrap()],
                disc_number: None,
            };
            let result = processor
                .process_group(&group, &NoOpProgressHandler, Some(&effective))
                .await
                .unwrap();

            let out = result.output_file.clone();
            let out_tag = mp4ameta::Tag::read_from_path(&out).unwrap();
            let sidecar = out.with_extension("chapters.txt");
            if clear {
                assert!(
                    out_tag.chapter_list().is_empty(),
                    "an explicit empty list must clear the embedded chapters"
                );
                assert!(!sidecar.exists(), "an explicit empty list must remove the sidecar");
            } else {
                assert_eq!(
                    out_tag.chapter_list().len(),
                    1,
                    "the default write must embed the source's resolved chapter"
                );
                assert!(sidecar.exists(), "the default write must write the sidecar");
            }
        }
    }

    #[tokio::test]
    async fn test_job_cover_preserve_multi_input_rejected() {
        // A merge carries no cover, so a strict preserve promise over a
        // multi-input group is refused before any output is produced.
        let temp_dir = TempDir::new().unwrap();
        let output_root = temp_dir.path().join("output");
        let config = create_test_config(&temp_dir);

        let book_dir = temp_dir.path().join("Cover Book");
        let f1 = create_test_m4b(&book_dir, 5);
        let book_dir2 = temp_dir.path().join("Cover Book2");
        let f2 = create_test_m4b(&book_dir2, 5);

        let job: crate::job::Job = serde_json::from_str(&format!(
            r#"{{ "schema_version": 1, "inputs": ["{a}", "{b}"],
                    "input_supplied": {{"title": "P", "authors": ["A"]}},
                    "output_root": "{root}", "path_format": "{{author}}/{{title}}",
                    "cover": "preserve" }}"#,
            a = f1.display(),
            b = f2.display(),
            root = output_root.display()
        ))
        .unwrap();
        let effective = job.resolve(None).await.unwrap();
        let processor = Processor::for_job(job, config).unwrap();
        let group = AudioGroup {
            name: "Cover Book (multi)".to_string(),
            files: vec![AudioFile::new(f1).unwrap(), AudioFile::new(f2).unwrap()],
            disc_number: None,
        };

        let err = processor
            .process_group(&group, &NoOpProgressHandler, Some(&effective))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("cover policy 'preserve'"),
            "expected the multi-input cover-preserve rejection, got {err}"
        );
        // The rejection happens before merge: nothing is written.
        assert!(!output_root.join("A/P.m4b").exists());
    }

    #[tokio::test]
    async fn test_job_cover_preserve_single_input_keeps_source_art() {
        // Single-input preserve is a native copy: the source's embedded cover
        // survives, and no URL is fetched (supplied jobs carry none).
        let temp_dir = TempDir::new().unwrap();
        let output_root = temp_dir.path().join("output");
        let config = create_test_config(&temp_dir);

        let book_dir = temp_dir.path().join("Art Book");
        let input = create_test_m4b(&book_dir, 5);
        let mut tag = mp4ameta::Tag::read_from_path(&input).unwrap();
        tag.set_artwork(mp4ameta::Img::new(mp4ameta::ImgFmt::Jpeg, jpeg_bytes()));
        tag.write_to_path(&input).unwrap();

        let job: crate::job::Job = serde_json::from_str(&format!(
            r#"{{ "schema_version": 1, "inputs": ["{input}"],
                    "input_supplied": {{"title": "P", "authors": ["A"]}},
                    "output_root": "{root}", "path_format": "{{author}}/{{title}}",
                    "cover": "preserve" }}"#,
            input = input.display(),
            root = output_root.display()
        ))
        .unwrap();
        let effective = job.resolve(None).await.unwrap();
        let processor = Processor::for_job(job, config).unwrap();
        let group = AudioGroup {
            name: "Art Book".to_string(),
            files: vec![AudioFile::new(input.clone()).unwrap()],
            disc_number: None,
        };
        let result =
            processor.process_group(&group, &NoOpProgressHandler, Some(&effective)).await.unwrap();

        let out_tag = mp4ameta::Tag::read_from_path(&result.output_file).unwrap();
        assert!(out_tag.artwork().is_some(), "preserve must keep the source's embedded cover");
    }

    #[tokio::test]
    async fn test_job_chapters_write_explicit_list_emitted() {
        // An explicit list embeds exactly the supplied chapters (replacing
        // the source's own), in range against the merged audio, and writes the
        // sidecar. This asserts the emitted chapters, not just that some list
        // is present.
        let temp_dir = TempDir::new().unwrap();
        let output_root = temp_dir.path().join("output");
        let config = create_test_config(&temp_dir);

        let book_dir = temp_dir.path().join("Explicit Book");
        let input = create_test_m4b(&book_dir, 10);
        // A source chapter that must NOT survive an explicit-list write.
        let mut tag = mp4ameta::Tag::read_from_path(&input).unwrap();
        tag.chapter_list_mut()
            .push(mp4ameta::Chapter::new(std::time::Duration::ZERO, "SourceChapter"));
        tag.write_to_path(&input).unwrap();

        let job: crate::job::Job = serde_json::from_str(&format!(
            r#"{{ "schema_version": 1, "inputs": ["{input}"],
                    "input_supplied": {{"title": "Explicit Book", "authors": ["A"]}},
                    "output_root": "{root}", "path_format": "{{author}}/{{title}}",
                    "chapters": {{ "write": {{ "chapters": [
                        {{"title": "Explicit One", "start_millis": 0, "duration_millis": 2000}},
                        {{"title": "Explicit Two", "start_millis": 2000, "duration_millis": 3000}} ] }} }} }}"#,
            input = input.display(),
            root = output_root.display()
        ))
        .unwrap();
        let effective = job.resolve(None).await.unwrap();
        let processor = Processor::for_job(job, config).unwrap();
        let group = AudioGroup {
            name: "Explicit Book".to_string(),
            files: vec![AudioFile::new(input).unwrap()],
            disc_number: None,
        };
        let result =
            processor.process_group(&group, &NoOpProgressHandler, Some(&effective)).await.unwrap();

        let out_tag = mp4ameta::Tag::read_from_path(&result.output_file).unwrap();
        let titles: Vec<&str> = out_tag.chapter_list().iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Explicit One", "Explicit Two"],
            "the explicit list must be embedded verbatim (source chapter replaced)"
        );
        // The chapter track must match the list (both representations).
        let track_titles: Vec<&str> =
            out_tag.chapter_track().iter().map(|c| c.title.as_str()).collect();
        assert_eq!(track_titles, vec!["Explicit One", "Explicit Two"]);
        // The sidecar lists the same chapters.
        let sidecar = result.output_file.with_extension("chapters.txt");
        assert!(sidecar.exists(), "an explicit write must write the sidecar");
        let sidecar_text = std::fs::read_to_string(&sidecar).unwrap();
        assert!(sidecar_text.contains("Explicit One") && sidecar_text.contains("Explicit Two"));
    }

    #[tokio::test]
    async fn test_job_chapters_preserve_keeps_source_chapters() {
        // A single-input preserve is a native copy: the source's own chapters
        // must survive. (The stale-sidecar test uses a chapter-less source, so
        // it cannot verify preservation of real chapters.)
        let temp_dir = TempDir::new().unwrap();
        let output_root = temp_dir.path().join("output");
        let config = create_test_config(&temp_dir);

        let book_dir = temp_dir.path().join("Preserve Ch Book");
        let input = create_test_m4b(&book_dir, 10);
        let mut tag = mp4ameta::Tag::read_from_path(&input).unwrap();
        tag.chapter_list_mut()
            .push(mp4ameta::Chapter::new(std::time::Duration::ZERO, "PreservedCh"));
        tag.write_to_path(&input).unwrap();

        let job: crate::job::Job = serde_json::from_str(&format!(
            r#"{{ "schema_version": 1, "inputs": ["{input}"],
                    "input_supplied": {{"title": "Preserve Ch Book", "authors": ["A"]}},
                    "output_root": "{root}", "path_format": "{{author}}/{{title}}",
                    "chapters": "preserve", "cover": "preserve" }}"#,
            input = input.display(),
            root = output_root.display()
        ))
        .unwrap();
        let effective = job.resolve(None).await.unwrap();
        let processor = Processor::for_job(job, config).unwrap();
        let group = AudioGroup {
            name: "Preserve Ch Book".to_string(),
            files: vec![AudioFile::new(input).unwrap()],
            disc_number: None,
        };
        let result =
            processor.process_group(&group, &NoOpProgressHandler, Some(&effective)).await.unwrap();

        let out_tag = mp4ameta::Tag::read_from_path(&result.output_file).unwrap();
        let titles: Vec<&str> = out_tag.chapter_list().iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["PreservedCh"],
            "a single-input preserve must keep the source's chapters"
        );
    }

    #[tokio::test]
    async fn test_job_chapters_preserve_multi_input_rejected() {
        // A merge cannot byte-preserve per-file chapters, so a multi-input
        // `chapters: preserve` is refused before any output is produced.
        let temp_dir = TempDir::new().unwrap();
        let output_root = temp_dir.path().join("output");
        let config = create_test_config(&temp_dir);

        let book_dir = temp_dir.path().join("MultiCh Book");
        let f1 = create_test_m4b(&book_dir, 5);
        let book_dir2 = temp_dir.path().join("MultiCh Book2");
        let f2 = create_test_m4b(&book_dir2, 5);

        let job: crate::job::Job = serde_json::from_str(&format!(
            r#"{{ "schema_version": 1, "inputs": ["{a}", "{b}"],
                    "input_supplied": {{"title": "P", "authors": ["A"]}},
                    "output_root": "{root}", "path_format": "{{author}}/{{title}}",
                    "chapters": "preserve" }}"#,
            a = f1.display(),
            b = f2.display(),
            root = output_root.display()
        ))
        .unwrap();
        let effective = job.resolve(None).await.unwrap();
        let processor = Processor::for_job(job, config).unwrap();
        let group = AudioGroup {
            name: "MultiCh Book (multi)".to_string(),
            files: vec![AudioFile::new(f1).unwrap(), AudioFile::new(f2).unwrap()],
            disc_number: None,
        };

        let err = processor
            .process_group(&group, &NoOpProgressHandler, Some(&effective))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("chapters policy 'preserve'"),
            "expected the multi-input chapter-preserve rejection, got {err}"
        );
        assert!(!output_root.join("A/P.m4b").exists());
    }

    #[test]
    fn job_output_path_stays_inside_root_for_normal_paths() {
        let temp_dir = TempDir::new().unwrap();
        let processor = path_test_processor(&temp_dir);
        let root = temp_dir.path().join("output");
        let path = processor.job_output_path_for(&root, "Author/Book.m4b").unwrap();
        assert!(path.starts_with(std::fs::canonicalize(&root).unwrap()));
        assert!(path.file_name().unwrap() == "Book.m4b");
    }

    #[cfg(unix)]
    #[test]
    fn job_output_path_rejects_a_symlinked_directory_component() {
        let temp_dir = TempDir::new().unwrap();
        let processor = path_test_processor(&temp_dir);
        let root = temp_dir.path().join("output");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Author")).unwrap();
        let err = processor.job_output_path_for(&root, "Author/Book.m4b").unwrap_err();
        assert!(
            err.to_string().contains("escapes output root"),
            "symlinked directory must be rejected, got {err}"
        );
        // Nothing may be written outside the root.
        assert!(!outside.join("Book.m4b").exists());
    }

    #[cfg(unix)]
    #[test]
    fn job_output_path_rejects_a_dangling_symlinked_directory() {
        // A symlink to a target that no longer exists: `exists()` is false,
        // so the containment check must reject it via the symlink itself,
        // not by resolving the target.
        let temp_dir = TempDir::new().unwrap();
        let processor = path_test_processor(&temp_dir);
        let root = temp_dir.path().join("output");
        std::fs::create_dir_all(&root).unwrap();
        let missing = temp_dir.path().join("does-not-exist");
        std::os::unix::fs::symlink(&missing, root.join("Author")).unwrap();
        let err = processor.job_output_path_for(&root, "Author/Book.m4b").unwrap_err();
        assert!(
            err.to_string().contains("escapes output root"),
            "dangling symlinked directory must be rejected, got {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn job_output_path_rejects_a_symlinked_final_entry() {
        let temp_dir = TempDir::new().unwrap();
        let processor = path_test_processor(&temp_dir);
        let root = temp_dir.path().join("output");
        let outside = temp_dir.path().join("outside.m4b");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&outside, b"existing").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Link.m4b")).unwrap();
        let err = processor.job_output_path_for(&root, "Link.m4b").unwrap_err();
        assert!(
            err.to_string().contains("escapes output root"),
            "symlinked final entry must be rejected, got {err}"
        );
        // The target file must be untouched.
        assert_eq!(std::fs::read(&outside).unwrap(), b"existing");
    }
}
