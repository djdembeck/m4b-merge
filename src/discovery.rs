use crate::audio::{AudioMetadata, FFmpeg, FFmpegError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

/// Errors that can occur during file discovery
#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Permission denied: {0}")]
    PermissionDenied(PathBuf),

    #[error("Invalid file format: {0}")]
    InvalidFormat(PathBuf),

    #[error("Unsupported file extension: {0}")]
    UnsupportedExtension(String),

    #[error("Path is not a file or directory: {0}")]
    InvalidPath(PathBuf),

    #[error("Directory not found: {0}")]
    DirectoryNotFound(PathBuf),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("FFmpeg error: {0}")]
    FFmpegError(#[from] FFmpegError),

    #[error("No audio files found in: {0}")]
    NoAudioFiles(PathBuf),

    #[error("Failed to read metadata for: {0}")]
    MetadataError(PathBuf),
}

/// Result type for discovery operations
pub type Result<T> = std::result::Result<T, DiscoveryError>;

/// Supported audio formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioFormat {
    MP3,
    M4A,
    M4B,
}

impl AudioFormat {
    /// Get file extensions for this format
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            AudioFormat::MP3 => &["mp3"],
            AudioFormat::M4A => &["m4a"],
            AudioFormat::M4B => &["m4b"],
        }
    }

    /// Get all supported extensions
    pub fn all_extensions() -> &'static [&'static str] {
        &["mp3", "m4a", "m4b"]
    }

    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_lowercase();
        match ext.as_str() {
            "mp3" => Some(AudioFormat::MP3),
            "m4a" => Some(AudioFormat::M4A),
            "m4b" => Some(AudioFormat::M4B),
            _ => None,
        }
    }

    /// Detect format from a path
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension().and_then(|e| e.to_str()).and_then(Self::from_extension)
    }
}

impl fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioFormat::MP3 => write!(f, "MP3"),
            AudioFormat::M4A => write!(f, "M4A"),
            AudioFormat::M4B => write!(f, "M4B"),
        }
    }
}

/// File metadata extracted from audio file
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileMetadata {
    pub duration: Duration,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub codec: String,
}

impl From<AudioMetadata> for FileMetadata {
    fn from(metadata: AudioMetadata) -> Self {
        Self {
            duration: metadata.duration.unwrap_or_default(),
            bitrate: metadata.bitrate.unwrap_or(0).try_into().unwrap_or(u32::MAX),
            sample_rate: metadata.sample_rate.unwrap_or(0),
            channels: metadata.channels.unwrap_or(0) as u8,
            codec: metadata.codec.unwrap_or_default(),
        }
    }
}

/// Represents a discovered audio file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFile {
    pub path: PathBuf,
    pub format: AudioFormat,
    pub metadata: Option<FileMetadata>,
}

impl AudioFile {
    /// Create a new AudioFile from a path
    pub fn new(path: PathBuf) -> Result<Self> {
        // Check if file exists
        if !path.exists() {
            return Err(DiscoveryError::FileNotFound(path));
        }

        // Check if it's a file
        if !path.is_file() {
            return Err(DiscoveryError::InvalidPath(path));
        }

        // Check permissions by attempting to open the file
        match std::fs::File::open(&path) {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(DiscoveryError::PermissionDenied(path));
            }
            Err(e) => return Err(DiscoveryError::IoError(e)),
        }

        // Detect format from extension
        let format = AudioFormat::from_path(&path)
            .ok_or_else(|| DiscoveryError::InvalidFormat(path.clone()))?;

        Ok(Self { path, format, metadata: None })
    }

    /// Probe and populate metadata using FFmpeg
    pub fn probe_metadata(&mut self, ffmpeg: &FFmpeg) -> Result<()> {
        let audio_metadata = ffmpeg.get_metadata(&self.path)?;
        self.metadata = Some(FileMetadata::from(audio_metadata));
        Ok(())
    }

    /// Get the filename as a string
    pub fn filename(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    }

    /// Get the parent directory
    pub fn parent(&self) -> Option<&Path> {
        self.path.parent()
    }
}

/// Represents a group of audio files (e.g., from one directory or disc)
#[derive(Debug, Clone)]
pub struct AudioGroup {
    pub name: String,
    pub files: Vec<AudioFile>,
    pub disc_number: Option<u32>,
}

