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
}

/// Result type for tagging operations
pub type Result<T> = std::result::Result<T, TaggingError>;

/// Behavior when destination file already exists
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteBehavior {
    /// Skip the file, don't overwrite
    Skip,
    /// Return an error
    Error,
    /// Overwrite the existing file
    Force,
}

impl Default for OverwriteBehavior {
    fn default() -> Self {
        Self::Error
    }
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
            tag.set_grouping(&format!("Narrator: {}", narrators));
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
                tag.set_grouping(&format!("Series: {}", series_info));
            } else {
                tag.set_grouping(&format!("{} | Series: {}", existing_grouping, series_info));
            }
        }

        // Write the tag back to file
        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write tags: {}", e)))?;

        info!("Successfully wrote metadata to: {}", path.display());
        Ok(())
    }

    /// Download cover art from URL and embed it in the MP4 file
    pub async fn embed_cover<P: AsRef<Path>>(
        &self,
        file_path: P,
        cover_url: &str,
    ) -> Result<()> {
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
    pub fn embed_cover_data<P: AsRef<Path>>(
        &self,
        file_path: P,
        image_data: &[u8],
    ) -> Result<()> {
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

    /// Download image from URL
    async fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| TaggingError::CoverDownload(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(TaggingError::CoverDownload(format!(
                "HTTP error: {}",
                response.status()
            )));
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
                let is_exdev = e.raw_os_error() == Some(18);

                if is_exdev {
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

            info!(
                "Moved file: {} -> {}",
                source.display(),
                dest_path.display()
            );
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
        return Err(TaggingError::InvalidFormat(
            "Image data too short".to_string(),
        ));
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

    Err(TaggingError::InvalidFormat(
        "Unknown image format".to_string(),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Chapter;
    use tempfile::TempDir;

    #[test]
    fn test_format_duration() {
        assert_eq!(
            format_duration(Duration::from_secs(0)),
            "00:00:00.000"
        );
        assert_eq!(
            format_duration(Duration::from_secs(90)),
            "00:01:30.000"
        );
        assert_eq!(
            format_duration(Duration::from_millis(5432100)),
            "01:30:32.100"
        );
        assert_eq!(
            format_duration(Duration::from_millis(3661001)),
            "01:01:01.001"
        );
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
            .add_chapter(Chapter::new("Chapter 1", Duration::from_secs(0), Duration::from_secs(600)))
            .add_chapter(Chapter::new("Chapter 2", Duration::from_secs(600), Duration::from_secs(600)));

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
        let moved = tagger
            .move_completed_files(&[file1.clone(), file2.clone()], &dest_dir)
            .unwrap();

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
        assert_eq!(
            std::fs::read_to_string(&existing_file).unwrap(),
            "existing content"
        );
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
        assert_eq!(
            std::fs::read_to_string(&existing_file).unwrap(),
            "new content"
        );
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
            .add_chapter(Chapter::new("Intro", Duration::from_millis(0), Duration::from_millis(5000)))
            .add_chapter(Chapter::new("Chapter 1", Duration::from_millis(5000), Duration::from_millis(10000)))
            .add_chapter(Chapter::new("Chapter 2", Duration::from_millis(15000), Duration::from_millis(10000)));

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
}
