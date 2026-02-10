use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, trace};

/// Errors that can occur when working with FFmpeg
#[derive(Error, Debug)]
pub enum FFmpegError {
    #[error("FFmpeg binary not found at path: {0}")]
    BinaryNotFound(PathBuf),

    #[error("FFmpeg binary not found in PATH or common locations")]
    DiscoveryFailed,

    #[error("Failed to execute FFmpeg command: {0}")]
    ExecutionFailed(String),

    #[error("FFmpeg returned non-zero exit code: {code}, stderr: {stderr}")]
    NonZeroExit { code: i32, stderr: String },

    #[error("Invalid UTF-8 in FFmpeg output: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),

    #[error("Failed to parse FFmpeg output: {0}")]
    ParseError(String),

    #[error("Failed to parse JSON output: {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("No audio stream found in file: {0}")]
    NoAudioStream(PathBuf),

    #[error("Invalid duration format: {0}")]
    InvalidDuration(String),
}

/// Result type for FFmpeg operations
pub type Result<T> = std::result::Result<T, FFmpegError>;

/// Represents a time range (start and end in seconds)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}

impl TimeRange {
    pub fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

/// Audio metadata extracted from ffprobe
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioMetadata {
    pub duration: Option<Duration>,
    pub bitrate: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub codec: Option<String>,
    pub format_name: Option<String>,
    pub format_long_name: Option<String>,
    pub tags: Option<std::collections::HashMap<String, String>>,
}

/// FFmpeg version information
#[derive(Debug, Clone)]
pub struct FFmpegVersion {
    pub version: String,
    pub copyright: String,
    pub built_with: String,
    pub configuration: Vec<String>,
    pub libraries: Vec<LibraryVersion>,
}

impl FFmpegVersion {
    fn parse(output: &str) -> Result<Self> {
        let lines: Vec<&str> = output.lines().collect();
        if lines.is_empty() {
            return Err(FFmpegError::ParseError("Empty version output".to_string()));
        }

        // Parse first line: "ffmpeg version N-71064-gd5e603ddc0-static https://johnvansickle.com/ffmpeg/  Copyright (c) 2000-2024 the FFmpeg developers"
        let first_line = lines[0];
        let version = first_line
            .split("version ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .map(String::from)
            .unwrap_or_else(|| "unknown".to_string());

        // Parse copyright
        let copyright = lines
            .iter()
            .find(|l| l.contains("Copyright"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Parse built with
        let built_with = lines
            .iter()
            .find(|l| l.starts_with("built with"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        // Parse configuration
        let configuration = lines
            .iter()
            .find(|l| l.starts_with("configuration:"))
            .map(|s| {
                s.split("configuration:")
                    .nth(1)
                    .unwrap_or("")
                    .split_whitespace()
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        // Parse library versions
        let mut libraries = Vec::new();
        for line in lines {
            // Library lines look like: "libavutil      59. 27.100 / 59. 27.100"
            // The format is: libname current_version / compiled_version
            let trimmed = line.trim();
            if trimmed.starts_with("lib") && trimmed.contains(" / ") {
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                // Find the position of "/" separator
                if let Some(sep_pos) = parts.iter().position(|&p| p == "/") {
                    if sep_pos >= 2 && parts.len() > sep_pos + 1 {
                        // Join version parts before "/"
                        let current_version = parts[1..sep_pos].join(" ");
                        // Join version parts after "/"
                        let compiled_version = parts[sep_pos + 1..].join(" ");
                        libraries.push(LibraryVersion {
                            name: parts[0].to_string(),
                            current_version,
                            compiled_version,
                        });
                    }
                }
            }
        }

        Ok(Self {
            version,
            copyright,
            built_with,
            configuration,
            libraries,
        })
    }
}

/// FFmpeg library version information
#[derive(Debug, Clone)]
pub struct LibraryVersion {
    pub name: String,
    pub current_version: String,
    pub compiled_version: String,
}

/// Internal struct for parsing ffprobe JSON output
#[derive(Debug, Deserialize)]
struct FFprobeOutput {
    format: Option<FFprobeFormat>,
    streams: Option<Vec<FFprobeStream>>,
}

#[derive(Debug, Deserialize)]
struct FFprobeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
    format_name: Option<String>,
    format_long_name: Option<String>,
    tags: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct FFprobeStream {
    codec_type: String,
    codec_name: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    duration: Option<String>,
    bit_rate: Option<String>,
    #[serde(flatten)]
    extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Main FFmpeg wrapper struct
#[derive(Debug, Clone)]
pub struct FFmpeg {
    ffmpeg_path: PathBuf,
    ffprobe_path: PathBuf,
    version: Option<FFmpegVersion>,
}

impl FFmpeg {
    /// Common paths where FFmpeg might be installed
    const COMMON_PATHS: &'static [&'static str] = &[
        "/usr/bin",
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/opt/local/bin",
        "/usr/lib/ffmpeg/bin",
    ];

    /// Create a new FFmpeg wrapper with explicit paths
    pub fn new<P: AsRef<Path>>(ffmpeg_path: P, ffprobe_path: P) -> Result<Self> {
        let ffmpeg_path = ffmpeg_path.as_ref().to_path_buf();
        let ffprobe_path = ffprobe_path.as_ref().to_path_buf();

        if !ffmpeg_path.exists() {
            return Err(FFmpegError::BinaryNotFound(ffmpeg_path));
        }

        if !ffprobe_path.exists() {
            return Err(FFmpegError::BinaryNotFound(ffprobe_path));
        }

        Ok(Self {
            ffmpeg_path,
            ffprobe_path,
            version: None,
        })
    }

    /// Discover FFmpeg binaries in PATH or common locations
    pub fn discover() -> Result<Self> {
        // Try to find ffmpeg in PATH first
        if let Ok(ffmpeg_path) = Self::find_in_path("ffmpeg") {
            if let Ok(ffprobe_path) = Self::find_in_path("ffprobe") {
                info!("Found FFmpeg in PATH: {}", ffmpeg_path.display());
                return Ok(Self {
                    ffmpeg_path,
                    ffprobe_path,
                    version: None,
                });
            }
        }

        // Try common installation locations
        for dir in Self::COMMON_PATHS {
            let ffmpeg_path = PathBuf::from(dir).join("ffmpeg");
            let ffprobe_path = PathBuf::from(dir).join("ffprobe");

            if ffmpeg_path.exists() && ffprobe_path.exists() {
                info!(
                    "Found FFmpeg in common location: {}",
                    ffmpeg_path.display()
                );
                return Ok(Self {
                    ffmpeg_path,
                    ffprobe_path,
                    version: None,
                });
            }
        }

        Err(FFmpegError::DiscoveryFailed)
    }

    /// Try to find a binary in PATH
    fn find_in_path(binary: &str) -> std::result::Result<PathBuf, std::io::Error> {
        #[cfg(windows)]
        let binary = format!("{}.exe", binary);

        let locator = if cfg!(windows) { "where" } else { "which" };

        let output = Command::new(locator).arg(&binary).output()?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout);
            let path = PathBuf::from(path.trim());
            if path.exists() {
                return Ok(path);
            }
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Binary '{}' not found in PATH", binary),
        ))
    }

    /// Get the ffmpeg binary path
    pub fn ffmpeg_path(&self) -> &Path {
        &self.ffmpeg_path
    }

    /// Get the ffprobe binary path
    pub fn ffprobe_path(&self) -> &Path {
        &self.ffprobe_path
    }

    /// Get FFmpeg version information
    pub fn version(&mut self) -> Result<&FFmpegVersion> {
        if self.version.is_none() {
            let output = Command::new(&self.ffmpeg_path)
                .arg("-version")
                .output()
                .map_err(|e| FFmpegError::ExecutionFailed(e.to_string()))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(FFmpegError::NonZeroExit {
                    code: output.status.code().unwrap_or(-1),
                    stderr: stderr.to_string(),
                });
            }

            let stdout = String::from_utf8(output.stdout)?;
            self.version = Some(FFmpegVersion::parse(&stdout)?);
        }

        Ok(self.version.as_ref().unwrap())
    }

    /// Check if FFmpeg is working by running -version
    pub fn check(&self) -> Result<()> {
        let output = Command::new(&self.ffmpeg_path)
            .arg("-version")
            .output()
            .map_err(|e| FFmpegError::ExecutionFailed(e.to_string()))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(FFmpegError::NonZeroExit {
                code: output.status.code().unwrap_or(-1),
                stderr: stderr.to_string(),
            })
        }
    }

    /// Probe a file and return raw ffprobe JSON output
    pub fn probe(&self, path: &Path) -> Result<serde_json::Value> {
        if !path.exists() {
            return Err(FFmpegError::FileNotFound(path.to_path_buf()));
        }

        let output = Command::new(&self.ffprobe_path)
            .arg("-v")
            .arg("error")
            .arg("-show_format")
            .arg("-show_streams")
            .arg("-of")
            .arg("json")
            .arg(path)
            .output()
            .map_err(|e| FFmpegError::ExecutionFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(FFmpegError::NonZeroExit {
                code: output.status.code().unwrap_or(-1),
                stderr: stderr.to_string(),
            });
        }

        let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        Ok(json)
    }

    /// Get audio metadata from a file
    pub fn get_metadata(&self, path: &Path) -> Result<AudioMetadata> {
        let probe = self.probe(path)?;
        let ffprobe: FFprobeOutput = serde_json::from_value(probe)?;

        let mut metadata = AudioMetadata::default();

        // Extract format-level info
        if let Some(format) = ffprobe.format {
            metadata.format_name = format.format_name;
            metadata.format_long_name = format.format_long_name;
            metadata.tags = format.tags;

            if let Some(duration_str) = format.duration {
                if let Ok(secs) = duration_str.parse::<f64>() {
                    metadata.duration = Some(Duration::from_secs_f64(secs));
                }
            }

            if let Some(bitrate_str) = format.bit_rate {
                if let Ok(bitrate) = bitrate_str.parse::<u64>() {
                    metadata.bitrate = Some(bitrate);
                }
            }
        }

        // Extract audio stream info
        if let Some(streams) = ffprobe.streams {
            for stream in streams {
                if stream.codec_type == "audio" {
                    metadata.codec = stream.codec_name;
                    metadata.channels = stream.channels;

                    if let Some(sample_rate) = stream.sample_rate {
                        if let Ok(rate) = sample_rate.parse::<u32>() {
                            metadata.sample_rate = Some(rate);
                        }
                    }

                    // Use stream duration if format duration wasn't available
                    if metadata.duration.is_none() {
                        if let Some(duration_str) = stream.duration {
                            if let Ok(secs) = duration_str.parse::<f64>() {
                                metadata.duration = Some(Duration::from_secs_f64(secs));
                            }
                        }
                    }

                    // Use stream bitrate if format bitrate wasn't available
                    if metadata.bitrate.is_none() {
                        if let Some(bitrate_str) = stream.bit_rate {
                            if let Ok(bitrate) = bitrate_str.parse::<u64>() {
                                metadata.bitrate = Some(bitrate);
                            }
                        }
                    }

                    break; // Only use first audio stream
                }
            }
        }

        Ok(metadata)
    }

    /// Get duration of an audio file
    pub fn get_duration(&self, path: &Path) -> Result<Duration> {
        let metadata = self.get_metadata(path)?;
        metadata
            .duration
            .ok_or_else(|| FFmpegError::NoAudioStream(path.to_path_buf()))
    }

    /// Detect silence periods in an audio file
    /// 
    /// # Arguments
    /// * `path` - Path to the audio file
    /// * `noise_db` - Noise threshold in dB (e.g., -50.0)
    /// * `min_duration` - Minimum silence duration in seconds
    pub fn detect_silence(
        &self,
        path: &Path,
        noise_db: f64,
        min_duration: f64,
    ) -> Result<Vec<TimeRange>> {
        if !path.exists() {
            return Err(FFmpegError::FileNotFound(path.to_path_buf()));
        }

        let silencedetect_filter = format!(
            "silencedetect=noise={}dB:d={}",
            noise_db, min_duration
        );

        let output = Command::new(&self.ffmpeg_path)
            .arg("-i")
            .arg(path)
            .arg("-af")
            .arg(&silencedetect_filter)
            .arg("-f")
            .arg("null")
            .arg("-")
            .output()
            .map_err(|e| FFmpegError::ExecutionFailed(e.to_string()))?;

        // FFmpeg outputs silence detection info to stderr even on success
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut silence_periods = Vec::new();

        // Parse silence detection output
        // Format:
        // [silencedetect @ ...] silence_start: 123.456
        // [silencedetect @ ...] silence_end: 456.789 | silence_duration: 333.333

        let mut current_start: Option<f64> = None;

        for line in stderr.lines() {
            trace!("FFmpeg stderr: {}", line);

            if line.contains("silence_start:") {
                if let Some(start_str) = line.split("silence_start:").nth(1) {
                    if let Ok(start) = start_str.trim().parse::<f64>() {
                        current_start = Some(start);
                    }
                }
            } else if line.contains("silence_end:") {
                if let Some(start) = current_start.take() {
                    if let Some(end_str) = line.split("silence_end:").nth(1) {
                        if let Some(end_part) = end_str.split('|').next() {
                            if let Ok(end) = end_part.trim().parse::<f64>() {
                                silence_periods.push(TimeRange::new(start, end));
                            }
                        }
                    }
                }
            }
        }

        info!(
            "Detected {} silence periods in {}",
            silence_periods.len(),
            path.display()
        );

        Ok(silence_periods)
    }

    /// Prepare a concat file list for FFmpeg's concat demuxer
    /// 
    /// The concat demuxer expects a file with lines like:
    /// file 'path/to/file1.mp3'
    /// file 'path/to/file2.mp3'
    ///
    /// # Arguments
    /// * `files` - List of file paths to concatenate
    /// * `output_path` - Where to write the concat file list
    pub fn prepare_concat_file_list<P: AsRef<Path>>(
        &self,
        files: &[P],
        output_path: &Path,
    ) -> Result<()> {
        if files.is_empty() {
            return Err(FFmpegError::ParseError(
                "Empty file list for concat".to_string(),
            ));
        }

        let mut content = String::new();

        for file in files {
            let path = file.as_ref();
            if !path.exists() {
                return Err(FFmpegError::FileNotFound(path.to_path_buf()));
            }

            // Escape single quotes in path by replacing ' with '\''
            let escaped_path = path.to_string_lossy().replace("'", "'\\''");
            content.push_str(&format!("file '{}'\n", escaped_path));
        }

        std::fs::write(output_path, content)?;

        debug!(
            "Created concat file list at {} with {} entries",
            output_path.display(),
            files.len()
        );

        Ok(())
    }

    /// Create a concat file list and return the content as a string
    /// This is useful for temporary concat lists that don't need to be saved
    pub fn create_concat_file_list<P: AsRef<Path>>(
        &self,
        files: &[P],
    ) -> Result<String> {
        if files.is_empty() {
            return Err(FFmpegError::ParseError(
                "Empty file list for concat".to_string(),
            ));
        }

        let mut content = String::new();

        for file in files {
            let path = file.as_ref();
            if !path.exists() {
                return Err(FFmpegError::FileNotFound(path.to_path_buf()));
            }

            // Escape single quotes in path by replacing ' with '\''
            let escaped_path = path.to_string_lossy().replace("'", "'\\''");
            content.push_str(&format!("file '{}'\n", escaped_path));
        }

        Ok(content)
    }

    /// Get information about all available audio codecs
    pub fn get_audio_codecs(&self) -> Result<Vec<String>> {
        let output = Command::new(&self.ffmpeg_path)
            .args(&["-codecs", "-hide_banner"])
            .output()
            .map_err(|e| FFmpegError::ExecutionFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(FFmpegError::NonZeroExit {
                code: output.status.code().unwrap_or(-1),
                stderr: stderr.to_string(),
            });
        }

        let stdout = String::from_utf8(output.stdout)?;
        let mut codecs = Vec::new();

        for line in stdout.lines() {
            // Lines look like: " DEA aac                  AAC (Advanced Audio Coding)"
            // We want lines that start with space and have 'A' (audio) in the type field
            if line.len() > 8 && line.starts_with(' ') && line.chars().nth(4) == Some('A') {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    codecs.push(parts[1].to_string());
                }
            }
        }

        Ok(codecs)
    }

    /// Check if a codec is available
    pub fn has_codec(&self, codec: &str) -> Result<bool> {
        let codecs = self.get_audio_codecs()?;
        Ok(codecs.iter().any(|c| c == codec))
    }
}

impl Default for FFmpeg {
    fn default() -> Self {
        // Try to discover FFmpeg, panic if not found
        Self::discover().expect("FFmpeg not found in PATH or common locations")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_ffmpeg() -> FFmpeg {
        FFmpeg::discover().expect("FFmpeg should be available for tests")
    }

    #[test]
    fn test_discovery() {
        let result = FFmpeg::discover();
        assert!(result.is_ok(), "FFmpeg should be discoverable in test environment");
        
        let ffmpeg = result.unwrap();
        assert!(ffmpeg.ffmpeg_path.exists());
        assert!(ffmpeg.ffprobe_path.exists());
    }

    #[test]
    fn test_check() {
        let ffmpeg = create_test_ffmpeg();
        assert!(ffmpeg.check().is_ok());
    }

    #[test]
    fn test_version() {
        let mut ffmpeg = create_test_ffmpeg();
        let version = ffmpeg.version();
        assert!(version.is_ok());
        
        let version = version.unwrap();
        assert!(!version.version.is_empty());
        assert!(!version.libraries.is_empty());
    }

    #[test]
    fn test_probe_nonexistent_file() {
        let ffmpeg = create_test_ffmpeg();
        let result = ffmpeg.probe(Path::new("/nonexistent/file.mp3"));
        assert!(matches!(result, Err(FFmpegError::FileNotFound(_))));
    }

    #[test]
    fn test_get_duration_nonexistent_file() {
        let ffmpeg = create_test_ffmpeg();
        let result = ffmpeg.get_duration(Path::new("/nonexistent/file.mp3"));
        assert!(matches!(result, Err(FFmpegError::FileNotFound(_))));
    }

    #[test]
    fn test_prepare_concat_file_list_empty() {
        let ffmpeg = create_test_ffmpeg();
        let temp_file = NamedTempFile::new().unwrap();
        let files: &[&Path] = &[];
        let result = ffmpeg.prepare_concat_file_list(files, temp_file.path());
        assert!(matches!(result, Err(FFmpegError::ParseError(_))));
    }

    #[test]
    fn test_prepare_concat_file_list_nonexistent() {
        let ffmpeg = create_test_ffmpeg();
        let temp_file = NamedTempFile::new().unwrap();
        let files = vec![Path::new("/nonexistent/file.mp3")];
        let result = ffmpeg.prepare_concat_file_list(&files, temp_file.path());
        assert!(matches!(result, Err(FFmpegError::FileNotFound(_))));
    }

    #[test]
    fn test_time_range() {
        let range = TimeRange::new(10.0, 20.0);
        assert_eq!(range.start, 10.0);
        assert_eq!(range.end, 20.0);
        assert_eq!(range.duration(), 10.0);
    }

    #[test]
    fn test_create_concat_file_list() {
        let ffmpeg = create_test_ffmpeg();
        
        // Create temporary files
        let mut temp_file1 = NamedTempFile::new().unwrap();
        let mut temp_file2 = NamedTempFile::new().unwrap();
        writeln!(temp_file1, "test1").unwrap();
        writeln!(temp_file2, "test2").unwrap();

        let files = vec![temp_file1.path(), temp_file2.path()];
        let result = ffmpeg.create_concat_file_list(&files);
        
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("file '"));
        assert!(content.contains(&temp_file1.path().to_string_lossy().to_string()));
        assert!(content.contains(&temp_file2.path().to_string_lossy().to_string()));
    }

