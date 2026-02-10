use crate::audio::ffmpeg::{FFmpeg, FFmpegError};
use crate::discovery::{AudioFile, AudioFormat};
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, info, trace, warn};

/// Errors that can occur during merge operations
#[derive(Error, Debug)]
pub enum MergeError {
    #[error("FFmpeg error: {0}")]
    FFmpeg(#[from] FFmpegError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("No input files provided")]
    NoInputFiles,

    #[error("Output path not specified")]
    NoOutputPath,

    #[error("Failed to create temporary file: {0}")]
    TempFileCreation(String),

    #[error("Failed to detect bitrate from source files")]
    BitrateDetectionFailed,

    #[error("Merge operation failed: {0}")]
    OperationFailed(String),

    #[error("Progress parsing error: {0}")]
    ProgressParseError(String),

    #[error("Incompatible formats for copy mode: expected M4A/M4B, found {0:?}")]
    IncompatibleFormats(Vec<AudioFormat>),
}

/// Result type for merge operations
pub type Result<T> = std::result::Result<T, MergeError>;

/// Merge mode - determines whether to copy or transcode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    /// Copy mode: No re-encoding, files must be M4A/M4B
    Copy,
    /// Transcode mode: Re-encode to AAC with target bitrate
    Transcode,
}

impl MergeMode {
    /// Determine merge mode from input files
    /// Returns Copy if all files are M4A/M4B, Transcode otherwise
    pub fn from_files(files: &[AudioFile]) -> Self {
        let all_m4 = files.iter().all(|f| {
            matches!(f.format, AudioFormat::M4A | AudioFormat::M4B)
        });

        if all_m4 {
            MergeMode::Copy
        } else {
            MergeMode::Transcode
        }
    }

    /// Get FFmpeg codec arguments for this mode
    fn codec_args(&self, target_bitrate: Option<u32>) -> Vec<String> {
        match self {
            MergeMode::Copy => vec!["-c".to_string(), "copy".to_string()],
            MergeMode::Transcode => {
                let bitrate = target_bitrate.unwrap_or(128);
                vec![
                    "-c:a".to_string(),
                    "aac".to_string(),
                    "-b:a".to_string(),
                    format!("{}k", bitrate),
                ]
            }
        }
    }
}

/// Merge job configuration
#[derive(Debug, Clone)]
pub struct MergeJob {
    /// Input audio files to merge
    pub input_files: Vec<AudioFile>,
    /// Output file path
    pub output_path: PathBuf,
    /// Merge mode (Copy or Transcode)
    pub mode: MergeMode,
    /// Target bitrate for transcoding (in kbps)
    pub target_bitrate: Option<u32>,
    /// Number of threads to use for encoding
    pub num_threads: Option<usize>,
}

impl MergeJob {
    /// Create a new merge job with auto-detected mode
    pub fn new(input_files: Vec<AudioFile>, output_path: PathBuf) -> Self {
        let mode = MergeMode::from_files(&input_files);

        Self {
            input_files,
            output_path,
            mode,
            target_bitrate: None,
            num_threads: None,
        }
    }

    /// Set the merge mode explicitly
    pub fn with_mode(mut self, mode: MergeMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set target bitrate for transcoding
    pub fn with_bitrate(mut self, bitrate: u32) -> Self {
        self.target_bitrate = Some(bitrate);
        self
    }

    /// Set number of threads for encoding
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.num_threads = Some(threads);
        self
    }

    /// Validate the job configuration
    pub fn validate(&self) -> Result<()> {
        if self.input_files.is_empty() {
            return Err(MergeError::NoInputFiles);
        }

        // Check that all input files exist
        for file in &self.input_files {
            if !file.path.exists() {
                return Err(MergeError::FFmpeg(FFmpegError::FileNotFound(
                    file.path.clone(),
                )));
            }
        }

        // For copy mode, verify all files are M4A/M4B
        if self.mode == MergeMode::Copy {
            let non_m4: Vec<_> = self
                .input_files
                .iter()
                .filter(|f| !matches!(f.format, AudioFormat::M4A | AudioFormat::M4B))
                .map(|f| f.format.clone())
                .collect();

            if !non_m4.is_empty() {
                return Err(MergeError::IncompatibleFormats(non_m4));
            }
        }

        Ok(())
    }
}

