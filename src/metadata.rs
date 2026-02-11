use std::time::Duration;

/// Chapter information for audiobooks
#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start_time: Duration,
    pub duration: Duration,
}

impl Chapter {
    /// Create a new chapter with the given title, start time, and duration
    pub fn new(title: impl Into<String>, start_time: Duration, duration: Duration) -> Self {
        Self { title: title.into(), start_time, duration }
    }

    /// Get the end time of this chapter
    pub fn end_time(&self) -> Duration {
        self.start_time + self.duration
    }
}

/// Complete metadata for an audiobook
#[derive(Debug, Clone, PartialEq)]
pub struct BookMetadata {
    pub asin: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub narrators: Vec<String>,
    pub series_name: Option<String>,
    pub series_position: Option<String>,
    pub description: String,
    pub genres: Vec<String>,
    pub year: Option<u32>,
    pub cover_url: Option<String>,
    pub chapters: Vec<Chapter>,
}

impl BookMetadata {
    /// Create a new BookMetadata with just the required fields
    pub fn new(
        asin: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            asin: asin.into(),
            title: title.into(),
            subtitle: None,
            authors: Vec::new(),
            narrators: Vec::new(),
            series_name: None,
            series_position: None,
            description: description.into(),
            genres: Vec::new(),
            year: None,
            cover_url: None,
            chapters: Vec::new(),
        }
    }

    /// Add an author to the metadata
    pub fn add_author(mut self, author: impl Into<String>) -> Self {
        self.authors.push(author.into());
        self
    }

    /// Add a narrator to the metadata
    pub fn add_narrator(mut self, narrator: impl Into<String>) -> Self {
        self.narrators.push(narrator.into());
        self
    }

    /// Add a genre to the metadata
    pub fn add_genre(mut self, genre: impl Into<String>) -> Self {
        self.genres.push(genre.into());
        self
    }

    /// Add a chapter to the metadata
    pub fn add_chapter(mut self, chapter: Chapter) -> Self {
        self.chapters.push(chapter);
        self
    }

    /// Set the subtitle
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Set the series information
    pub fn with_series(mut self, name: impl Into<String>, position: impl Into<String>) -> Self {
        self.series_name = Some(name.into());
        self.series_position = Some(position.into());
        self
    }

    /// Set the release year
    pub fn with_year(mut self, year: u32) -> Self {
        self.year = Some(year);
        self
    }

    /// Set the cover URL
    pub fn with_cover_url(mut self, url: impl Into<String>) -> Self {
        self.cover_url = Some(url.into());
        self
    }

    /// Validate the ASIN format (10 alphanumeric characters)
    pub fn is_valid_asin(asin: &str) -> bool {
        asin.len() == 10 && asin.chars().all(|c| c.is_alphanumeric())
    }

    /// Get the full display title (title + subtitle if available)
    pub fn full_title(&self) -> String {
        match &self.subtitle {
            Some(subtitle) => format!("{}: {}", self.title, subtitle),
            None => self.title.clone(),
        }
    }

    /// Get a comma-separated string of authors
    pub fn authors_str(&self) -> String {
        self.authors.join(", ")
    }

    /// Get a comma-separated string of narrators
    pub fn narrators_str(&self) -> String {
        self.narrators.join(", ")
    }

    /// Get total duration of the book based on chapters
    pub fn total_duration(&self) -> Option<Duration> {
        if self.chapters.is_empty() {
            return None;
        }

        let last_chapter = self.chapters.last()?;
        Some(last_chapter.end_time())
    }
}

/// Trait for types that can be converted to metadata
pub trait IntoBookMetadata {
    fn into_book_metadata(self) -> BookMetadata;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chapter_creation() {
        let chapter = Chapter::new("Chapter 1", Duration::from_secs(0), Duration::from_secs(600));
        assert_eq!(chapter.title, "Chapter 1");
        assert_eq!(chapter.start_time, Duration::from_secs(0));
        assert_eq!(chapter.duration, Duration::from_secs(600));
        assert_eq!(chapter.end_time(), Duration::from_secs(600));
    }

    #[test]
    fn test_book_metadata_creation() {
        let metadata = BookMetadata::new("B08XYZ123", "Book Title", "Description");
        assert_eq!(metadata.asin, "B08XYZ123");
        assert_eq!(metadata.title, "Book Title");
        assert_eq!(metadata.description, "Description");
        assert!(metadata.authors.is_empty());
        assert!(metadata.chapters.is_empty());
    }

    #[test]
    fn test_book_metadata_builder_methods() {
        let metadata = BookMetadata::new("B08XYZ123", "Title", "Desc")
            .add_author("John Doe")
            .add_narrator("Jane Smith")
            .add_genre("Fiction")
            .with_subtitle("The Sequel")
            .with_series("My Series", "1")
            .with_year(2023)
            .with_cover_url("https://example.com/cover.jpg")
            .add_chapter(Chapter::new("Intro", Duration::from_secs(0), Duration::from_secs(300)));

        assert_eq!(metadata.authors, vec!["John Doe"]);
        assert_eq!(metadata.narrators, vec!["Jane Smith"]);
        assert_eq!(metadata.genres, vec!["Fiction"]);
        assert_eq!(metadata.subtitle, Some("The Sequel".to_string()));
        assert_eq!(metadata.series_name, Some("My Series".to_string()));
        assert_eq!(metadata.series_position, Some("1".to_string()));
        assert_eq!(metadata.year, Some(2023));
        assert_eq!(metadata.cover_url, Some("https://example.com/cover.jpg".to_string()));
        assert_eq!(metadata.chapters.len(), 1);
    }

    #[test]
    fn test_valid_asin() {
        assert!(BookMetadata::is_valid_asin("B08XYZ1234"));
        assert!(BookMetadata::is_valid_asin("B08XYZ123A"));
        assert!(BookMetadata::is_valid_asin("1234567890"));
        assert!(!BookMetadata::is_valid_asin("B08XYZ123")); // Too short
        assert!(!BookMetadata::is_valid_asin("B08XYZ12345")); // Too long
        assert!(!BookMetadata::is_valid_asin("B08-XYZ123")); // Special char
        assert!(!BookMetadata::is_valid_asin(""));
    }

    #[test]
    fn test_full_title() {
        let metadata_with_subtitle =
            BookMetadata::new("A1", "Title", "Desc").with_subtitle("The Sequel");
        assert_eq!(metadata_with_subtitle.full_title(), "Title: The Sequel");

        let metadata_no_subtitle = BookMetadata::new("A1", "Title", "Desc");
        assert_eq!(metadata_no_subtitle.full_title(), "Title");
    }

    #[test]
    fn test_total_duration() {
        let metadata = BookMetadata::new("A1", "Title", "Desc")
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

        assert_eq!(metadata.total_duration(), Some(Duration::from_secs(1200)));

        let empty_metadata = BookMetadata::new("A1", "Title", "Desc");
        assert_eq!(empty_metadata.total_duration(), None);
    }
}
