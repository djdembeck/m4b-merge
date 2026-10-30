use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::api::{MetadataError, MetadataSource};
use crate::audio::ffmpeg::{FFmpeg, FFmpegError};
use crate::config::Config;
use crate::discovery::{AudioFile, AudioGroup, DiscoveryError, discover_and_group};
use crate::merge::{MergeError, MergeJob, Merger};
use crate::metadata::BookMetadata;
use crate::tagging::{Tagger, TaggingError, format_duration};

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

        Ok(Self { config, ffmpeg, api_client, merger, tagger })
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

        Self { config, ffmpeg, api_client, merger, tagger }
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

        // Skip creating output directory in dry-run mode
        if !self.config.dry_run {
            if let Some(output) = &self.config.output {
                std::fs::create_dir_all(output)
                    .map_err(|e| ProcessorError::OutputDirCreation(e.to_string()))?;
            }
        }

        // Stage 1: Discovery
        progress_handler.on_progress(ProcessingProgress {
            stage: ProcessingStage::Discovery,
            current_file: None,
            total_files: inputs.len(),
            completed_files: 0,
            message: "Discovering audio files...".to_string(),
        });

        let groups = self.discover_files(&inputs)?;
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

            match self.process_group(group, progress_handler).await {
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
    ) -> Result<ProcessingResult> {
        let input_paths: Vec<PathBuf> = group.files.iter().map(|f| f.path.clone()).collect();

        debug!("Processing group '{}' with {} files", group.name, input_paths.len());

        // Stage 2: API Lookup (optional - only if metadata ID is provided or can be inferred)
        // Skipped in dry-run mode to avoid unnecessary network calls
        let metadata = if self.config.dry_run {
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

        // Extract chapters from input file if API didn't provide them
        let metadata = if let Some(mut meta) = metadata {
            if meta.chapters.is_empty() && !group.files.is_empty() {
                // Try to read chapters from the first input file
                match crate::chapters::read_chapters(
                    &group.files[0].path,
                    self.ffmpeg.ffprobe_path().to_str(),
                ) {
                    Ok(file_chapters) if !file_chapters.is_empty() => {
                        info!("Extracted {} chapters from input file", file_chapters.len());
                        // Convert file chapters to metadata chapters
                        meta.chapters = file_chapters
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
                    Ok(_) => {} // No chapters found
                    Err(e) => {
                        warn!("Failed to extract chapters from input file: {}", e);
                    }
                }
            }
            Some(meta)
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
        // list, so fail the book and name every offending chapter.
        if let Some(meta) = metadata.as_ref() {
            if !meta.chapters.is_empty() {
                match self.ffmpeg.get_duration(&merged_path) {
                    Ok(duration) => {
                        let out_of_range = meta.out_of_range_chapters(duration);
                        if !out_of_range.is_empty() {
                            let details = out_of_range
                                .iter()
                                .map(|chapter| {
                                    format!(
                                        "'{}' at {}",
                                        chapter.title,
                                        format_duration(chapter.start_time)
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Err(ProcessorError::ChaptersOutOfRange {
                                path: merged_path,
                                count: out_of_range.len(),
                                playlength: format_duration(duration),
                                details,
                            });
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Could not determine duration of {}; refusing to tag chapters: {}",
                            merged_path.display(),
                            e
                        );
                        return Err(ProcessorError::ProbeFailed { path: merged_path, source: e });
                    }
                }
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

                // Embed chapters into M4B file
                if !meta.chapters.is_empty() {
                    if let Err(e) = self.tagger.embed_chapters(&merged_path, &meta.chapters) {
                        warn!("Failed to embed chapters: {}", e);
                    } else {
                        info!("Successfully embedded {} chapters", meta.chapters.len());
                    }
                }

                // Write chapters.txt next to the output file
                let chapters_txt_path = merged_path.with_extension("chapters.txt");
                if let Err(e) = self.tagger.write_chapters_txt(&chapters_txt_path, meta) {
                    warn!("Failed to write chapters.txt: {}", e);
                }

                true
            }
        } else {
            false
        };

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

    /// Discover audio files from input paths
    fn discover_files(&self, inputs: &[PathBuf]) -> Result<Vec<AudioGroup>> {
        let groups = match discover_and_group(inputs) {
            Ok(g) => g,
            Err(e) => {
                if self.config.dry_run {
                    warn!("Discovery error in dry-run mode: {}", e);
                    return Ok(Vec::new());
                }
                return Err(e.into());
            }
        };
        Ok(groups)
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
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        let processor = Processor::new(config).unwrap();

        // Empty inputs returns empty groups (not an error at discovery stage)
        let result = processor.discover_files(&[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
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

        let result = processor.process_group(&group, &NoOpProgressHandler).await;
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

        let result = processor.process_group(&group, &NoOpProgressHandler).await;
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

            let result = processor.process_group(&group, &NoOpProgressHandler).await;
            assert!(result.is_ok());
            let result = result.unwrap();
            assert!(result.output_file.starts_with(&output_dir));
            assert_eq!(result.output_file.extension().unwrap(), "m4b");
        }
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

        let err = processor.process_group(&group, &NoOpProgressHandler).await.unwrap_err();

        match &err {
            ProcessorError::ChaptersOutOfRange { count, playlength, details, .. } => {
                assert_eq!(*count, 1);
                assert_eq!(playlength, "00:00:10.000");
                assert!(details.contains("End Credits"), "{details}");
            }
            other => panic!("expected ChaptersOutOfRange, got {other:?}"),
        }

        // The check runs before tagging: the copied output has no chapters and
        // no chapters.txt was written.
        let output = temp_dir.path().join("output/Test Author/Book abc123def456.m4b");
        assert!(output.exists());
        let tag = mp4ameta::Tag::read_from_path(&output).unwrap();
        assert!(tag.chapter_list().is_empty(), "chapters must not be embedded");
        assert!(!output.with_extension("chapters.txt").exists());

        // And before the source files are moved.
        assert!(input.exists(), "source file must not be moved after a failed check");
        let completed = temp_dir.path().join("completed");
        let moved = std::fs::read_dir(&completed).map(|entries| entries.count()).unwrap_or(0);
        assert_eq!(moved, 0);
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

        let err = processor.process_group(&group, &NoOpProgressHandler).await.unwrap_err();
        let message = err.to_string();

        match &err {
            ProcessorError::ProbeFailed { path, .. } => {
                assert!(path.to_string_lossy().ends_with("Book abc123def456.m4b"));
            }
            other => panic!("expected ProbeFailed, got {other:?}"),
        }
        assert!(message.contains("refusing to tag chapters"), "{message}");
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
}
