use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

/// Check if FFmpeg is available
fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Create a minimal valid M4B file for testing using FFmpeg
fn create_test_m4b(path: &Path, duration_secs: u32) {
    let status = Command::new("ffmpeg")
        .args(&[
            "-f",
            "lavfi",
            "-i",
            &format!("anullsrc=r=44100:cl=mono"),
            "-t",
            &duration_secs.to_string(),
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-y",
        ])
        .arg(path)
        .status()
        .expect("Failed to run FFmpeg");

    assert!(status.success(), "FFmpeg failed to create test M4B file");
}

#[test]
fn test_chapter_embed_and_read_roundtrip() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 10-second test M4B file
    create_test_m4b(&m4b_path, 10);

    // Define chapters to embed
    let chapters = vec![
        m4b_merge::metadata::Chapter::new("Intro", Duration::ZERO, Duration::from_secs(3)),
        m4b_merge::metadata::Chapter::new(
            "Chapter 1",
            Duration::from_secs(3),
            Duration::from_secs(4),
        ),
        m4b_merge::metadata::Chapter::new("Outro", Duration::from_secs(7), Duration::from_secs(3)),
    ];

    // Embed chapters using Tagger
    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &chapters).expect("Failed to embed chapters");

    // Read back chapters using the chapters module
    let read_chapters =
        m4b_merge::chapters::read_chapters(&m4b_path).expect("Failed to read chapters");

    // Verify chapter count
    assert_eq!(read_chapters.len(), 3, "Should have 3 chapters embedded and read back");

    // Verify first chapter (Intro)
    assert_eq!(read_chapters[0].title, "Intro", "First chapter should be 'Intro'");
    assert_eq!(read_chapters[0].start_time, 0, "First chapter should start at 0ms");

    // Verify second chapter (Chapter 1)
    assert_eq!(read_chapters[1].title, "Chapter 1", "Second chapter should be 'Chapter 1'");
    assert_eq!(
        read_chapters[1].start_time, 3000,
        "Second chapter should start at 3000ms (3 seconds)"
    );

    // Verify third chapter (Outro)
    assert_eq!(read_chapters[2].title, "Outro", "Third chapter should be 'Outro'");
    assert_eq!(
        read_chapters[2].start_time, 7000,
        "Third chapter should start at 7000ms (7 seconds)"
    );
}

#[test]
fn test_chapter_embed_empty_chapters() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 5-second test M4B file
    create_test_m4b(&m4b_path, 5);

    // Embed empty chapters (should succeed without error)
    let tagger = m4b_merge::tagging::Tagger::new();
    let empty_chapters: Vec<m4b_merge::metadata::Chapter> = vec![];
    let result = tagger.embed_chapters(&m4b_path, &empty_chapters);

    assert!(result.is_ok(), "Embedding empty chapters should succeed");
}

#[test]
fn test_chapter_embed_single_chapter() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 10-second test M4B file
    create_test_m4b(&m4b_path, 10);

    // Embed a single chapter
    let chapters = vec![m4b_merge::metadata::Chapter::new(
        "Single Chapter",
        Duration::from_secs(2),
        Duration::from_secs(8),
    )];

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &chapters).expect("Failed to embed single chapter");

    // Read back and verify
    let read_chapters =
        m4b_merge::chapters::read_chapters(&m4b_path).expect("Failed to read chapters");

    assert_eq!(read_chapters.len(), 1, "Should have exactly 1 chapter");
    assert_eq!(read_chapters[0].title, "Single Chapter", "Chapter title should match");
    assert_eq!(read_chapters[0].start_time, 2000, "Chapter should start at 2000ms (2 seconds)");
}

