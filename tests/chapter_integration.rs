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

/// Create a test M4B with the `moov` atom written before `mdat` (streaming
/// layout), the layout that exposes chunk-offset corruption after post-write
/// byte splicing.
fn create_faststart_m4b(path: &Path, duration_secs: u32) {
    let status = Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=44100:cl=mono",
            "-t",
            &duration_secs.to_string(),
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(path)
        .status()
        .expect("Failed to run FFmpeg");

    assert!(status.success(), "FFmpeg failed to create faststart fixture");
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
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

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
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

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
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

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
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");
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
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

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
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

    assert!(read_chapters.is_empty(), "File without chapters should return empty list");
}

#[test]
fn test_chapter_embed_caps_count_at_chpl_limit() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");
    create_test_m4b(&m4b_path, 10);

    // 256 chapters would wrap the chpl atom's one-byte entry count.
    let chapters: Vec<m4b_merge::metadata::Chapter> = (0..=255)
        .map(|i| {
            m4b_merge::metadata::Chapter::new(
                format!("Chapter {}", i),
                Duration::from_millis(i as u64 * 10),
                Duration::from_millis(10),
            )
        })
        .collect();

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &chapters).expect("embed truncated chapter list");

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read back chapters");
    assert_eq!(tag.chapter_list().len(), 255, "chpl atom holds at most 255 chapters");
    assert_eq!(tag.chapter_list()[0].title, "Chapter 0");
    assert_eq!(tag.chapter_list()[254].title, "Chapter 254");
}

#[test]
fn test_chapter_embed_caps_chpl_but_keeps_full_track() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");
    create_test_m4b(&m4b_path, 10);

    // 256 chapters: the Nero `chpl` list is capped at the atom's one-byte
    // entry count, but the QuickTime chapter track must keep the full list.
    let chapters: Vec<m4b_merge::metadata::Chapter> = (0..=255)
        .map(|i| {
            m4b_merge::metadata::Chapter::new(
                format!("Chapter {}", i),
                Duration::from_millis(i as u64 * 10),
                Duration::from_millis(10),
            )
        })
        .collect();

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &chapters).expect("embed chapters");

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read back chapters");
    assert_eq!(tag.chapter_list().len(), 255, "chpl atom holds at most 255 chapters");
    assert_eq!(
        tag.chapter_track().len(),
        256,
        "the QuickTime chapter track keeps the uncapped list"
    );
    assert_eq!(
        tag.chapter_track()[255].title,
        "Chapter 255",
        "the final chapter must reach the track"
    );
}

#[test]
fn test_chapter_embed_multibyte_title_does_not_panic() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");
    create_test_m4b(&m4b_path, 10);

    // 400 bytes of 2-byte characters; byte 255 falls inside a character.
    let title = "é".repeat(200);
    let chapters =
        vec![m4b_merge::metadata::Chapter::new(title, Duration::ZERO, Duration::from_secs(10))];

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &chapters).expect("embed multibyte chapter title");

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read back chapters");
    let embedded = &tag.chapter_list()[0].title;
    assert_eq!(embedded, &"é".repeat(127), "title truncated on a char boundary under 255 bytes");
}

#[test]
fn test_chapter_read_prefers_track_when_chpl_list_is_truncated() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");
    create_test_m4b(&m4b_path, 30);

    // Reproduce the post-#464 shape: the QuickTime track holds all chapters
    // while the Nero `chpl` list is capped at the one-byte entry count.
    let mut tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    tag.chapter_list_mut().clear();
    tag.chapter_track_mut().clear();
    let all: Vec<mp4ameta::Chapter> = (0..257)
        .map(|i| {
            mp4ameta::Chapter::new(Duration::from_millis(i as u64 * 100), format!("Chapter {}", i))
        })
        .collect();
    tag.chapter_list_mut().extend(all[..255].iter().cloned());
    tag.chapter_track_mut().extend(all);
    tag.write_to_path(&m4b_path).expect("write chapters");

    let read_back =
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

    assert_eq!(read_back.len(), 257, "the full chapter track must win over the truncated list");
    assert_eq!(read_back[0].title, "Chapter 0");
    assert_eq!(read_back[256].title, "Chapter 256");
    assert_eq!(read_back[256].start_time, 25_600);
}

