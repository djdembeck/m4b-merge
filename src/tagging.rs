use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use crate::metadata::BookMetadata;

/// Errors that can occur during tagging operations
#[derive(Error, Debug)]
pub enum TaggingError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("MP4 metadata error: {0}")]
    Mp4Meta(String),

    #[error("Cover download error: {0}")]
    CoverDownload(String),

    #[error("Invalid file format: {0}")]
    InvalidFormat(String),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("File move error: {0}")]
    FileMove(String),

    #[error("Duplicate chapter start times detected")]
    DuplicateChapterTimes,
}

/// Result type for tagging operations
pub type Result<T> = std::result::Result<T, TaggingError>;

/// Behavior when destination file already exists
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverwriteBehavior {
    /// Skip the file, don't overwrite
    Skip,
    /// Return an error
    #[default]
    Error,
    /// Overwrite the existing file
    Force,
}

/// Convert metadata chapters to mp4ameta chapters
///
/// Sorts chapters by start_time and converts each to mp4ameta::Chapter.
/// The mp4ameta crate handles duration calculation internally based on chapter start times.
fn convert_chapters_for_embedding(
    chapters: &[crate::metadata::Chapter],
    _total_duration: Option<Duration>,
) -> Vec<mp4ameta::Chapter> {
    let mut metadata_chapters = chapters.to_vec();

    // Sort by start_time
    metadata_chapters.sort_by_key(|c| c.start_time);

    let mut result = Vec::with_capacity(metadata_chapters.len());

    for chapter in metadata_chapters.iter() {
        // Truncate title to 255 characters (chpl atom limit)
        let title = if chapter.title.len() > 255 { &chapter.title[..255] } else { &chapter.title };

        result.push(mp4ameta::Chapter::new(chapter.start_time, title));
    }

    result
}

/// Validate chapters before embedding
///
/// Sorts chapters by start_time and checks for duplicate start times.
/// Returns an error if duplicates are found.
fn validate_and_sort_chapters(chapters: &mut [crate::metadata::Chapter]) -> Result<()> {
    if chapters.is_empty() {
        return Ok(());
    }

    // Sort by start_time
    chapters.sort_by_key(|c| c.start_time);

    // Check for duplicates
    for i in 1..chapters.len() {
        if chapters[i].start_time == chapters[i - 1].start_time {
            return Err(TaggingError::DuplicateChapterTimes);
        }
    }

    Ok(())
}

/// Handles metadata tagging and file operations for audiobooks
#[derive(Debug, Clone)]
pub struct Tagger {
    /// HTTP client for downloading cover art
    http_client: reqwest::Client,
    /// Behavior when destination file exists
    overwrite_behavior: OverwriteBehavior,
}

impl Tagger {
    /// Create a new Tagger with default settings
    pub fn new() -> Self {
        Self {
            http_client: reqwest::Client::new(),
            overwrite_behavior: OverwriteBehavior::default(),
        }
    }

    /// Create a new Tagger with custom overwrite behavior
    pub fn with_overwrite_behavior(mut self, behavior: OverwriteBehavior) -> Self {
        self.overwrite_behavior = behavior;
        self
    }