/// Progress information during merge
#[derive(Debug, Clone)]
pub struct MergeProgress {
    /// Current file being processed (1-based index)
    pub current_file: usize,
    /// Total number of files
    pub total_files: usize,
    /// Current processing time
    pub time: Duration,
    /// Processing speed (e.g., 32.1x)
    pub speed: f64,
    /// Current bitrate
    pub bitrate: Option<u64>,
    /// Size processed so far (in bytes)
    pub size: Option<u64>,
}

impl MergeProgress {
    /// Calculate progress percentage (0.0 to 1.0)
    pub fn percent_complete(&self) -> f64 {
        if self.total_files == 0 {
            return 0.0;
        }

        // Progress based on files completed plus current file progress
        let file_progress = (self.current_file.saturating_sub(1)) as f64 / self.total_files as f64;
        let current_file_contribution = 1.0 / self.total_files as f64;

        file_progress + (current_file_contribution * 0.5) // Assume 50% through current file
    }
}

/// Trait for handling merge progress callbacks
pub trait ProgressHandler: Send + Sync {
    /// Called when progress is updated
    fn on_progress(&self, progress: MergeProgress);
}

/// Simple progress handler that logs progress
pub struct LoggingProgressHandler;

impl ProgressHandler for LoggingProgressHandler {
    fn on_progress(&self, progress: MergeProgress) {
        info!(
            "Progress: {}/{} files, time: {:?}, speed: {:.1}x",
            progress.current_file, progress.total_files, progress.time, progress.speed
        );
    }
}

/// No-op progress handler
pub struct NoOpProgressHandler;

impl ProgressHandler for NoOpProgressHandler {
    fn on_progress(&self, _progress: MergeProgress) {}
}

/// Manages audio file merging operations
pub struct Merger {
    ffmpeg: Arc<FFmpeg>,
}

impl Merger {
    /// Create a new Merger with the given FFmpeg instance
    pub fn new(ffmpeg: Arc<FFmpeg>) -> Self {
        Self { ffmpeg }
    }

    /// Create a new Merger by discovering FFmpeg in PATH
    pub fn discover() -> Result<Self> {
        let ffmpeg = Arc::new(FFmpeg::discover()?);
        Ok(Self { ffmpeg })
    }

    /// Detect the optimal bitrate from source files
    /// Returns the most common bitrate among input files, rounded to standard values
    pub fn detect_bitrate(&self, files: &[AudioFile]) -> Result<u32> {
        if files.is_empty() {
            return Err(MergeError::BitrateDetectionFailed);
        }

        // Collect bitrates from files that have metadata
        let bitrates: Vec<u32> = files
            .iter()
            .filter_map(|f| f.metadata.as_ref().map(|m| m.bitrate))
            .filter(|&b| b > 0)
            .collect();

        if bitrates.is_empty() {
            // Probe the first file if metadata not available
            let first_file = &files[0];
            let metadata = self
                .ffmpeg
                .get_metadata(&first_file.path)
                .map_err(MergeError::FFmpeg)?;

            let bitrate = metadata.bitrate.ok_or(MergeError::BitrateDetectionFailed)?;
            Ok(Self::round_to_standard_bitrate((bitrate / 1000) as u32))
        } else {
            // Use the most common bitrate
            let avg_bitrate = bitrates.iter().sum::<u32>() / bitrates.len() as u32;
            Ok(Self::round_to_standard_bitrate(avg_bitrate))
        }
    }

    /// Round bitrate to nearest standard value
    fn round_to_standard_bitrate(bitrate: u32) -> u32 {
        let standards = [64, 96, 128, 192, 256, 320];

        // Find closest standard bitrate
        standards
            .iter()
            .min_by_key(|&&std| (std as i32 - bitrate as i32).abs())
            .copied()
            .unwrap_or(128)
    }

    /// Execute a merge job
    pub fn merge(&self, job: &MergeJob) -> Result<PathBuf> {
        self.merge_with_progress(job, &NoOpProgressHandler)
    }