    #[test]
    fn test_has_codec_aac() {
        let ffmpeg = create_test_ffmpeg();
        // AAC should be available in any reasonable FFmpeg build
        let has_aac = ffmpeg.has_codec("aac");
        assert!(has_aac.is_ok());
        // We can't guarantee it exists, but the function should work
    }

    #[test]
    fn test_error_display() {
        let err = FFmpegError::DiscoveryFailed;
        assert_eq!(
            err.to_string(),
            "FFmpeg binary not found in PATH or common locations"
        );

        let err = FFmpegError::FileNotFound(PathBuf::from("/test"));
        assert!(err.to_string().contains("/test"));
    }

    #[test]
    fn test_ffmpeg_version_parse() {
        let version_output = r#"ffmpeg version N-71064-gd5e603ddc0-static https://johnvansickle.com/ffmpeg/  Copyright (c) 2000-2024 the FFmpeg developers
built with gcc 8 (Debian 8.3.0-6)
configuration: --enable-gpl
libavutil      59. 27.100 / 59. 27.100
libavcodec     61.  9.100 / 61.  9.100"#;

        let version = FFmpegVersion::parse(version_output).unwrap();
        assert_eq!(version.version, "N-71064-gd5e603ddc0-static");
        assert!(!version.libraries.is_empty());
        assert_eq!(version.libraries[0].name, "libavutil");
    }
}