    /// Create a new Tagger with a custom HTTP client
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = client;
        self
    }

    /// Write metadata tags to an MP4 file
    ///
    /// Maps BookMetadata to MP4 atoms:
    /// - title -> ©nam
    /// - artist (authors) -> ©ART
    /// - album (title) -> ©alb
    /// - genre -> ©gen
    /// - year -> ©day
    /// - comment (description) -> ©cmt
    /// - narrator -> ©nrt (custom)
    /// - series -> ©grp (grouping)
    pub fn write_metadata<P: AsRef<Path>>(
        &self,
        file_path: P,
        metadata: &BookMetadata,
    ) -> Result<()> {
        let path = file_path.as_ref();

        if !path.exists() {
            return Err(TaggingError::FileNotFound(path.to_path_buf()));
        }

        debug!("Writing metadata to: {}", path.display());

        // Read existing tag or create new one
        let mut tag = mp4ameta::Tag::read_from_path(path).unwrap_or_default();

        // Standard tags
        tag.set_title(&metadata.title);

        // Artist (authors)
        if !metadata.authors.is_empty() {
            let authors = metadata.authors.join(", ");
            tag.set_artist(&authors);
        }

        // Album (use title for audiobooks)
        tag.set_album(&metadata.title);

        // Genre
        if let Some(first_genre) = metadata.genres.first() {
            tag.set_genre(first_genre.as_str());
        }

        // Year
        if let Some(year) = metadata.year {
            tag.set_year(year.to_string());
        }

        // Comment (description)
        if !metadata.description.is_empty() {
            tag.set_comment(&metadata.description);
        }

        // Extended tags (using custom atoms)

        // Narrator
        if !metadata.narrators.is_empty() {
            let narrators = metadata.narrators.join(", ");
            // Use grouping atom for narrator (custom convention)
            tag.set_grouping(format!("Narrator: {}", narrators));
        }

        // Series information
        if let Some(series_name) = &metadata.series_name {
            let series_info = match &metadata.series_position {
                Some(pos) => format!("{} #{}", series_name, pos),
                None => series_name.clone(),
            };
            // Add to grouping if not already set, otherwise append
            let existing_grouping = tag.grouping().map(|s| s.to_string()).unwrap_or_default();
            if existing_grouping.is_empty() {
                tag.set_grouping(format!("Series: {}", series_info));
            } else {
                tag.set_grouping(format!("{} | Series: {}", existing_grouping, series_info));
            }
        }

        // Write the tag back to file
        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write tags: {}", e)))?;

        info!("Successfully wrote metadata to: {}", path.display());
        Ok(())
    }

    /// Download cover art from URL and embed it in the MP4 file
    pub async fn embed_cover<P: AsRef<Path>>(&self, file_path: P, cover_url: &str) -> Result<()> {
        let path = file_path.as_ref();

        if !path.exists() {
            return Err(TaggingError::FileNotFound(path.to_path_buf()));
        }

        debug!("Downloading cover from: {}", cover_url);

        // Download cover image
        let image_data = self.download_image(cover_url).await?;

        debug!("Downloaded cover image ({} bytes)", image_data.len());

        // Embed in file
        self.embed_cover_data(path, &image_data)
    }

    /// Embed pre-downloaded cover image data into an MP4 file
    pub fn embed_cover_data<P: AsRef<Path>>(&self, file_path: P, image_data: &[u8]) -> Result<()> {
        let path = file_path.as_ref();

        if !path.exists() {
            return Err(TaggingError::FileNotFound(path.to_path_buf()));
        }

        debug!("Embedding cover art into: {}", path.display());

        // Detect image format
        let format = detect_image_format(image_data)?;

        // Read existing tag or create new one
        let mut tag = mp4ameta::Tag::read_from_path(path).unwrap_or_default();

        // Set artwork based on format
        let img_fmt = match format {
            ImageFormat::Jpeg => mp4ameta::ImgFmt::Jpeg,
            ImageFormat::Png => mp4ameta::ImgFmt::Png,
        };
        let artwork = mp4ameta::Img::new(img_fmt, image_data.to_vec());
        tag.set_artwork(artwork);

        // Write the tag back to file
        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to embed cover: {}", e)))?;

        info!("Successfully embedded cover art in: {}", path.display());
        Ok(())
    }

    /// Embed chapters into an M4B file
    ///
    /// This method writes chapters to the MP4 container using the chapter list (chpl atom).
    /// Any existing chapters in the file are replaced.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the M4B file
    /// * `chapters` - Slice of chapters to embed
    /// * `total_duration` - Optional total duration of the audio (for calculating last chapter duration)
    ///
    /// # Errors
    ///
    /// Returns `TaggingError` if:
    /// - File doesn't exist
    /// - Chapters are invalid (empty, duplicates)
    /// - Writing to file fails
    pub fn embed_chapters<P: AsRef<Path>>(
        &self,
        file_path: P,
        chapters: &[crate::metadata::Chapter],
        _total_duration: Option<Duration>,
    ) -> Result<()> {
        let path = file_path.as_ref();

        // Validate file exists
        if !path.exists() {
            return Err(TaggingError::FileNotFound(path.to_path_buf()));
        }

        // Skip if empty (with debug log)
        if chapters.is_empty() {
            debug!("No chapters to embed, skipping");
            return Ok(());
        }

        // Validate chapters (using helper from Task 2)
        let mut chapters_vec = chapters.to_vec();
        validate_and_sort_chapters(&mut chapters_vec)?;

        info!("Embedding {} chapters into: {}", chapters_vec.len(), path.display());

        // Read existing tag
        let mut tag = mp4ameta::Tag::read_from_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to read tag: {}", e)))?;

        // Clear existing chapters
        tag.chapter_list_mut().clear();

        // Convert and add chapters (using helper from Task 2)
        let mp4_chapters = convert_chapters_for_embedding(&chapters_vec, _total_duration);
        tag.chapter_list_mut().extend(mp4_chapters);

        // Write back to file
        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write chapters: {}", e)))?;

        info!("Successfully embedded {} chapters", chapters_vec.len());
        Ok(())
    }

    /// Download image from URL
    async fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .http_client
            .get(url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| TaggingError::CoverDownload(format!("Request failed/timeout: {}", e)))?;

        if !response.status().is_success() {
            return Err(TaggingError::CoverDownload(format!("HTTP error: {}", response.status())));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| TaggingError::CoverDownload(format!("Failed to read bytes: {}", e)))?;

        Ok(bytes.to_vec())
    }

    /// Write chapters to a chapters.txt file in mp4v2 format
    ///
    /// Format:
    /// ```text
    /// ## artist: Author Name
    /// ## album: Book Title
    /// ## total-duration: 10:23:45.123
    /// 00:00:00.000 Chapter 1 Title
    /// 00:15:32.500 Chapter 2 Title
    /// 01:23:45.000 Chapter 3 Title
    /// ```
    pub fn write_chapters_txt<P: AsRef<Path>>(
        &self,
        output_path: P,
        metadata: &BookMetadata,
    ) -> Result<()> {
        let path = output_path.as_ref();

        debug!("Writing chapters.txt to: {}", path.display());

        // Build chapters content
        let mut content = String::new();

        // Add metadata headers
        if !metadata.authors.is_empty() {
            content.push_str(&format!("## artist: {}\n", metadata.authors.join(", ")));
        }
        content.push_str(&format!("## album: {}\n", metadata.title));

        // Add total duration if available
        if let Some(duration) = metadata.total_duration() {
            content.push_str(&format!("## total-duration: {}\n", format_duration(duration)));
        }

        // Add empty line before chapters
        content.push('\n');

        // Add chapters
        for chapter in &metadata.chapters {
            let timestamp = format_duration(chapter.start_time);
            content.push_str(&format!("{} {}\n", timestamp, chapter.title));
        }

        // Write to file
        std::fs::write(path, content)?;

        info!("Successfully wrote chapters.txt to: {}", path.display());
        Ok(())
    }

    /// Move source files to completed directory
    ///
    /// Returns a list of successfully moved file paths
    pub fn move_completed_files<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        files: &[P],
        dest_dir: Q,
    ) -> Result<Vec<PathBuf>> {
        let dest = dest_dir.as_ref();

        // Ensure destination directory exists
        std::fs::create_dir_all(dest)?;

        let mut moved_files = Vec::new();

        for file in files {
            let source = file.as_ref();

            if !source.exists() {
                warn!("Source file does not exist, skipping: {}", source.display());
                continue;
            }

            let file_name = source
                .file_name()
                .ok_or_else(|| TaggingError::FileMove("Invalid source path".to_string()))?;

            let dest_path = dest.join(file_name);

            // Check if destination exists
            if dest_path.exists() {
                match self.overwrite_behavior {
                    OverwriteBehavior::Skip => {
                        warn!("Destination exists, skipping: {}", dest_path.display());
                        continue;
                    }
                    OverwriteBehavior::Error => {
                        return Err(TaggingError::FileMove(format!(
                            "Destination already exists: {}",
                            dest_path.display()
                        )));
                    }
                    OverwriteBehavior::Force => {
                        warn!("Overwriting existing file: {}", dest_path.display());
                        std::fs::remove_file(&dest_path).map_err(|e| {
                            TaggingError::FileMove(format!(
                                "Failed to remove existing destination {}: {}",
                                dest_path.display(),
                                e
                            ))
                        })?;
                    }
                }
            }

            // Move the file
            if let Err(e) = std::fs::rename(source, &dest_path) {
                if is_cross_device_error(&e) {
                    std::fs::copy(source, &dest_path).map_err(|copy_err| {
                        TaggingError::FileMove(format!(
                            "Failed to copy {} to {} (cross-device fallback): {}",
                            source.display(),
                            dest_path.display(),
                            copy_err
                        ))
                    })?;

                    std::fs::remove_file(source).map_err(|remove_err| {
                        TaggingError::FileMove(format!(
                            "Failed to remove source {} after copy: {}",
                            source.display(),
                            remove_err
                        ))
                    })?;
                } else {
                    return Err(TaggingError::FileMove(format!(
                        "Failed to move {} to {}: {}",
                        source.display(),
                        dest_path.display(),
                        e
                    )));
                }
            }

            info!("Moved file: {} -> {}", source.display(), dest_path.display());
            moved_files.push(dest_path);
        }

        info!("Successfully moved {} files to {}", moved_files.len(), dest.display());
        Ok(moved_files)
    }

    /// Process a completed audiobook: tag, add cover, write chapters, and move source files
    ///
    /// This is a convenience method that performs all post-merge operations
    pub async fn process_completed_book<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        output_file: P,
        metadata: &BookMetadata,
        source_files: &[Q],
        completed_dir: Option<&Path>,
    ) -> Result<()> {
        let output = output_file.as_ref();

        // 1. Write metadata tags
        self.write_metadata(output, metadata)?;

        // 2. Download and embed cover art if URL is available
        if let Some(cover_url) = &metadata.cover_url {
            if let Err(e) = self.embed_cover(output, cover_url).await {
                warn!("Failed to embed cover art: {}", e);
                // Don't fail the whole operation if cover embedding fails
            }
        }

        // 3. Write chapters.txt alongside output file
        let chapters_txt_path = output.with_extension("").with_extension("chapters.txt");
        if let Err(e) = self.write_chapters_txt(&chapters_txt_path, metadata) {
            warn!("Failed to write chapters.txt: {}", e);
            // Don't fail the whole operation if chapters writing fails
        }

        // 4. Move source files to completed directory if specified
        if let Some(dest_dir) = completed_dir {
            let source_paths: Vec<_> = source_files.iter().map(|p| p.as_ref()).collect();
            if let Err(e) = self.move_completed_files(&source_paths, dest_dir) {
                error!("Failed to move source files: {}", e);
                // Don't fail the whole operation if file moving fails
            }
        }

        info!("Completed post-processing for: {}", output.display());
        Ok(())
    }
}