#[test]
fn test_chapter_read_prefers_list_over_stale_track() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");
    create_test_m4b(&m4b_path, 30);

    // Write the replacement chapters through the public API so both the
    // `chpl` list and the QuickTime track hold the same three chapters.
    let chapters = vec![
        m4b_merge::metadata::Chapter::new("New one", Duration::ZERO, Duration::from_secs(10)),
        m4b_merge::metadata::Chapter::new(
            "New two",
            Duration::from_secs(10),
            Duration::from_secs(10),
        ),
        m4b_merge::metadata::Chapter::new(
            "New three",
            Duration::from_secs(20),
            Duration::from_secs(10),
        ),
    ];
    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &chapters).expect("embed chapters");

    // Now desynchronize the two representations: keep the three-chapter
    // `chpl` list but write a stale ten-chapter QuickTime track, as an older
    // write of the same file by another tool would leave behind.
    let mut tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    assert_eq!(tag.chapter_list().len(), 3, "list must hold the replacement chapters");
    tag.chapter_track_mut().clear();
    tag.chapter_track_mut().extend(
        (0..10).map(|i| mp4ameta::Chapter::new(Duration::from_secs(i * 3), format!("Old {}", i))),
    );
    tag.write_to_path(&m4b_path).expect("write stale chapter track");
    let planted = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    assert_eq!(planted.chapter_list().len(), 3, "list must be untouched by the track write");
    assert_eq!(planted.chapter_track().len(), 10, "stale track must be present");

    // The two representations disagree without the list being the 255-entry
    // truncated cap, so `read_chapters` must treat the `chpl` list as the
    // canonical form rather than falling back to the stale track.
    let read_back =
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

    assert_eq!(read_back.len(), 3, "readback must follow the chpl list, not the stale track");
    assert_eq!(read_back[0].title, "New one");
    assert_eq!(read_back[2].title, "New three");
    assert_eq!(read_back[2].start_time, 20_000);
}

#[test]
fn test_chapter_embed_rejects_start_past_file_duration() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 10-second test M4B file
    create_test_m4b(&m4b_path, 10);

    // A chapter 25 hours into a 10-second file cannot be represented; the
    // embedding boundary must reject it instead of writing an unplayable entry.
    let long_start = Duration::from_secs(90_000);
    let chapters = vec![
        m4b_merge::metadata::Chapter::new("Early Chapter", Duration::ZERO, Duration::from_secs(5)),
        m4b_merge::metadata::Chapter::new("Late Chapter", long_start, Duration::from_secs(3_600)),
    ];

    let tagger = m4b_merge::tagging::Tagger::new();
    let err = tagger
        .embed_chapters(&m4b_path, &chapters)
        .expect_err("out-of-range chapter must be rejected");

    match &err {
        m4b_merge::tagging::TaggingError::ChaptersOutOfRange {
            count, playlength, details, ..
        } => {
            assert_eq!(*count, 1);
            assert_eq!(playlength, "00:00:10.000");
            assert!(details.contains("Late Chapter"), "{details}");
        }
        other => panic!("expected ChaptersOutOfRange, got {other:?}"),
    }

    // Nothing was written: the earlier, in-range chapter is not embedded either.
    let read_back =
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");
    assert!(read_back.is_empty(), "rejected embed must leave the chapter list untouched");
}

#[test]
fn test_process_completed_book_does_not_embed_out_of_range_chapters() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("completed.m4b");
    create_test_m4b(&m4b_path, 10);

    let metadata = m4b_merge::metadata::BookMetadata::new("id", "Title", "description")
        .add_chapter(m4b_merge::metadata::Chapter::new(
            "Intro",
            Duration::ZERO,
            Duration::from_secs(5),
        ))
        .add_chapter(m4b_merge::metadata::Chapter::new(
            "End Credits",
            Duration::from_secs(20),
            Duration::from_secs(30),
        ));

    let tagger = m4b_merge::tagging::Tagger::new();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime
        .block_on(tagger.process_completed_book(&m4b_path, &metadata, &["a.mp3"], None))
        .expect("chapter failure is logged, not fatal");

    // `process_completed_book` warns and continues, but the invalid chapter list
    // must not have been written.
    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    assert!(
        tag.chapter_list().is_empty(),
        "out-of-range chapters must not reach the file: {:?}",
        tag.chapter_list()
    );
}