    /// Execute a merge job with progress reporting
    pub fn merge_with_progress(
        &self,
        job: &MergeJob,
        progress_handler: &dyn ProgressHandler,
    ) -> Result<PathBuf> {
        // Validate job
        job.validate()?;

        // Determine target bitrate if not set and in transcode mode
        let target_bitrate = match job.mode {
            MergeMode::Copy => None,
            MergeMode::Transcode => {
                Some(job.target_bitrate.unwrap_or_else(|| {
                    self.detect_bitrate(&job.input_files).unwrap_or(128)
                }))
            }
        };

        info!(
            "Starting merge: {} files -> {} (mode: {:?}, bitrate: {:?})",
            job.input_files.len(),
            job.output_path.display(),
            job.mode,
            target_bitrate
        );

        // Create temporary concat file list
        let temp_file = self.create_concat_file_list(&job.input_files)?;
        let temp_path = temp_file.path().to_path_buf();

        // Ensure temp file cleanup on failure
        let result = self.execute_merge(job, &temp_path, target_bitrate, progress_handler);

        // Clean up temp file
        if let Err(e) = std::fs::remove_file(&temp_path) {
            warn!("Failed to remove temp file {}: {}", temp_path.display(), e);
        }

        result
    }

    /// Create a temporary concat file list for FFmpeg
    fn create_concat_file_list(&self, files: &[AudioFile]) -> Result<NamedTempFile> {
        let mut temp_file = NamedTempFile::new()
            .map_err(|e| MergeError::TempFileCreation(e.to_string()))?;

        // Build concat file list content
        let mut content = String::new();
        for file in files {
            // Get absolute path and escape single quotes
            let abs_path = file
                .path
                .canonicalize()
                .unwrap_or_else(|_| file.path.clone());
            let escaped_path = abs_path.to_string_lossy().replace("'", "'\\''");
            content.push_str(&format!("file '{}'\n", escaped_path));
        }

        // Write to temp file
        std::io::Write::write_all(&mut temp_file, content.as_bytes())?;

        debug!(
            "Created concat file list with {} entries",
            files.len()
        );

        Ok(temp_file)
    }

    /// Execute the actual FFmpeg merge command
    fn execute_merge(
        &self,
        job: &MergeJob,
        concat_list_path: &Path,
        target_bitrate: Option<u32>,
        progress_handler: &dyn ProgressHandler,
    ) -> Result<PathBuf> {
        // Ensure output directory exists
        if let Some(parent) = job.output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Special case: single file can be copied directly without concat
        if job.input_files.len() == 1 {
            return self.copy_single_file(&job.input_files[0], &job.output_path, target_bitrate, progress_handler);
        }

        // Build FFmpeg command
        let mut cmd = Command::new(self.ffmpeg.ffmpeg_path());

        // Add thread count if specified
        if let Some(threads) = job.num_threads {
            cmd.arg("-threads").arg(threads.to_string());
        }

        // Add concat demuxer input
        cmd.arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(concat_list_path);

        // Add codec arguments based on mode
        let codec_args = job.mode.codec_args(target_bitrate);
        for arg in codec_args {
            cmd.arg(arg);
        }

        // Add output path (overwrite if exists)
        cmd.arg("-y").arg(&job.output_path);

        // Setup progress parsing
        let progress_regex = Regex::new(
            r"time=(\d+:\d+:\d+\.\d+)\s+.*?(?:bitrate=\s*([\d.]+)kbits/s)?\s+.*?speed=\s*([\d.]+)x"
        ).map_err(|e| MergeError::ProgressParseError(e.to_string()))?;

        debug!("Running FFmpeg command: {:?}", cmd);

        // Execute FFmpeg with stderr capture for progress
        let mut child = cmd
            .stderr(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| MergeError::OperationFailed(format!("Failed to spawn FFmpeg: {}", e)))?;

        // Parse progress from stderr
        if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);
            let total_files = job.input_files.len();