impl Default for Tagger {
    fn default() -> Self {
        Self::new()
    }
}

/// Image format enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageFormat {
    Jpeg,
    Png,
}

/// Detect image format from magic bytes
fn detect_image_format(data: &[u8]) -> Result<ImageFormat> {
    if data.len() < 8 {
        return Err(TaggingError::InvalidFormat("Image data too short".to_string()));
    }

    // JPEG magic bytes: FF D8 FF
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Ok(ImageFormat::Jpeg);
    }

    // PNG magic bytes: 89 50 4E 47 0D 0A 1A 0A
    if data[0] == 0x89
        && data[1] == 0x50
        && data[2] == 0x4E
        && data[3] == 0x47
        && data[4] == 0x0D
        && data[5] == 0x0A
        && data[6] == 0x1A
        && data[7] == 0x0A
    {
        return Ok(ImageFormat::Png);
    }

    Err(TaggingError::InvalidFormat("Unknown image format".to_string()))
}

/// Format a Duration as HH:MM:SS.mmm
fn format_duration(duration: Duration) -> String {
    let total_millis = duration.as_millis();
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let seconds = (total_millis % 60_000) / 1_000;
    let millis = total_millis % 1_000;

    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

/// Check if an IO error indicates a cross-device link (EXDEV on Unix)
/// Note: On Windows, cross-device moves fail with ERROR_NOT_SAME_DEVICE (17).
/// This function only checks for Unix errno 18 (EXDEV).
fn is_cross_device_error(err: &std::io::Error) -> bool {
    // Unix: errno 18 is EXDEV (Invalid cross-device link)
    err.raw_os_error() == Some(18)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Chapter;
    use tempfile::TempDir;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00:00.000");
        assert_eq!(format_duration(Duration::from_secs(90)), "00:01:30.000");
        assert_eq!(format_duration(Duration::from_millis(5432100)), "01:30:32.100");
        assert_eq!(format_duration(Duration::from_millis(3661001)), "01:01:01.001");
    }

    #[test]
    fn test_detect_image_format_jpeg() {
        // JPEG magic bytes
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        assert_eq!(detect_image_format(&jpeg_data).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn test_detect_image_format_png() {
        // PNG magic bytes
        let png_data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_image_format(&png_data).unwrap(), ImageFormat::Png);
    }

    #[test]
    fn test_detect_image_format_unknown() {
        let unknown_data = vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        assert!(detect_image_format(&unknown_data).is_err());
    }

    #[test]
    fn test_detect_image_format_too_short() {
        let short_data = vec![0xFF, 0xD8];
        assert!(detect_image_format(&short_data).is_err());
    }

    #[test]
    fn test_overwrite_behavior_default() {
        assert_eq!(OverwriteBehavior::default(), OverwriteBehavior::Error);
    }

    #[test]
    fn test_tagger_creation() {
        let tagger = Tagger::new();
        assert!(matches!(tagger.overwrite_behavior, OverwriteBehavior::Error));
    }

    #[test]
    fn test_tagger_with_overwrite_behavior() {
        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Force);
        assert!(matches!(tagger.overwrite_behavior, OverwriteBehavior::Force));
    }

    #[test]
    fn test_write_chapters_txt() {
        let temp_dir = TempDir::new().unwrap();
        let chapters_path = temp_dir.path().join("chapters.txt");

        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Description")
            .add_author("Test Author")
            .add_chapter(Chapter::new(
                "Chapter 1",
                Duration::from_secs(0),
                Duration::from_secs(600),
            ))
            .add_chapter(Chapter::new(
                "Chapter 2",
                Duration::from_secs(600),
                Duration::from_secs(600),
            ));

        let tagger = Tagger::new();
        tagger.write_chapters_txt(&chapters_path, &metadata).unwrap();

        let content = std::fs::read_to_string(&chapters_path).unwrap();
        assert!(content.contains("## artist: Test Author"));
        assert!(content.contains("## album: Test Book"));
        assert!(content.contains("00:00:00.000 Chapter 1"));
        assert!(content.contains("00:10:00.000 Chapter 2"));
    }

    #[test]
    fn test_move_completed_files() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let file2 = source_dir.join("file2.txt");
        std::fs::write(&file1, "content1").unwrap();
        std::fs::write(&file2, "content2").unwrap();

        let tagger = Tagger::new();
        let moved =
            tagger.move_completed_files(&[file1.clone(), file2.clone()], &dest_dir).unwrap();

        assert_eq!(moved.len(), 2);
        assert!(dest_dir.join("file1.txt").exists());
        assert!(dest_dir.join("file2.txt").exists());
        assert!(!file1.exists());
        assert!(!file2.exists());
    }

    #[test]
    fn test_move_completed_files_skip_existing() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let existing_file = dest_dir.join("file1.txt");
        std::fs::write(&file1, "new content").unwrap();
        std::fs::write(&existing_file, "existing content").unwrap();

        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Skip);
        let moved = tagger.move_completed_files(&[file1.clone()], &dest_dir).unwrap();

        assert_eq!(moved.len(), 0);
        assert!(file1.exists()); // Should not be moved
        assert_eq!(std::fs::read_to_string(&existing_file).unwrap(), "existing content");
    }

    #[test]
    fn test_move_completed_files_error_on_existing() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let existing_file = dest_dir.join("file1.txt");
        std::fs::write(&file1, "new content").unwrap();
        std::fs::write(&existing_file, "existing content").unwrap();

        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Error);
        let result = tagger.move_completed_files(&[file1.clone()], &dest_dir);

        assert!(result.is_err());
    }

    #[test]
    fn test_move_completed_files_force_overwrite() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let existing_file = dest_dir.join("file1.txt");
        std::fs::write(&file1, "new content").unwrap();
        std::fs::write(&existing_file, "existing content").unwrap();

        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Force);
        let moved = tagger.move_completed_files(&[file1.clone()], &dest_dir).unwrap();

        assert_eq!(moved.len(), 1);
        assert!(!file1.exists());
        assert_eq!(std::fs::read_to_string(&existing_file).unwrap(), "new content");
    }

    #[test]
    fn test_tagger_file_not_found() {
        let tagger = Tagger::new();
        let metadata = BookMetadata::new("B08XYZ1234", "Test", "Desc");

        let result = tagger.write_metadata("/nonexistent/path/file.m4b", &metadata);
        assert!(matches!(result, Err(TaggingError::FileNotFound(_))));
    }

    #[test]
    fn test_chapters_txt_format() {
        let temp_dir = TempDir::new().unwrap();
        let chapters_path = temp_dir.path().join("chapters.txt");

        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Description")
            .add_author("Author One")
            .add_author("Author Two")
            .with_year(2023)
            .add_chapter(Chapter::new(
                "Intro",
                Duration::from_millis(0),
                Duration::from_millis(5000),
            ))
            .add_chapter(Chapter::new(
                "Chapter 1",
                Duration::from_millis(5000),
                Duration::from_millis(10000),
            ))
            .add_chapter(Chapter::new(
                "Chapter 2",
                Duration::from_millis(15000),
                Duration::from_millis(10000),
            ));

        let tagger = Tagger::new();
        tagger.write_chapters_txt(&chapters_path, &metadata).unwrap();

        let content = std::fs::read_to_string(&chapters_path).unwrap();

        // Check header format
        assert!(content.contains("## artist: Author One, Author Two"));
        assert!(content.contains("## album: Test Book"));
        assert!(content.contains("## total-duration: 00:00:25.000"));

        // Check chapter format
        assert!(content.contains("00:00:00.000 Intro"));
        assert!(content.contains("00:00:05.000 Chapter 1"));
        assert!(content.contains("00:00:15.000 Chapter 2"));
    }

    // Chapter embedding helper function tests

    #[test]
    fn test_convert_chapters_for_embedding() {
        let chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
        ];

        let mp4_chapters = convert_chapters_for_embedding(&chapters, None);
        assert_eq!(mp4_chapters.len(), 2);
        assert_eq!(mp4_chapters[0].title, "Ch1");
        assert_eq!(mp4_chapters[0].start, Duration::ZERO);
        assert_eq!(mp4_chapters[1].title, "Ch2");
        assert_eq!(mp4_chapters[1].start, Duration::from_secs(60));
    }

    #[test]
    fn test_convert_chapters_for_embedding_sorts_chapters() {
        // Test that chapters are sorted by start_time
        let chapters = vec![
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch3", Duration::from_secs(120), Duration::from_secs(60)),
        ];

        let mp4_chapters = convert_chapters_for_embedding(&chapters, None);
        assert_eq!(mp4_chapters.len(), 3);
        assert_eq!(mp4_chapters[0].title, "Ch1");
        assert_eq!(mp4_chapters[1].title, "Ch2");
        assert_eq!(mp4_chapters[2].title, "Ch3");
    }

    #[test]
    fn test_convert_chapters_for_embedding_title_truncation() {
        // Test that titles longer than 255 characters are truncated
        let long_title = "A".repeat(300);
        let chapters =
            vec![Chapter::new(long_title.clone(), Duration::ZERO, Duration::from_secs(60))];

        let mp4_chapters = convert_chapters_for_embedding(&chapters, None);
        assert_eq!(mp4_chapters.len(), 1);
        assert_eq!(mp4_chapters[0].title.len(), 255);
    }

    #[test]
    fn test_convert_chapters_for_embedding_empty() {
        let chapters: Vec<Chapter> = vec![];
        let mp4_chapters = convert_chapters_for_embedding(&chapters, None);
        assert!(mp4_chapters.is_empty());
    }

    #[test]
    fn test_validate_and_sort_chapters_empty() {
        let mut chapters: Vec<Chapter> = vec![];
        let result = validate_and_sort_chapters(&mut chapters);
        assert!(result.is_ok());
        assert!(chapters.is_empty());
    }

    #[test]
    fn test_validate_and_sort_chapters_single() {
        let mut chapters = vec![Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60))];
        let result = validate_and_sort_chapters(&mut chapters);
        assert!(result.is_ok());
        assert_eq!(chapters.len(), 1);
    }

    #[test]
    fn test_validate_and_sort_chapters_unsorted_gets_sorted() {
        let mut chapters = vec![
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch3", Duration::from_secs(120), Duration::from_secs(60)),
        ];

        validate_and_sort_chapters(&mut chapters).unwrap();

        assert_eq!(chapters[0].title, "Ch1");
        assert_eq!(chapters[0].start_time, Duration::ZERO);
        assert_eq!(chapters[1].title, "Ch2");
        assert_eq!(chapters[1].start_time, Duration::from_secs(60));
        assert_eq!(chapters[2].title, "Ch3");
        assert_eq!(chapters[2].start_time, Duration::from_secs(120));
    }

    #[test]
    fn test_validate_and_sort_chapters_duplicates() {
        let chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::ZERO, Duration::from_secs(60)),
        ];
        let mut chapters_copy = chapters;
        let result = validate_and_sort_chapters(&mut chapters_copy);
        assert!(matches!(result, Err(TaggingError::DuplicateChapterTimes)));
    }

    #[test]
    fn test_validate_and_sort_chapters_already_sorted() {
        let mut chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
        ];

        let result = validate_and_sort_chapters(&mut chapters);
        assert!(result.is_ok());
        assert_eq!(chapters[0].title, "Ch1");
        assert_eq!(chapters[1].title, "Ch2");
    }

    // Helper function to create a minimal M4B file for testing
    fn create_minimal_m4b(dir: &TempDir) -> PathBuf {
        use std::process::Command;

        let path = dir.path().join("test.m4b");
        let status = Command::new("ffmpeg")
            .args(&[
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=mono",
                "-t",
                "10",
                "-c:a",
                "aac",
                "-b:a",
                "64k",
                "-y",
                path.to_str().unwrap(),
            ])
            .status();

        match status {
            Ok(s) if s.success() => path,
            _ => panic!("FFmpeg required for tests"),
        }
    }

    #[test]
    fn test_embed_chapters_empty() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        let tagger = Tagger::new();
        let chapters: Vec<Chapter> = vec![];

        let result = tagger.embed_chapters(&m4b_path, &chapters, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_embed_chapters_success() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        let chapters = vec![
            Chapter::new("Chapter 1", Duration::ZERO, Duration::from_secs(5)),
            Chapter::new("Chapter 2", Duration::from_secs(5), Duration::from_secs(5)),
        ];

        let tagger = Tagger::new();
        let result = tagger.embed_chapters(&m4b_path, &chapters, None);
        assert!(result.is_ok());

        // Verify chapters were embedded by reading the file
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let chapter_list = tag.chapter_list();
        assert_eq!(chapter_list.len(), 2);
        assert_eq!(chapter_list[0].title, "Chapter 1");
        assert_eq!(chapter_list[1].title, "Chapter 2");
    }

    #[test]
    fn test_embed_chapters_replaces_existing() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        // First embed some chapters
        let initial_chapters =
            vec![Chapter::new("Initial Chapter", Duration::ZERO, Duration::from_secs(10))];
        let tagger = Tagger::new();
        tagger.embed_chapters(&m4b_path, &initial_chapters, None).unwrap();

        // Verify initial chapters
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        assert_eq!(tag.chapter_list().len(), 1);

        // Now embed new chapters (should replace)
        let new_chapters = vec![
            Chapter::new("New Chapter 1", Duration::ZERO, Duration::from_secs(5)),
            Chapter::new("New Chapter 2", Duration::from_secs(5), Duration::from_secs(5)),
        ];
        tagger.embed_chapters(&m4b_path, &new_chapters, None).unwrap();

        // Verify chapters were replaced
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let chapter_list = tag.chapter_list();
        assert_eq!(chapter_list.len(), 2);
        assert_eq!(chapter_list[0].title, "New Chapter 1");
        assert_eq!(chapter_list[1].title, "New Chapter 2");
    }

    #[test]
    fn test_embed_chapters_sorts_unsorted_input() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        // Provide chapters out of order
        let chapters = vec![
            Chapter::new("Chapter 2", Duration::from_secs(5), Duration::from_secs(5)),
            Chapter::new("Chapter 1", Duration::ZERO, Duration::from_secs(5)),
        ];

        let tagger = Tagger::new();
        let result = tagger.embed_chapters(&m4b_path, &chapters, None);
        assert!(result.is_ok());

        // Verify chapters were sorted
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let chapter_list = tag.chapter_list();
        assert_eq!(chapter_list.len(), 2);
        assert_eq!(chapter_list[0].title, "Chapter 1");
        assert_eq!(chapter_list[1].title, "Chapter 2");
    }

    #[test]
    fn test_embed_chapters_file_not_found() {
        let tagger = Tagger::new();
        let chapters = vec![Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60))];

        let result = tagger.embed_chapters("/nonexistent/path/file.m4b", &chapters, None);
        assert!(matches!(result, Err(TaggingError::FileNotFound(_))));
    }

    #[test]
    fn test_embed_chapters_duplicate_times_error() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        let chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::ZERO, Duration::from_secs(60)),
        ];

        let tagger = Tagger::new();
        let result = tagger.embed_chapters(&m4b_path, &chapters, None);
        assert!(matches!(result, Err(TaggingError::DuplicateChapterTimes)));
    }
}