// ---------------------------------------------------------------------------
// Foreign-clock chapter track regression
//
// mp4ameta 0.13.0 regenerates a chapter track's sample table in the movie
// timescale (`mvhd`) while leaving the track's own media timescale (`mdhd`)
// untouched. Files produced by other tools commonly use a 44.1 kHz chapter
// track under a 600 Hz movie header, so every write rescales the chapter times
// by `mvhd.timescale / mdhd.timescale`; the corruption compounds across
// writes and consumers such as ffprobe and Audiobookshelf then show every
// chapter at 0:00. `process_completed_book` performs three writes per book.
// ---------------------------------------------------------------------------

/// FFmetadata for a 10-second book with the three fixture chapters.
const FOREIGN_CLOCK_FFMETADATA: &str = "\
;FFMETADATA1
title=Foreign Clock
[CHAPTER]
TIMEBASE=1/1000
START=0
END=2000
title=Intro
[CHAPTER]
TIMEBASE=1/1000
START=2000
END=7000
title=One
[CHAPTER]
TIMEBASE=1/1000
START=7000
END=10000
title=Outro
";

/// 1x1 transparent PNG, used to exercise the cover-art write pass.
const TINY_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 0, 2, 0, 0, 5, 0, 1,
    122, 94, 171, 63, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// A parsed MP4 atom inside the fixture buffer.
#[derive(Clone)]
struct Atom {
    /// Offset of the size field.
    start: usize,
    /// Size of the atom header (8 bytes, or 16 for 64-bit sizes).
    header: usize,
    /// Offset one past the atom.
    end: usize,
    fourcc: [u8; 4],
}

fn be_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(buf[offset..offset + 4].try_into().expect("4-byte read"))
}

fn be_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(buf[offset..offset + 8].try_into().expect("8-byte read"))
}

fn atoms_in(buf: &[u8], start: usize, end: usize) -> Vec<Atom> {
    let mut atoms = Vec::new();
    let mut pos = start;
    while pos + 8 <= end {
        let size = be_u32(buf, pos) as usize;
        let fourcc: [u8; 4] = buf[pos + 4..pos + 8].try_into().expect("fourcc");
        let (size, header) = match size {
            0 => (end - pos, 8),
            1 => (be_u64(buf, pos + 8) as usize, 16),
            size => (size, 8),
        };
        assert!(size >= header && pos + size <= end, "malformed atom {fourcc:?}");
        atoms.push(Atom { start: pos, header, end: pos + size, fourcc });
        pos += size;
    }
    atoms
}

impl Atom {
    fn child(&self, buf: &[u8], fourcc: &[u8; 4]) -> Option<Atom> {
        atoms_in(buf, self.start + self.header, self.end)
            .into_iter()
            .find(|atom| &atom.fourcc == fourcc)
    }
}

fn descend(buf: &[u8], mut atom: Atom, path: &[&[u8; 4]]) -> Option<Atom> {
    for fourcc in path {
        atom = atom.child(buf, fourcc)?;
    }
    Some(atom)
}

/// Locate the chapter track referenced by a `tref`/`chap` entry.
fn chapter_track(buf: &[u8]) -> Atom {
    let moov = atoms_in(buf, 0, buf.len())
        .into_iter()
        .find(|atom| &atom.fourcc == b"moov")
        .expect("moov atom");
    let traks: Vec<Atom> = atoms_in(buf, moov.start + moov.header, moov.end)
        .into_iter()
        .filter(|atom| &atom.fourcc == b"trak")
        .collect();

    let chapter_id = traks
        .iter()
        .find_map(|trak| {
            let chap = descend(buf, trak.clone(), &[b"tref", b"chap"])?;
            // `tref` children hold raw track IDs; unlike full boxes they carry
            // no version/flags word.
            Some(be_u32(buf, chap.start + chap.header))
        })
        .expect("chap track reference");

    let track_id = |trak: &Atom| {
        let tkhd = trak.child(buf, b"tkhd").expect("tkhd atom");
        let version = buf[tkhd.start + tkhd.header];
        let offset = tkhd.start + tkhd.header + if version == 0 { 12 } else { 20 };
        be_u32(buf, offset)
    };

    traks.into_iter().find(|trak| track_id(trak) == chapter_id).expect("chapter track")
}