            for line in reader.lines() {
                let line = line.map_err(MergeError::Io)?;
                trace!("FFmpeg: {}", line);

                // Try to parse progress
                if let Some(captures) = progress_regex.captures(&line) {
                    let time_str = captures.get(1).map(|m| m.as_str()).unwrap_or("00:00:00.00");
                    let bitrate = captures
                        .get(2)
                        .and_then(|m| m.as_str().parse::<f64>().ok())
                        .map(|b| b as u64);
                    let speed = captures
                        .get(3)
                        .and_then(|m| m.as_str().parse::<f64>().ok())
                        .unwrap_or(0.0);

                    if let Ok(time) = parse_ffmpeg_time(time_str) {
                        let progress = MergeProgress {
                            current_file: 1, // FFmpeg concat processes all files
                            total_files,
                            time,
                            speed,
                            bitrate,
                            size: None,
                        };
                        progress_handler.on_progress(progress);
                    }
                }
            }
        }

        // Wait for completion
        let status = child
            .wait()
            .map_err(|e| MergeError::OperationFailed(format!("FFmpeg process error: {}", e)))?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            return Err(MergeError::OperationFailed(format!(
                "FFmpeg exited with code {}",
                code
            )));
        }

        // Verify output file was created
        if !job.output_path.exists() {
            return Err(MergeError::OperationFailed(
                "Output file was not created".to_string(),
            ));
        }

        info!(
            "Merge completed successfully: {} ({} files)",
            job.output_path.display(),
            job.input_files.len()
        );

        Ok(job.output_path.clone())
    }

    /// Copy a single file directly without using concat demuxer
    fn copy_single_file(
        &self,
        file: &AudioFile,
        output_path: &Path,
        target_bitrate: Option<u32>,
        progress_handler: &dyn ProgressHandler,
    ) -> Result<PathBuf> {
        info!("Copying single file: {} -> {}", file.path.display(), output_path.display());

        let mut cmd = Command::new(self.ffmpeg.ffmpeg_path());
        cmd.arg("-i").arg(&file.path);

        // Add codec arguments based on mode and target bitrate
        if let Some(bitrate) = target_bitrate {
            // Transcode mode
            cmd.arg("-c:a").arg("aac")
                .arg("-b:a").arg(format!("{}k", bitrate));
        } else {
            // Copy mode
            cmd.arg("-c").arg("copy");
        }

        cmd.arg("-y").arg(output_path);

        let output = cmd.output()
            .map_err(|e| MergeError::OperationFailed(format!("Failed to execute FFmpeg: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(MergeError::OperationFailed(format!(
                "FFmpeg failed: {}", stderr
            )));
        }

        // Report completion
        progress_handler.on_progress(MergeProgress {
            current_file: 1,
            total_files: 1,
            time: Duration::from_secs(0),
            speed: 1.0,
            bitrate: target_bitrate.map(|b| b as u64 * 1000),
            size: None,
        });

        info!("Single file copy completed: {}", output_path.display());
        Ok(output_path.to_path_buf())
    }

    /// Merge multiple files with automatic mode detection
    pub fn merge_files<P: AsRef<Path>>(
        &self,
        files: &[AudioFile],
        output: P,
    ) -> Result<PathBuf> {
        let job = MergeJob::new(files.to_vec(), output.as_ref().to_path_buf());
        self.merge(&job)
    }

    /// Check if files can be merged without re-encoding
    pub fn can_copy_merge(&self, files: &[AudioFile]) -> bool {
        files
            .iter()
            .all(|f| matches!(f.format, AudioFormat::M4A | AudioFormat::M4B))
    }

    /// Get recommended bitrate for a set of files
    pub fn recommended_bitrate(&self, files: &[AudioFile]) -> u32 {
        self.detect_bitrate(files).unwrap_or(128)
    }
}

/// Parse FFmpeg time format (HH:MM:SS.xx) to Duration
fn parse_ffmpeg_time(time_str: &str) -> Result<Duration> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() != 3 {
        return Err(MergeError::ProgressParseError(format!(
            "Invalid time format: {}",
            time_str
        )));
    }

    let hours: u64 = parts[0]
        .parse()
        .map_err(|_| MergeError::ProgressParseError(format!("Invalid hours: {}", parts[0])))?;
    let minutes: u64 = parts[1]
        .parse()
        .map_err(|_| MergeError::ProgressParseError(format!("Invalid minutes: {}", parts[1])))?;
    let seconds: f64 = parts[2]
        .parse()
        .map_err(|_| MergeError::ProgressParseError(format!("Invalid seconds: {}", parts[2])))?;

    let total_secs = hours * 3600 + minutes * 60 + seconds as u64;
    let nanos = ((seconds.fract()) * 1_000_000_000.0) as u32;

    Ok(Duration::new(total_secs, nanos))
}

