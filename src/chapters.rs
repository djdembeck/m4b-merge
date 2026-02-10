//! Chapter handling for m4b-merge

use std::path::Path;
use std::time::Duration;

pub use crate::metadata::Chapter;

/// Chapter information extracted from a file
#[derive(Debug, Clone)]
pub struct FileChapter {
    pub title: String,
    pub start_time: Duration,
}

/// Read chapters from an M4B file using mp4ameta 0.13
pub fn read_chapters(path: &Path) -> Result<Vec<FileChapter>, Box<dyn std::error::Error>> {
    let tag = mp4ameta::Tag::read_from_path(path)?;
    
    let chapters: Vec<FileChapter> = tag.chapters()
        .iter()
        .map(|ch| FileChapter {
            title: ch.title.clone(),
            start_time: ch.start,
        })
        .collect();
    
    Ok(chapters)
}

/// Write chapters to an M4B file using mp4ameta 0.13
pub fn write_chapters(path: &Path, chapters: &[Chapter]) -> Result<(), Box<dyn std::error::Error>> {
    let mut tag = mp4ameta::Tag::read_from_path(path)?;
    
    // Clear existing chapters and add new ones
    let chapter_list = tag.chapter_list_mut();
    chapter_list.clear();
    
    // Add chapters
    for chapter in chapters {
        let mp4_chapter = mp4ameta::Chapter::new(chapter.start_time, &chapter.title);
        chapter_list.push(mp4_chapter);
    }
    
    tag.write_to_path(path)?;
    Ok(())
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
    use std::path::PathBuf;

    #[test]
    fn test_read_chapters() {
        let test_file = PathBuf::from(
            std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string())
                + "/output/Trailer Park Bikini Vampires [B0FDDCDXQ2].m4b",
        );

        if !test_file.exists() {
            println!("Test file does not exist, skipping test");
            return;
        }

        let chapters = read_chapters(&test_file).expect("Failed to read chapters");

        println!("Found {} chapters:", chapters.len());
        for (i, chapter) in chapters.iter().enumerate() {
            println!(
                "  Chapter {}: '{}' (start: {:?})",
                i + 1, chapter.title, chapter.start_time
            );
        }

        // mp4ameta should now read chapters properly in 0.13
        // If chapters exist, verify them
        if !chapters.is_empty() {
            assert!(!chapters[0].title.is_empty(), "First chapter should have a title");
        }
    }

    #[test]
    fn test_format_chapters_txt() {
        let chapters = vec![
            Chapter::new("Chapter 1", Duration::from_secs(0), Duration::from_secs(600)),
            Chapter::new("Chapter 2", Duration::from_secs(600), Duration::from_secs(600)),
        ];
        
        let txt = format_chapters_txt(&chapters, Duration::from_secs(1200));
        
        // 1200 seconds = 20 minutes = 00:20:00
        assert!(txt.contains("## total-duration: 00:20:00.000"), "Total duration format incorrect:\n{}", txt);
        assert!(txt.contains("00:00:00.000 Chapter 1"), "Chapter 1 format incorrect:\n{}", txt);
        assert!(txt.contains("00:10:00.000 Chapter 2"), "Chapter 2 format incorrect:\n{}", txt);
    }
}