/// Return `(timescale, mdhd duration, sum of stts sample durations)`.
fn chapter_track_clock(path: &Path) -> (u32, u64, u64) {
    let buf = std::fs::read(path).expect("read fixture");
    let trak = chapter_track(&buf);
    let mdhd = descend(&buf, trak.clone(), &[b"mdia", b"mdhd"]).expect("mdhd atom");
    let stts = descend(&buf, trak, &[b"mdia", b"minf", b"stbl", b"stts"]).expect("stts atom");

    let version = buf[mdhd.start + mdhd.header];
    assert_eq!(version, 0, "fixture mdhd is version 0");
    let timescale = be_u32(&buf, mdhd.start + mdhd.header + 12);
    let duration = be_u32(&buf, mdhd.start + mdhd.header + 16) as u64;

    let entries = be_u32(&buf, stts.start + stts.header + 4) as usize;
    let mut total = 0u64;
    for entry in 0..entries {
        let offset = stts.start + stts.header + 8 + entry * 8;
        total += be_u32(&buf, offset) as u64 * be_u32(&buf, offset + 4) as u64;
    }

    (timescale, duration, total)
}

/// Rewrite the chapter track so its media clock (44.1 kHz) differs from the
/// movie clock (600 Hz), like chapter tracks produced by Audible tooling.
fn rescale_chapter_track(path: &Path) {
    const NEW_TIMESCALE: u32 = 44_100;

    let mut buf = std::fs::read(path).expect("read fixture");
    let trak = chapter_track(&buf);
    let mdhd = descend(&buf, trak.clone(), &[b"mdia", b"mdhd"]).expect("mdhd atom");
    let stts = descend(&buf, trak, &[b"mdia", b"minf", b"stbl", b"stts"]).expect("stts atom");

    let version = buf[mdhd.start + mdhd.header];
    assert_eq!(version, 0, "fixture mdhd is version 0");
    let timescale_offset = mdhd.start + mdhd.header + 12;
    let duration_offset = mdhd.start + mdhd.header + 16;
    let old_timescale = be_u32(&buf, timescale_offset);
    assert_eq!(be_u32(&buf, duration_offset), 6_000, "10 s of movie ticks");

    let entries = be_u32(&buf, stts.start + stts.header + 4) as usize;
    assert_eq!(entries, 3, "fixture has three chapters");

    let mut new_duration = 0u64;
    for entry in 0..entries {
        let offset = stts.start + stts.header + 8 + entry * 8 + 4;
        let old = be_u32(&buf, offset) as u64;
        assert_eq!(
            old * NEW_TIMESCALE as u64 % old_timescale as u64,
            0,
            "fixture durations must rescale losslessly"
        );
        let scaled = old * NEW_TIMESCALE as u64 / old_timescale as u64;
        buf[offset..offset + 4].copy_from_slice(&(scaled as u32).to_be_bytes());
        new_duration += scaled;
    }

    buf[timescale_offset..timescale_offset + 4].copy_from_slice(&NEW_TIMESCALE.to_be_bytes());
    buf[duration_offset..duration_offset + 4].copy_from_slice(&(new_duration as u32).to_be_bytes());

    std::fs::write(path, &buf).expect("write fixture");
}

/// Create a 10-second M4B with a 44.1 kHz chapter track and a 600 Hz movie header.
fn create_foreign_clock_m4b(path: &Path) {
    let metadata_path = path.with_extension("ffmeta");
    std::fs::write(&metadata_path, FOREIGN_CLOCK_FFMETADATA).expect("write ffmetadata");

    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-y", "-v", "error", "-f", "lavfi"])
        .args(["-i", "anullsrc=r=44100:cl=mono:d=10"])
        .arg("-i")
        .arg(&metadata_path)
        .args([
            "-map",
            "0:a",
            "-map_metadata",
            "1",
            "-map_chapters",
            "1",
            "-c:a",
            "aac",
            "-b:a",
            "64k",
            "-movie_timescale",
            "600",
        ])
        .arg(path)
        .status()
        .expect("Failed to run FFmpeg");

    assert!(status.success(), "FFmpeg failed to create the foreign-clock fixture");
    rescale_chapter_track(path);
}

fn fixture_chapters() -> Vec<m4b_merge::metadata::Chapter> {
    vec![
        m4b_merge::metadata::Chapter::new("Intro", Duration::ZERO, Duration::from_secs(2)),
        m4b_merge::metadata::Chapter::new("One", Duration::from_secs(2), Duration::from_secs(5)),
        m4b_merge::metadata::Chapter::new("Outro", Duration::from_secs(7), Duration::from_secs(3)),
    ]
}

fn chapter_starts_ms(chapters: &[m4b_merge::chapters::Chapter]) -> Vec<u64> {
    chapters.iter().map(|chapter| chapter.start_time).collect()
}

fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Decode the file end to end; a corrupt chunk offset makes this fail.
fn decode_ok(path: &Path) -> bool {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .expect("Failed to run FFmpeg decode");

    output.status.success() && output.stderr.is_empty()
}

fn ffprobe_chapter_count(path: &Path) -> usize {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_chapters", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .expect("Failed to run ffprobe");
    assert!(output.status.success(), "ffprobe failed on {}", path.display());

    String::from_utf8_lossy(&output.stdout).lines().filter(|line| !line.trim().is_empty()).count()
}

/// `(payload_start, end)` of the `mdat` atom.
fn mdat_payload_range(path: &Path) -> (u64, u64) {
    let buf = std::fs::read(path).expect("read fixture");
    let mdat = atoms_in(&buf, 0, buf.len())
        .into_iter()
        .find(|atom| &atom.fourcc == b"mdat")
        .expect("mdat atom");

    ((mdat.start + mdat.header) as u64, mdat.end as u64)
}

/// True when `moov` is stored before `mdat` (faststart layout).
fn moov_precedes_mdat(path: &Path) -> bool {
    let buf = std::fs::read(path).expect("read fixture");
    let atoms = atoms_in(&buf, 0, buf.len());
    let moov = atoms.iter().find(|atom| &atom.fourcc == b"moov").expect("moov atom");
    let mdat = atoms.iter().find(|atom| &atom.fourcc == b"mdat").expect("mdat atom");

    moov.start < mdat.start
}

/// All absolute chunk offsets from `stco` atoms (the audio track uses `stco`).
fn stco_offsets(path: &Path) -> Vec<u64> {
    let buf = std::fs::read(path).expect("read fixture");
    let moov = atoms_in(&buf, 0, buf.len())
        .into_iter()
        .find(|atom| &atom.fourcc == b"moov")
        .expect("moov atom");

    let mut offsets = Vec::new();
    for trak in atoms_in(&buf, moov.start + moov.header, moov.end)
        .into_iter()
        .filter(|atom| &atom.fourcc == b"trak")
    {
        let Some(stco) = descend(&buf, trak, &[b"mdia", b"minf", b"stbl", b"stco"]) else {
            continue;
        };
        let content = stco.start + stco.header;
        let count = be_u32(&buf, content + 4) as usize;
        for index in 0..count {
            offsets.push(u64::from(be_u32(&buf, content + 8 + index * 4)));
        }
    }

    offsets
}

#[test]
fn test_foreign_clock_chapter_track_keeps_times_across_writes() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("foreign-clock.m4b");
    create_foreign_clock_m4b(&m4b_path);

    // Precondition: the chapter track is internally consistent in its own clock.
    assert_eq!(chapter_track_clock(&m4b_path), (44_100, 441_000, 441_000));
    let before =
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("read fixture chapters");
    assert_eq!(chapter_starts_ms(&before), vec![0, 2_000, 7_000]);

    // The same write sequence as `process_completed_book`: metadata, cover art, chapters.
    let tagger = m4b_merge::tagging::Tagger::new();
    let mut metadata =
        m4b_merge::metadata::BookMetadata::new("fixture", "Foreign Clock", "description");
    metadata.authors = vec!["Author".to_string()];

    tagger.write_metadata(&m4b_path, &metadata).expect("write metadata");
    tagger.embed_cover_data(&m4b_path, TINY_PNG).expect("embed cover");
    tagger.embed_chapters(&m4b_path, &fixture_chapters()).expect("embed chapters");

    // The regenerated sample table must stay in the chapter track's own clock.
    assert_eq!(chapter_track_clock(&m4b_path), (44_100, 441_000, 441_000));

    let after = m4b_merge::chapters::read_chapters(&m4b_path, None)
        .expect("read chapters after metadata writes");
    assert_eq!(
        chapter_starts_ms(&after),
        vec![0, 2_000, 7_000],
        "chapter starts must survive repeated tag writes"
    );

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    let track_starts: Vec<u64> =
        tag.chapter_track().iter().map(|chapter| chapter.start.as_millis() as u64).collect();
    assert_eq!(
        track_starts,
        vec![0, 2_000, 7_000],
        "QuickTime chapter track must keep the same starts"
    );
}