/// Builder for creating merge jobs
pub struct MergeJobBuilder {
    input_files: Vec<AudioFile>,
    output_path: Option<PathBuf>,
    mode: Option<MergeMode>,
    target_bitrate: Option<u32>,
    num_threads: Option<usize>,
}

impl MergeJobBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            input_files: Vec::new(),
            output_path: None,
            mode: None,
            target_bitrate: None,
            num_threads: None,
        }
    }

    /// Add input files
    pub fn input_files(mut self, files: Vec<AudioFile>) -> Self {
        self.input_files = files;
        self
    }

    /// Set output path
    pub fn output<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.output_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set merge mode
    pub fn mode(mut self, mode: MergeMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Set target bitrate
    pub fn bitrate(mut self, bitrate: u32) -> Self {
        self.target_bitrate = Some(bitrate);
        self
    }

    /// Set number of threads
    pub fn threads(mut self, threads: usize) -> Self {
        self.num_threads = Some(threads);
        self
    }

    /// Build the merge job
    pub fn build(self) -> Result<MergeJob> {
        let output_path = self.output_path.ok_or(MergeError::NoOutputPath)?;

        let mode = self.mode.unwrap_or_else(|| MergeMode::from_files(&self.input_files));

        Ok(MergeJob {
            input_files: self.input_files,
            output_path,
            mode,
            target_bitrate: self.target_bitrate,
            num_threads: self.num_threads,
        })
    }
}