#[test]
fn test_chapter_embed_multiple_chapters() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 60-second test M4B file
    create_test_m4b(&m4b_path, 60);

    // Embed multiple chapters
    let chapters = vec![
        m4b_merge::metadata::Chapter::new("Prologue", Duration::ZERO, Duration::from_secs(10)),
        m4b_merge::metadata::Chapter::new(
            "Chapter 1: The Beginning",
            Duration::from_secs(10),
            Duration::from_secs(20),
        ),
        m4b_merge::metadata::Chapter::new(
            "Chapter 2: The Middle",
            Duration::from_secs(30),
            Duration::from_secs(15),
        ),
        m4b_merge::metadata::Chapter::new(
            "Epilogue",
            Duration::from_secs(45),
            Duration::from_secs(15),
        ),
    ];

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &chapters).expect("Failed to embed chapters");

    // Read back and verify
    let read_chapters =
        m4b_merge::chapters::read_chapters(&m4b_path).expect("Failed to read chapters");

    assert_eq!(read_chapters.len(), 4, "Should have 4 chapters");

    // Verify all chapters in order
    let expected_titles =
        ["Prologue", "Chapter 1: The Beginning", "Chapter 2: The Middle", "Epilogue"];
    let expected_start_times = [0, 10000, 30000, 45000];

    for (i, (chapter, (&expected_title, &expected_start))) in read_chapters
        .iter()
        .zip(expected_titles.iter().zip(expected_start_times.iter()))
        .enumerate()
    {
        assert_eq!(chapter.title, expected_title, "Chapter {} title should match", i + 1);
        assert_eq!(chapter.start_time, expected_start, "Chapter {} start time should match", i + 1);
    }
}

#[test]
fn test_chapter_embed_replaces_existing() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 10-second test M4B file
    create_test_m4b(&m4b_path, 10);

    let tagger = m4b_merge::tagging::Tagger::new();

    // First embed: 2 chapters
    let initial_chapters = vec![
        m4b_merge::metadata::Chapter::new(
            "Initial Chapter 1",
            Duration::ZERO,
            Duration::from_secs(5),
        ),
        m4b_merge::metadata::Chapter::new(
            "Initial Chapter 2",
            Duration::from_secs(5),
            Duration::from_secs(5),
        ),
    ];
    tagger.embed_chapters(&m4b_path, &initial_chapters).expect("Failed to embed initial chapters");

    // Verify initial chapters
    let read_chapters =
        m4b_merge::chapters::read_chapters(&m4b_path).expect("Failed to read chapters");
    assert_eq!(read_chapters.len(), 2, "Should have 2 initial chapters");

    // Second embed: 3 chapters (should replace)
    let new_chapters = vec![
        m4b_merge::metadata::Chapter::new("New Chapter 1", Duration::ZERO, Duration::from_secs(3)),
        m4b_merge::metadata::Chapter::new(
            "New Chapter 2",
            Duration::from_secs(3),
            Duration::from_secs(3),
        ),
        m4b_merge::metadata::Chapter::new(
            "New Chapter 3",
            Duration::from_secs(6),
            Duration::from_secs(4),
        ),
    ];
    tagger.embed_chapters(&m4b_path, &new_chapters).expect("Failed to embed new chapters");

    // Verify chapters were replaced
    let read_chapters =
        m4b_merge::chapters::read_chapters(&m4b_path).expect("Failed to read chapters");

    assert_eq!(read_chapters.len(), 3, "Should have 3 chapters after replacement");
    assert_eq!(
        read_chapters[0].title, "New Chapter 1",
        "First chapter should be the new first chapter"
    );
    assert_eq!(
        read_chapters[1].title, "New Chapter 2",
        "Second chapter should be the new second chapter"
    );
    assert_eq!(
        read_chapters[2].title, "New Chapter 3",
        "Third chapter should be the new third chapter"
    );
}

#[test]
fn test_chapter_read_from_file_without_chapters() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 5-second test M4B file without chapters
    create_test_m4b(&m4b_path, 5);

    // Read chapters from file that has none
    let read_chapters =
        m4b_merge::chapters::read_chapters(&m4b_path).expect("Failed to read chapters");

    assert!(read_chapters.is_empty(), "File without chapters should return empty list");
}
