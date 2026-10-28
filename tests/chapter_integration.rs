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
fn test_chapter_long_duration() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let m4b_path = temp_dir.path().join("test.m4b");

    // Create a 10-second test M4B file
    create_test_m4b(&m4b_path, 10);

    // Define a chapter with start_time > 24 hours (90000s = 25 hours)
    // to verify no overflow or truncation occurs
    let long_start = Duration::from_secs(90_000); // 25 hours
    let chapters = vec![
        m4b_merge::metadata::Chapter::new("Early Chapter", Duration::ZERO, Duration::from_secs(5)),
        m4b_merge::metadata::Chapter::new("Late Chapter", long_start, Duration::from_secs(3_600)),
    ];

    // Embed chapters using Tagger
    let tagger = m4b_merge::tagging::Tagger::new();
    tagger
        .embed_chapters(&m4b_path, &chapters)
        .expect("Failed to embed chapters with long duration");

    // Read back chapters
    let read_chapters =
        m4b_merge::chapters::read_chapters(&m4b_path, None).expect("Failed to read chapters");

    // Verify we have both chapters
    assert_eq!(read_chapters.len(), 2, "Should have 2 chapters after embedding long duration");

    // Verify the long-duration chapter start_time survived without overflow
    let late_chapter = read_chapters
        .iter()
        .find(|c| c.title == "Late Chapter")
        .expect("Late Chapter should be readable back");

    assert_eq!(
        late_chapter.start_time,
        (long_start.as_secs() * 1000) as u64,
        "Late chapter start_time should be {}ms (25 hours), got {}ms",
        long_start.as_secs() * 1000,
        late_chapter.start_time
    );

    // Duration is derived from the next chapter's start_time by mp4ameta.
    // For the last chapter there is no "next chapter", so duration may be 0.
    // The important invariant is that the start_time survived without overflow.
    assert_eq!(late_chapter.duration, 0, "Last chapter duration should be 0 (no next chapter)");
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