impl AudioGroup {
    pub fn new(name: String, files: Vec<AudioFile>) -> Self {
        let disc_number = detect_disc_number(&name);
        Self { name, files, disc_number }
    }

    /// Get total duration of all files in this group
    pub fn total_duration(&self) -> Duration {
        self.files
            .iter()
            .filter_map(|f| f.metadata.as_ref().map(|m| m.duration))
            .fold(Duration::ZERO, |acc, d| acc + d)
    }

    /// Sort files naturally by filename
    pub fn sort_naturally(&mut self) {
        self.files.sort_by(|a, b| {
            let a_name = a.filename();
            let b_name = b.filename();
            natord::compare(&a_name, &b_name)
        });
    }
}

/// Multi-disc detection patterns
const DISC_PATTERNS: &[&str] = &[
    r"(?i)^CD\s*(\d+)",     // CD1, CD 1, CD01
    r"(?i)^DISC\s*(\d+)",   // Disc1, Disc 1, DISC 01
    r"(?i)^DISK\s*(\d+)",   // Disk1, Disk 1, DISK 01
    r"(?i)^PART\s*(\d+)",   // Part1, Part 1, PART 01
    r"(?i)^DISC[_-]?(\d+)", // Disc_1, Disc-1
    r"(?i)^CD[_-]?(\d+)",   // CD_1, CD-1
];

/// Precompiled regex patterns for disc number detection
static DISC_REGEX_CACHE: OnceLock<Vec<Regex>> = OnceLock::new();

/// Get or initialize the precompiled regex patterns
fn get_disc_regexes() -> &'static Vec<Regex> {
    DISC_REGEX_CACHE.get_or_init(|| {
        DISC_PATTERNS.iter().filter_map(|pattern| Regex::new(pattern).ok()).collect()
    })
}

/// Detect disc number from directory name
fn detect_disc_number(name: &str) -> Option<u32> {
    for re in get_disc_regexes() {
        if let Some(captures) = re.captures(name) {
            if let Some(num_match) = captures.get(1) {
                if let Ok(num) = num_match.as_str().parse::<u32>() {
                    return Some(num);
                }
            }
        }
    }
    None
}

/// Check if a directory appears to contain multi-disc subdirectories
fn has_multi_disc_subdirs(dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if detect_disc_number(name).is_some() {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Collect all audio files from a directory
fn collect_files_from_dir(dir: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in walkdir::WalkDir::new(dir)
        .max_depth(if recursive { usize::MAX } else { 1 })
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            if AudioFormat::from_path(path).is_some() {
                files.push(path.to_path_buf());
            }
        }
    }

    Ok(files)
}

/// Validate that a file is a supported audio file
fn validate_audio_file(path: &Path) -> Result<()> {
    // Check file exists
    if !path.exists() {
        return Err(DiscoveryError::FileNotFound(path.to_path_buf()));
    }

    // Check it's a file
    if !path.is_file() {
        return Err(DiscoveryError::InvalidPath(path.to_path_buf()));
    }

    // Check extension
    if AudioFormat::from_path(path).is_none() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(String::from)
            .unwrap_or_else(|| "unknown".to_string());
        return Err(DiscoveryError::UnsupportedExtension(ext));
    }

    // Check permissions (try to open for read)
    match std::fs::File::open(path) {
        Ok(_) => (),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(DiscoveryError::PermissionDenied(path.to_path_buf()));
        }
        Err(e) => return Err(DiscoveryError::IoError(e)),
    }

    Ok(())
}

/// Discover audio files from a single path (file or directory)
fn discover_from_path(path: &Path) -> Result<Vec<AudioFile>> {
    if path.is_file() {
        // Single file
        validate_audio_file(path)?;
        let file = AudioFile::new(path.to_path_buf())?;
        Ok(vec![file])
    } else if path.is_dir() {
        // Directory
        if has_multi_disc_subdirs(path) {
            // Multi-disc structure - collect from all disc subdirectories
            discover_multi_disc_dir(path)
        } else {
            // Regular directory - collect all audio files
            let paths = collect_files_from_dir(path, false)?;
            let mut files = Vec::new();
            for p in paths {
                if let Ok(file) = AudioFile::new(p) {
                    files.push(file);
                }
            }
            if files.is_empty() {
                return Err(DiscoveryError::NoAudioFiles(path.to_path_buf()));
            }
            Ok(files)
        }
    } else {
        Err(DiscoveryError::InvalidPath(path.to_path_buf()))
    }
}

