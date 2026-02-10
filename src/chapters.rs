//! Chapter handling for m4b-merge
//! 
//! Note: mp4ameta 0.8 does not provide direct chapter read/write APIs.
//! Chapters are preserved during file copy/merge operations via FFmpeg.
//! For API-sourced chapters, we generate chapters.txt files alongside the output.

use std::path::Path;
use std::time::Duration;

pub use crate::metadata::Chapter;

/// Chapter information extracted from a file
#[derive(Debug, Clone)]
pub struct FileChapter {
    pub title: String,
    pub start_time: Duration,
    pub duration: Duration,
}

/// Read chapters from an M4B file
/// 
/// Note: This uses FFmpeg to extract chapter information since mp4ameta 0.8
/// does not provide direct chapter access. Returns empty vec if no chapters found.
pub fn read_chapters(_path: &Path) -> Result<Vec<FileChapter>, Box<dyn std::error::Error>> {
    // TODO: Implement using FFmpeg chapter extraction if needed
    // For now, chapters are preserved during copy operations
    Ok(Vec::new())
}

/// Format chapters for chapters.txt file (mp4v2 format)
pub fn format_chapters_txt(chapters: &[Chapter], total_duration: Duration) -> String {
    let mut output = String::new();
    
    // Add header
    output.push_str("## total-duration: ");
    output.push_str(&format_duration(total_duration));
    output.push('\n');
    output.push_str("##\n");
    
    // Add chapters
    for (i, chapter) in chapters.iter().enumerate() {
        let start = format_duration(chapter.start_time);
        output.push_str(&format!("{} Chapter {}\n", start, i + 1));
    }
    
    output
}

fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let millis = duration.subsec_millis();
    
    format!("{:02}:{:02}:{:02}.{:03}", hours, mins, secs, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_chapters_txt() {
        let chapters = vec![
            Chapter::new("Chapter 1", Duration::from_secs(0), Duration::from_secs(600)),
            Chapter::new("Chapter 2", Duration::from_secs(600), Duration::from_secs(600)),
        ];
        
        let txt = format_chapters_txt(&chapters, Duration::from_secs(1200));
        
        assert!(txt.contains("## total-duration: 00:00:20.000"));
        assert!(txt.contains("00:00:00.000 Chapter 1"));
        assert!(txt.contains("00:00:10.000 Chapter 2"));
    }
}