impl Default for MergeJobBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_audio_file(dir: &TempDir, name: &str, content: &[u8]) -> AudioFile {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content).unwrap();
        
        AudioFile::new(path).unwrap()
    }

    #[test]
    fn test_merge_mode_from_files() {
        let temp_dir = TempDir::new().unwrap();
        
        // All M4A files
        let m4a_files = vec![
            create_test_audio_file(&temp_dir, "file1.m4a", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.m4a", b"dummy"),
        ];
        assert_eq!(MergeMode::from_files(&m4a_files), MergeMode::Copy);

        // All M4B files
        let m4b_files = vec![
            create_test_audio_file(&temp_dir, "file1.m4b", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.m4b", b"dummy"),
        ];
        assert_eq!(MergeMode::from_files(&m4b_files), MergeMode::Copy);

        // Mixed M4A/M4B files
        let mixed_files = vec![
            create_test_audio_file(&temp_dir, "file1.m4a", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.m4b", b"dummy"),
        ];
        assert_eq!(MergeMode::from_files(&mixed_files), MergeMode::Copy);

        // MP3 files
        let mp3_files = vec![
            create_test_audio_file(&temp_dir, "file1.mp3", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.mp3", b"dummy"),
        ];
        assert_eq!(MergeMode::from_files(&mp3_files), MergeMode::Transcode);

        // Mixed MP3/M4A
        let mixed_mp3_m4a = vec![
            create_test_audio_file(&temp_dir, "file1.mp3", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.m4a", b"dummy"),
        ];
        assert_eq!(MergeMode::from_files(&mixed_mp3_m4a), MergeMode::Transcode);
    }

    #[test]
    fn test_merge_mode_codec_args() {
        // Copy mode
        let copy_args = MergeMode::Copy.codec_args(None);
        assert_eq!(copy_args, vec!["-c", "copy"]);

        // Transcode mode with default bitrate
        let transcode_args = MergeMode::Transcode.codec_args(None);
        assert_eq!(transcode_args, vec!["-c:a", "aac", "-b:a", "128k"]);

        // Transcode mode with custom bitrate
        let transcode_args_192 = MergeMode::Transcode.codec_args(Some(192));
        assert_eq!(transcode_args_192, vec!["-c:a", "aac", "-b:a", "192k"]);
    }

    #[test]
    fn test_round_to_standard_bitrate() {
        assert_eq!(Merger::round_to_standard_bitrate(60), 64);
        assert_eq!(Merger::round_to_standard_bitrate(65), 64);
        assert_eq!(Merger::round_to_standard_bitrate(110), 96);
        assert_eq!(Merger::round_to_standard_bitrate(140), 128);
        assert_eq!(Merger::round_to_standard_bitrate(170), 192);
        assert_eq!(Merger::round_to_standard_bitrate(220), 192);
        assert_eq!(Merger::round_to_standard_bitrate(300), 320);
    }

    #[test]
    fn test_parse_ffmpeg_time() {
        let time = parse_ffmpeg_time("00:00:00.00").unwrap();
        assert_eq!(time, Duration::new(0, 0));

        let time = parse_ffmpeg_time("00:01:30.50").unwrap();
        assert_eq!(time.as_secs(), 90);
        assert!(time.subsec_nanos() > 0);

        let time = parse_ffmpeg_time("01:30:45.25").unwrap();
        assert_eq!(time.as_secs(), 5445);
    }

    #[test]
    fn test_merge_job_validation_empty_files() {
        let temp_dir = TempDir::new().unwrap();

        let job = MergeJob {
            input_files: vec![],
            output_path: temp_dir.path().join("output.m4b"),
            mode: MergeMode::Copy,
            target_bitrate: None,
            num_threads: None,
        };
        assert!(matches!(job.validate(), Err(MergeError::NoInputFiles)));
    }

    #[test]
    fn test_merge_job_builder() {
        let temp_dir = TempDir::new().unwrap();
        let file = create_test_audio_file(&temp_dir, "test.mp3", b"dummy");

        let job = MergeJobBuilder::new()
            .input_files(vec![file])
            .output(temp_dir.path().join("output.m4b"))
            .mode(MergeMode::Transcode)
            .bitrate(192)
            .threads(4)
            .build()
            .unwrap();

        assert_eq!(job.mode, MergeMode::Transcode);
        assert_eq!(job.target_bitrate, Some(192));
        assert_eq!(job.num_threads, Some(4));
    }

    #[test]
    fn test_merge_progress() {
        let progress = MergeProgress {
            current_file: 2,
            total_files: 4,
            time: Duration::from_secs(30),
            speed: 32.5,
            bitrate: Some(128000),
            size: Some(1024000),
        };

        assert_eq!(progress.percent_complete(), 0.375); // (2-1)/4 + 0.25/2
    }

    #[test]
    fn test_merger_can_copy_merge() {
        let temp_dir = TempDir::new().unwrap();
        let ffmpeg = Arc::new(FFmpeg::discover().expect("FFmpeg should be available"));
        let merger = Merger::new(ffmpeg);

        // All M4A/M4B files
        let m4_files = vec![
            create_test_audio_file(&temp_dir, "file1.m4a", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.m4b", b"dummy"),
        ];
        assert!(merger.can_copy_merge(&m4_files));

        // Contains MP3
        let mixed_files = vec![
            create_test_audio_file(&temp_dir, "file1.mp3", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.m4a", b"dummy"),
        ];
        assert!(!merger.can_copy_merge(&mixed_files));
    }

    #[test]
    fn test_merge_job_new() {
        let temp_dir = TempDir::new().unwrap();
        let files = vec![
            create_test_audio_file(&temp_dir, "file1.m4a", b"dummy"),
            create_test_audio_file(&temp_dir, "file2.m4a", b"dummy"),
        ];
        let output = temp_dir.path().join("output.m4b");

        let job = MergeJob::new(files, output.clone());

        assert_eq!(job.mode, MergeMode::Copy);
        assert_eq!(job.output_path, output);
        assert_eq!(job.input_files.len(), 2);
    }

    #[test]
    fn test_error_display() {
        let err = MergeError::NoInputFiles;
        assert_eq!(err.to_string(), "No input files provided");

        let err = MergeError::NoOutputPath;
        assert_eq!(err.to_string(), "Output path not specified");
    }
}