/// Discover files from a multi-disc directory structure
fn discover_multi_disc_dir(dir: &Path) -> Result<Vec<AudioFile>> {
    let mut all_files: Vec<(Option<u32>, AudioFile)> = Vec::new();

    // Read directory entries
    let entries = std::fs::read_dir(dir).map_err(|e| DiscoveryError::IoError(e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();

        // Check if this is a disc directory
        let disc_num = detect_disc_number(&name);

        // Collect files from this subdirectory
        let files = collect_files_from_dir(&path, false)?;
        for p in files {
            if let Ok(file) = AudioFile::new(p) {
                all_files.push((disc_num, file));
            }
        }
    }

    // Sort by disc number (None last), then naturally by filename
    all_files.sort_by(|a, b| {
        match (a.0, b.0) {
            (Some(n1), Some(n2)) => n1.cmp(&n2),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| natord::compare(&a.1.filename(), &b.1.filename()))
    });

    if all_files.is_empty() {
        return Err(DiscoveryError::NoAudioFiles(dir.to_path_buf()));
    }

    Ok(all_files.into_iter().map(|(_, f)| f).collect())
}

/// Group files by their parent directory
pub fn group_files_by_directory(files: Vec<AudioFile>) -> Vec<AudioGroup> {
    let mut groups: HashMap<PathBuf, Vec<AudioFile>> = HashMap::new();

    for file in files {
        if let Some(parent) = file.parent() {
            groups.entry(parent.to_path_buf()).or_default().push(file);
        }
    }

    groups
        .into_iter()
        .map(|(dir, files)| {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            AudioGroup::new(name, files)
        })
        .collect()
}

/// Main discovery function: discover audio files from multiple input paths
///
/// # Arguments
/// * `paths` - List of paths to files or directories to scan
///
/// # Returns
/// A vector of AudioFile structs, naturally sorted within each directory
pub fn discover_files(paths: &[PathBuf]) -> Result<Vec<AudioFile>> {
    let mut all_files = Vec::new();

    for path in paths {
        info!("Discovering files from: {}", path.display());

        match discover_from_path(path) {
            Ok(files) => {
                debug!("Found {} files in {}", files.len(), path.display());
                all_files.extend(files);
            }
            Err(e) => {
                error!("Error discovering files from {}: {}", path.display(), e);
                return Err(e);
            }
        }
    }

    // Sort files naturally by their full path
    all_files.sort_by(|a, b| natord::compare(&a.path.to_string_lossy(), &b.path.to_string_lossy()));

    info!("Total files discovered: {}", all_files.len());
    Ok(all_files)
}

/// Discover files and group by directory for batch processing
pub fn discover_and_group(paths: &[PathBuf]) -> Result<Vec<AudioGroup>> {
    let files = discover_files(paths)?;
    let mut groups = group_files_by_directory(files);

    // Sort each group naturally
    for group in &mut groups {
        group.sort_naturally();
    }

    // Sort groups by disc number if present
    groups.sort_by(|a, b| match (a.disc_number, b.disc_number) {
        (Some(n1), Some(n2)) => n1.cmp(&n2),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    Ok(groups)
}

/// Discover files and probe metadata for each
pub fn discover_with_metadata(paths: &[PathBuf], ffmpeg: &FFmpeg) -> Result<Vec<AudioFile>> {
    let mut files = discover_files(paths)?;

    for file in &mut files {
        if let Err(e) = file.probe_metadata(ffmpeg) {
            warn!("Failed to probe metadata for {}: {}", file.path.display(), e);
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_file(dir: &TempDir, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content).unwrap();
        path
    }

    #[test]
    fn test_audio_format_from_extension() {
        assert_eq!(AudioFormat::from_extension("mp3"), Some(AudioFormat::MP3));
        assert_eq!(AudioFormat::from_extension("MP3"), Some(AudioFormat::MP3));
        assert_eq!(AudioFormat::from_extension("m4a"), Some(AudioFormat::M4A));
        assert_eq!(AudioFormat::from_extension("m4b"), Some(AudioFormat::M4B));
        assert_eq!(AudioFormat::from_extension("wav"), None);
        assert_eq!(AudioFormat::from_extension("ogg"), None);
    }

    #[test]
    fn test_audio_format_from_path() {
        assert_eq!(AudioFormat::from_path(Path::new("/path/to/file.mp3")), Some(AudioFormat::MP3));
        assert_eq!(AudioFormat::from_path(Path::new("/path/to/file.m4b")), Some(AudioFormat::M4B));
        assert_eq!(AudioFormat::from_path(Path::new("/path/to/file.txt")), None);
    }

    #[test]
    fn test_audio_format_display() {
        assert_eq!(format!("{}", AudioFormat::MP3), "MP3");
        assert_eq!(format!("{}", AudioFormat::M4A), "M4A");
        assert_eq!(format!("{}", AudioFormat::M4B), "M4B");
    }

    #[test]
    fn test_detect_disc_number() {
        assert_eq!(detect_disc_number("CD1"), Some(1));
        assert_eq!(detect_disc_number("CD 1"), Some(1));
        assert_eq!(detect_disc_number("CD01"), Some(1));
        assert_eq!(detect_disc_number("CD10"), Some(10));
        assert_eq!(detect_disc_number("Disc 2"), Some(2));
        assert_eq!(detect_disc_number("DISC3"), Some(3));
        assert_eq!(detect_disc_number("Disk 1"), Some(1));
        assert_eq!(detect_disc_number("Part 1"), Some(1));
        assert_eq!(detect_disc_number("Disc_1"), Some(1));
        assert_eq!(detect_disc_number("CD-1"), Some(1));
        assert_eq!(detect_disc_number("Some Book"), None);
        assert_eq!(detect_disc_number("Chapter 1"), None);
    }

    #[test]
    fn test_has_multi_disc_subdirs() {
        let temp_dir = TempDir::new().unwrap();
        let disc1_dir = temp_dir.path().join("CD1");
        let disc2_dir = temp_dir.path().join("CD2");
        std::fs::create_dir(&disc1_dir).unwrap();
        std::fs::create_dir(&disc2_dir).unwrap();

        assert!(has_multi_disc_subdirs(temp_dir.path()));

        // Clean up and test without multi-disc
        std::fs::remove_dir_all(&disc1_dir).unwrap();
        std::fs::remove_dir_all(&disc2_dir).unwrap();
        let regular_dir = temp_dir.path().join("RegularDir");
        std::fs::create_dir(&regular_dir).unwrap();

        assert!(!has_multi_disc_subdirs(temp_dir.path()));
    }

    #[test]
    fn test_audio_file_new() {
        let temp_dir = TempDir::new().unwrap();
        let path = create_test_file(&temp_dir, "test.mp3", b"dummy mp3 content");

        let audio_file = AudioFile::new(path.clone());
        assert!(audio_file.is_ok());

        let file = audio_file.unwrap();
        assert_eq!(file.format, AudioFormat::MP3);
        assert_eq!(file.filename(), "test.mp3");
        assert!(file.metadata.is_none());
    }

    #[test]
    fn test_audio_file_new_nonexistent() {
        let path = PathBuf::from("/nonexistent/file.mp3");
        let result = AudioFile::new(path);
        assert!(matches!(result, Err(DiscoveryError::FileNotFound(_))));
    }

    #[test]
    fn test_audio_file_new_wrong_extension() {
        let temp_dir = TempDir::new().unwrap();
        let path = create_test_file(&temp_dir, "test.txt", b"text content");

        let result = AudioFile::new(path);
        assert!(matches!(result, Err(DiscoveryError::InvalidFormat(_))));
    }

    #[test]
    fn test_validate_audio_file() {
        let temp_dir = TempDir::new().unwrap();
        let valid_path = create_test_file(&temp_dir, "test.mp3", b"content");
        let invalid_ext = create_test_file(&temp_dir, "test.txt", b"content");

        assert!(validate_audio_file(&valid_path).is_ok());
        assert!(matches!(
            validate_audio_file(&invalid_ext),
            Err(DiscoveryError::UnsupportedExtension(_))
        ));
        assert!(matches!(
            validate_audio_file(Path::new("/nonexistent.mp3")),
            Err(DiscoveryError::FileNotFound(_))
        ));
    }

    #[test]
    fn test_collect_files_from_dir() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(&temp_dir, "01.mp3", b"content");
        create_test_file(&temp_dir, "02.mp3", b"content");
        create_test_file(&temp_dir, "03.m4a", b"content");
        create_test_file(&temp_dir, "readme.txt", b"text");

        let files = collect_files_from_dir(temp_dir.path(), false).unwrap();
        assert_eq!(files.len(), 3);

        // Check that only audio files were collected
        let extensions: Vec<_> =
            files.iter().filter_map(|p| p.extension().and_then(|e| e.to_str())).collect();
        assert!(extensions.contains(&"mp3"));
        assert!(extensions.contains(&"m4a"));
        assert!(!extensions.contains(&"txt"));
    }

    #[test]
    fn test_discover_from_path_single_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_file(&temp_dir, "single.mp3", b"content");

        let files = discover_from_path(&file_path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename(), "single.mp3");
    }

    #[test]
    fn test_discover_from_path_directory() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(&temp_dir, "01.mp3", b"content");
        create_test_file(&temp_dir, "02.mp3", b"content");
        create_test_file(&temp_dir, "03.m4a", b"content");

        let files = discover_from_path(temp_dir.path()).unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_discover_multi_disc_dir() {
        let temp_dir = TempDir::new().unwrap();

        // Create disc subdirectories
        let disc1 = temp_dir.path().join("CD1");
        let disc2 = temp_dir.path().join("CD2");
        std::fs::create_dir(&disc1).unwrap();
        std::fs::create_dir(&disc2).unwrap();

        // Create files in each disc
        create_test_file(&temp_dir, "CD1/01.mp3", b"content");
        create_test_file(&temp_dir, "CD1/02.mp3", b"content");
        create_test_file(&temp_dir, "CD2/01.mp3", b"content");
        create_test_file(&temp_dir, "CD2/02.mp3", b"content");

        let files = discover_from_path(temp_dir.path()).unwrap();
        assert_eq!(files.len(), 4);

        // Files should be sorted by disc, then naturally
        // CD1 files should come before CD2 files
        let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
        assert!(paths[0].to_string_lossy().contains("CD1"));
        assert!(paths[1].to_string_lossy().contains("CD1"));
        assert!(paths[2].to_string_lossy().contains("CD2"));
        assert!(paths[3].to_string_lossy().contains("CD2"));
    }

    #[test]
    fn test_discover_files_multiple_paths() {
        let temp_dir = TempDir::new().unwrap();
        let subdir1 = temp_dir.path().join("dir1");
        let subdir2 = temp_dir.path().join("dir2");
        std::fs::create_dir(&subdir1).unwrap();
        std::fs::create_dir(&subdir2).unwrap();

        create_test_file(&temp_dir, "dir1/01.mp3", b"content");
        create_test_file(&temp_dir, "dir2/02.m4a", b"content");

        let paths = vec![subdir1, subdir2];
        let files = discover_files(&paths).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_natural_sorting() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(&temp_dir, "10.mp3", b"content");
        create_test_file(&temp_dir, "01.mp3", b"content");
        create_test_file(&temp_dir, "02.mp3", b"content");
        create_test_file(&temp_dir, "100.mp3", b"content");

        let files = discover_from_path(temp_dir.path()).unwrap();
        let sorted: Vec<_> = files.iter().map(|f| f.filename()).collect();

        // Should be sorted naturally: 01, 02, 10, 100
        // Not lexicographically: 01, 02, 10, 100 (same in this case)
        // But let's test a clearer case
        assert_eq!(sorted.len(), 4);

        // Test with a fresh directory with different names
        let temp_dir2 = TempDir::new().unwrap();
        create_test_file(&temp_dir2, "file_10.mp3", b"content");
        create_test_file(&temp_dir2, "file_1.mp3", b"content");
        create_test_file(&temp_dir2, "file_2.mp3", b"content");

        // Use discover_files which applies natural sorting
        let files2 = discover_files(&[temp_dir2.path().to_path_buf()]).unwrap();
        let sorted2: Vec<_> = files2.iter().map(|f| f.filename()).collect();

        // Natural sort: file_1, file_2, file_10
        // Lexicographic: file_1, file_10, file_2
        assert_eq!(sorted2[0], "file_1.mp3");
        assert_eq!(sorted2[1], "file_2.mp3");
        assert_eq!(sorted2[2], "file_10.mp3");
    }

    #[test]
    fn test_group_files_by_directory() {
        let temp_dir = TempDir::new().unwrap();
        let dir1 = temp_dir.path().join("Book1");
        let dir2 = temp_dir.path().join("Book2");
        std::fs::create_dir(&dir1).unwrap();
        std::fs::create_dir(&dir2).unwrap();

        let file1 =
            AudioFile::new(create_test_file(&temp_dir, "Book1/chapter1.mp3", b"content")).unwrap();
        let file2 =
            AudioFile::new(create_test_file(&temp_dir, "Book1/chapter2.mp3", b"content")).unwrap();
        let file3 =
            AudioFile::new(create_test_file(&temp_dir, "Book2/chapter1.mp3", b"content")).unwrap();

        let files = vec![file1, file2, file3];
        let groups = group_files_by_directory(files);

        assert_eq!(groups.len(), 2);

        let group_names: Vec<_> = groups.iter().map(|g| g.name.clone()).collect();
        assert!(group_names.contains(&"Book1".to_string()));
        assert!(group_names.contains(&"Book2".to_string()));

        // Find Book1 group
        let book1_group = groups.iter().find(|g| g.name == "Book1").unwrap();
        assert_eq!(book1_group.files.len(), 2);
    }

    #[test]
    fn test_audio_group_sort_naturally() {
        let temp_dir = TempDir::new().unwrap();

        let files = vec![
            AudioFile::new(create_test_file(&temp_dir, "10.mp3", b"content")).unwrap(),
            AudioFile::new(create_test_file(&temp_dir, "1.mp3", b"content")).unwrap(),
            AudioFile::new(create_test_file(&temp_dir, "2.mp3", b"content")).unwrap(),
        ];

        let mut group = AudioGroup::new("Test".to_string(), files.clone());
        group.sort_naturally();

        let sorted: Vec<_> = group.files.iter().map(|f| f.filename()).collect();
        assert_eq!(sorted, vec!["1.mp3", "2.mp3", "10.mp3"]);
    }

    #[test]
    fn test_audio_group_disc_number() {
        let group1 = AudioGroup::new("CD1".to_string(), vec![]);
        assert_eq!(group1.disc_number, Some(1));

        let group2 = AudioGroup::new("Disc 2".to_string(), vec![]);
        assert_eq!(group2.disc_number, Some(2));

        let group3 = AudioGroup::new("Regular".to_string(), vec![]);
        assert_eq!(group3.disc_number, None);
    }

    #[test]
    fn test_discover_and_group() {
        let temp_dir = TempDir::new().unwrap();
        let book1 = temp_dir.path().join("Book1");
        let book2 = temp_dir.path().join("Book2");
        std::fs::create_dir(&book1).unwrap();
        std::fs::create_dir(&book2).unwrap();

        create_test_file(&temp_dir, "Book1/01.mp3", b"content");
        create_test_file(&temp_dir, "Book2/01.mp3", b"content");

        let groups = discover_and_group(&[book1, book2]).unwrap();
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_discover_no_audio_files() {
        let temp_dir = TempDir::new().unwrap();
        create_test_file(&temp_dir, "readme.txt", b"text content");

        let result = discover_from_path(temp_dir.path());
        assert!(matches!(result, Err(DiscoveryError::NoAudioFiles(_))));
    }

    #[test]
    fn test_file_metadata_from_audio_metadata() {
        let audio_metadata = AudioMetadata {
            duration: Some(Duration::from_secs(120)),
            bitrate: Some(128000),
            sample_rate: Some(44100),
            channels: Some(2),
            codec: Some("aac".to_string()),
            format_name: Some("m4a".to_string()),
            format_long_name: Some("MP4/M4A".to_string()),
            tags: None,
        };

        let file_metadata: FileMetadata = audio_metadata.into();
        assert_eq!(file_metadata.duration, Duration::from_secs(120));
        assert_eq!(file_metadata.bitrate, 128000);
        assert_eq!(file_metadata.sample_rate, 44100);
        assert_eq!(file_metadata.channels, 2);
        assert_eq!(file_metadata.codec, "aac");
    }
}