#[test]
fn test_embed_writes_chpl_and_quicktime_track() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("dual-format.m4b");
    create_test_m4b(&m4b_path, 10);

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &fixture_chapters()).expect("embed chapters");

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    assert_eq!(tag.chapter_list().len(), 3, "Nero chpl chapter list");
    assert_eq!(tag.chapter_track().len(), 3, "QuickTime chapter text track");
    assert_eq!(tag.chapter_list()[1].start, Duration::from_secs(2));
    assert_eq!(tag.chapter_track()[1].start, Duration::from_secs(2));

    let buf = std::fs::read(&m4b_path).expect("read output");
    let trak = chapter_track(&buf);
    let hdlr = descend(&buf, trak, &[b"mdia", b"hdlr"]).expect("text hdlr");
    let subtype = &buf[hdlr.start + hdlr.header + 8..hdlr.start + hdlr.header + 12];
    assert_eq!(subtype, b"text", "chapter track must use the text handler");

    assert!(decode_ok(&m4b_path), "output must decode cleanly");
    if ffprobe_available() {
        assert_eq!(ffprobe_chapter_count(&m4b_path), 3);
    }
}

#[test]
fn test_embed_on_faststart_input_keeps_chunks_decodable() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("faststart.m4b");
    create_faststart_m4b(&m4b_path, 10);
    assert!(moov_precedes_mdat(&m4b_path), "fixture must use the faststart layout");

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &fixture_chapters()).expect("embed chapters");

    assert!(moov_precedes_mdat(&m4b_path), "layout must be preserved");
    assert!(decode_ok(&m4b_path), "audio must remain decodable after chapter embedding");

    let (mdat_start, mdat_end) = mdat_payload_range(&m4b_path);
    let offsets = stco_offsets(&m4b_path);
    assert!(!offsets.is_empty(), "audio track must keep its chunk offsets");
    for offset in offsets {
        assert!(
            offset >= mdat_start && offset < mdat_end,
            "chunk offset {offset} must point inside mdat [{mdat_start}, {mdat_end})"
        );
    }

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    assert_eq!(tag.chapter_track().len(), 3);
}

#[test]
fn test_read_chapters_derives_last_duration_from_media_duration() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("last-duration.m4b");
    create_test_m4b(&m4b_path, 10);

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &fixture_chapters()).expect("embed chapters");

    let chapters = m4b_merge::chapters::read_chapters(&m4b_path, None).expect("read chapters");
    let last = chapters.last().expect("chapters");
    assert_eq!(last.start_time, 7_000);
    assert_eq!(last.duration, 3_000, "last chapter must end at the media duration");
}

#[test]
fn test_embed_replaces_both_chapter_representations() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("replace-both.m4b");
    create_test_m4b(&m4b_path, 10);

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.embed_chapters(&m4b_path, &fixture_chapters()).expect("first embed");

    let replacement = vec![
        m4b_merge::metadata::Chapter::new("New One", Duration::ZERO, Duration::from_secs(4)),
        m4b_merge::metadata::Chapter::new(
            "New Two",
            Duration::from_secs(4),
            Duration::from_secs(6),
        ),
    ];
    tagger.embed_chapters(&m4b_path, &replacement).expect("second embed");

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read tag");
    for chapters in [tag.chapter_list(), tag.chapter_track()] {
        assert_eq!(chapters.len(), 2);
        let titles: Vec<&str> = chapters.iter().map(|chapter| chapter.title.as_str()).collect();
        assert_eq!(titles, vec!["New One", "New Two"]);
    }
    assert!(decode_ok(&m4b_path));
}

#[test]
fn test_written_metadata_uses_composer_for_narrators() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("metadata.m4b");
    create_test_m4b(&m4b_path, 2);

    let metadata = m4b_merge::metadata::BookMetadata::new("id", "Title", "description")
        .add_author("Author One")
        .add_narrator("Narrator One")
        .add_narrator("Narrator Two")
        .with_series("Series Name", "2");

    let tagger = m4b_merge::tagging::Tagger::new();
    tagger.write_metadata(&m4b_path, &metadata).expect("write metadata");
    tagger.write_metadata(&m4b_path, &metadata).expect("rewrite metadata");

    let tag = mp4ameta::Tag::read_from_path(&m4b_path).expect("read back tags");
    assert_eq!(tag.composer(), Some("Narrator One, Narrator Two"));
    assert_eq!(tag.artist(), Some("Author One"));
    assert_eq!(tag.grouping(), Some("Series: Series Name #2"));
}
