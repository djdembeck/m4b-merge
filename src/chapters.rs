use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::tagging::CHAPTER_LIST_MAX_CHAPTERS;

const MAX_TITLE_LEN: u32 = 10 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start_time: u64,
    pub duration: u64,
}

/// Generate one chapter per input track, laid end to end.
///
/// This mirrors m4b-tool's `ChaptersFromFileTracks` followed by the numbering in
/// the pre-rewrite `m4b_helper.py`: the merged output is the concatenation of the
/// tracks in order, so each track becomes a `Chapter NN` chapter whose start time
/// is the sum of the preceding track durations.
///
/// `durations` are track durations in milliseconds. Zero-length tracks cannot
/// produce a chapter and are skipped; the running offset only advances by the
/// durations that are kept, so start times always describe the merged timeline.
pub fn generate_chapters_from_durations(durations: &[u64]) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = Vec::with_capacity(durations.len());
    let mut offset: u64 = 0;

    for duration in durations.iter().copied().filter(|d| *d > 0) {
        chapters.push(Chapter {
            title: format!("Chapter {:02}", chapters.len() + 1),
            start_time: offset,
            duration,
        });
        offset = offset.saturating_add(duration);
    }

    chapters
}

/// Read chapters from an M4B file.
///
/// Prefers the Nero `chpl` list because its start times are exact; ffprobe
/// prefers the QuickTime chapter text track, whose sample timeline is rebased
/// to zero. Falls back to ffprobe, then to direct chpl parsing.
/// Pass `ffprobe_path` to use a specific ffprobe binary; `None` uses PATH.
pub fn read_chapters(
    path: &Path,
    ffprobe_path: Option<&str>,
) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    if let Ok(tag) = mp4ameta::Tag::read_from_path(path) {
        let list = tag.chapter_list();
        let track = tag.chapter_track();

        if !track.is_empty() {
            // A `chpl` list stores its entry count in one byte, so a file with
            // both representations can carry a list truncated at 255 entries
            // while the track holds the full set: use the track only when the
            // list is exactly that cap and repeats the track's prefix.
            //
            // Otherwise the two disagree and the list is authoritative: a
            // truncated list means the track is also incomplete, and a
            // matching-length disagreement says the two representations were
            // written independently (e.g. by a tool other than
            // `embed_chapters`) and the chpl list is the canonical form.
            let source =
                if list.is_empty() || list_is_truncated_chpl(list, track) { track } else { list };

            let chapters = chapters_from_mp4ameta(source, tag.duration());
            if !chapters.is_empty() {
                return Ok(chapters);
            }
        }
    }

    // Try to read chapters using ffprobe first
    if let Ok(chapters) = read_chapters_ffprobe(path, ffprobe_path) {
        if !chapters.is_empty() {
            return Ok(chapters);
        }
    }

    // Fall back to parsing the chpl atom directly
    read_chapters_from_atom(path)
}

/// Tolerance when matching a `chpl` start against a chapter-track start.
///
/// The list stores times in a fixed 10 MHz clock while the track uses its own
/// timescale, so the same instant can land in a neighbouring millisecond.
const CHAPTER_START_TOLERANCE: Duration = Duration::from_millis(20);

/// Whether `list` is a `chpl` list truncated at the atom's entry cap.
///
/// True only when the list is exactly `CHAPTER_LIST_MAX_CHAPTERS` long, the
/// track is longer, and every listed chapter matches the track's prefix. That
/// is the shape `embed_chapters` produces for a >255-chapter book; any other
/// disagreement between the representations means the track is stale.
fn list_is_truncated_chpl(list: &[mp4ameta::Chapter], track: &[mp4ameta::Chapter]) -> bool {
    if list.len() != CHAPTER_LIST_MAX_CHAPTERS || track.len() <= list.len() {
        return false;
    }

    list.iter().zip(track).all(|(listed, tracked)| {
        listed.title == tracked.title
            && listed.start.abs_diff(tracked.start) <= CHAPTER_START_TOLERANCE
    })
}

/// Derive durations from chapter start times.
///
/// Both chapter sources store starts, not durations, so each chapter's
/// duration is the gap to the next start; the last chapter runs to
/// `media_duration`, or keeps a duration of 0 rather than a guess when the
/// media duration is unknown.
fn derive_durations(chapters: &mut [Chapter], media_duration: Option<Duration>) {
    for index in 0..chapters.len() {
        if index + 1 < chapters.len() {
            chapters[index].duration =
                chapters[index + 1].start_time.saturating_sub(chapters[index].start_time);
        }
    }
    if let (Some(media_duration), Some(last)) = (media_duration, chapters.last_mut()) {
        last.duration = (media_duration.as_millis() as u64).saturating_sub(last.start_time);
    }
}

/// Convert mp4ameta chapters into chapters.
///
/// The source stores starts, not durations, so each chapter's duration is the
/// gap to the next start; the last chapter runs to `media_duration`.
fn chapters_from_mp4ameta(track: &[mp4ameta::Chapter], media_duration: Duration) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = track
        .iter()
        .map(|chapter| Chapter {
            title: chapter.title.clone(),
            start_time: chapter.start.as_millis() as u64,
            duration: 0,
        })
        .collect();

    derive_durations(&mut chapters, Some(media_duration));

    chapters
}

/// Read chapters using ffprobe.
///
/// Pass `ffprobe_path` to use a specific ffprobe binary; `None` uses PATH.
fn read_chapters_ffprobe(
    path: &Path,
    ffprobe_path: Option<&str>,
) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    let probe_cmd = ffprobe_path.unwrap_or("ffprobe");
    let output = Command::new(probe_cmd)
        .args(["-v", "quiet", "-print_format", "json", "-show_chapters"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("ffprobe error: {}", stderr);
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout)?;

    if let Some(chapters_array) = json.get("chapters").and_then(|c| c.as_array()) {
        let mut chapters = Vec::new();

        for chapter in chapters_array {
            let start_time = chapter
                .get("start_time")
                .and_then(|t| t.as_str())
                .and_then(|t| t.parse::<f64>().ok())
                .unwrap_or(0.0);

            let end_time = chapter
                .get("end_time")
                .and_then(|t| t.as_str())
                .and_then(|t| t.parse::<f64>().ok())
                .unwrap_or(0.0);

            let duration = if end_time > start_time { end_time - start_time } else { 0.0 };

            let title = chapter
                .get("tags")
                .and_then(|t| t.get("title"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();

            // Convert from seconds to milliseconds
            chapters.push(Chapter {
                title,
                start_time: (start_time * 1000.0).round() as u64,
                duration: (duration * 1000.0).round() as u64,
            });
        }

        return Ok(chapters);
    }

    Ok(Vec::new())
}

/// Read chapters by parsing the chpl atom directly.
///
/// The movie header's `mvhd` supplies the media duration the last chapter
/// runs to, matching `chapters_from_mp4ameta`; without it, the last chapter
/// keeps a duration of 0.
fn read_chapters_from_atom(path: &Path) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    let mut file = std::fs::File::open(path)?;

    // Read ftyp
    let mut buffer = [0u8; 8];
    file.read_exact(&mut buffer)?;
    let ftyp_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    let ftyp_ident = std::str::from_utf8(&buffer[4..8]).unwrap_or("");

    if ftyp_ident != "ftyp" {
        return Err("Not a valid MP4 file".into());
    }

    file.seek(SeekFrom::Start(ftyp_len as u64))?;

    // Search for moov atom
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read < 8 {
            break;
        }

        let atom_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        let atom_ident = std::str::from_utf8(&buffer[4..8]).unwrap_or("");

        if atom_len < 8 {
            break;
        } else if atom_ident == "moov" {
            let moov_end = file.stream_position()? + (atom_len as u64 - 8);
            return search_chapters_in_moov(&mut file, moov_end);
        } else if atom_len == 8 {
            continue;
        } else {
            file.seek(SeekFrom::Current((atom_len as i64) - 8))?;
        }
    }

    Ok(Vec::new())
}

/// Read the media duration in milliseconds from an `mvhd` atom.
///
/// `mvhd` is the movie header: its `timescale` and `duration` describe the
/// whole media timeline, so the direct-atom fallback derives the last
/// chapter's duration the same way `chapters_from_mp4ameta` does. Returns
/// `None` when the atom is truncated, unknown, carries a zero timescale, or
/// holds a duration too large to scale to milliseconds.
fn read_media_duration_from_mvhd(file: &mut std::fs::File, content_len: u32) -> Option<Duration> {
    let mut buffer = vec![0; content_len as usize];
    if file.read_exact(&mut buffer).is_err() {
        return None;
    }

    // Per ISO 14496-12: mvhd content = version(1) + flags(3) +
    // creation_time + modification_time + timescale(4) + duration. v0
    // timestamps and durations are 4 bytes, v1 is 8.
    let stamp = match buffer.first()? {
        0 => 4usize,
        1 => 8,
        _ => return None,
    };

    let timescale_offset = 4 + 2 * stamp; // version/flags + two timestamps
    let duration_offset = timescale_offset + 4;
    if buffer.len() < duration_offset + stamp {
        return None;
    }

    let timescale =
        u32::from_be_bytes(buffer[timescale_offset..timescale_offset + 4].try_into().ok()?);
    if timescale == 0 {
        return None;
    }

    let mut duration: u64 = 0;
    for &byte in &buffer[duration_offset..duration_offset + stamp] {
        duration = (duration << 8) | (byte as u64);
    }

    // A v1 `duration` is 64 bits, so a sentinel such as `u64::MAX` would
    // overflow the scale to milliseconds: treat any value that does not fit
    // as unknown, the way mp4ameta's `scale_duration` does. Dividing after
    // the multiply keeps the same result as the unchecked form.
    let millis = duration.checked_mul(1000)? / timescale as u64;

    Some(Duration::from_millis(millis))
}

fn search_chapters_in_moov(
    file: &mut std::fs::File,
    moov_end: u64,
) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    // The movie header carries the media duration the last chapter runs to.
    let mut media_duration: Option<Duration> = None;

    while file.stream_position()? < moov_end {
        let mut buffer = [0u8; 8];
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read < 8 {
            break;
        }

        let atom_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        let atom_ident = std::str::from_utf8(&buffer[4..8]).unwrap_or("");

        if atom_len < 8 {
            break;
        } else if atom_ident == "mvhd" {
            let mvhd_end = file.stream_position()? + (atom_len as u64 - 8);
            media_duration = read_media_duration_from_mvhd(file, atom_len - 8);
            if file.stream_position()? != mvhd_end {
                file.seek(SeekFrom::Start(mvhd_end))?;
            }
        } else if atom_ident == "udta" {
            let udta_end = file.stream_position()? + (atom_len as u64 - 8);
            return search_chapters_in_udta(file, udta_end, media_duration);
        } else if atom_len == 8 {
            continue;
        } else {
            file.seek(SeekFrom::Current((atom_len as i64) - 8))?;
        }
    }

    Ok(Vec::new())
}

fn search_chapters_in_udta(
    file: &mut std::fs::File,
    udta_end: u64,
    media_duration: Option<Duration>,
) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    while file.stream_position()? < udta_end {
        let mut buffer = [0u8; 8];
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read < 8 {
            break;
        }

        let atom_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        let atom_ident = std::str::from_utf8(&buffer[4..8]).unwrap_or("");

        if atom_len < 8 {
            break;
        } else if atom_ident == "meta" {
            let meta_end = file.stream_position()? + (atom_len as u64 - 8);
            return search_chapters_in_meta(file, meta_end, media_duration);
        } else if atom_ident == "chpl" {
            return parse_chpl_atom(file, atom_len - 8, media_duration);
        } else if atom_len == 8 {
            continue;
        } else {
            file.seek(SeekFrom::Current((atom_len as i64) - 8))?;
        }
    }

    Ok(Vec::new())
}

fn search_chapters_in_meta(
    file: &mut std::fs::File,
    meta_end: u64,
    media_duration: Option<Duration>,
) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    // meta atom has a 4-byte header (version/flags)
    let mut header = [0u8; 4];
    if file.read(&mut header)? != 4 {
        return Ok(Vec::new());
    }

    while file.stream_position()? < meta_end {
        let mut buffer = [0u8; 8];
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read < 8 {
            break;
        }

        let atom_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        let atom_ident = std::str::from_utf8(&buffer[4..8]).unwrap_or("");

        if atom_len < 8 {
            break;
        } else if atom_ident == "chpl" {
            return parse_chpl_atom(file, atom_len - 8, media_duration);
        } else if atom_len == 8 {
            continue;
        } else {
            file.seek(SeekFrom::Current((atom_len as i64) - 8))?;
        }
    }

    Ok(Vec::new())
}

fn parse_chpl_atom(
    file: &mut std::fs::File,
    content_len: u32,
    media_duration: Option<Duration>,
) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    let mut chapters = Vec::new();
    let mut buffer = vec![0; content_len as usize];
    file.read_exact(&mut buffer)?;

    // Per ISO 14496-12 / ffmpeg mov_read_chpl:
    // chpl content = version(1) + flags(3) + reserved(4 when version!=0) + chapter_count(1)
    if buffer.len() < 4 {
        return Ok(chapters);
    }
    let version = buffer[0];
    // skip version(1) + flags(3) + reserved(4 if version!=0)
    let header_size = if version != 0 { 8 } else { 4 };
    if buffer.len() < header_size + 1 {
        return Ok(chapters);
    }
    let num_chapters = buffer[header_size] as u32;
    let mut offset = header_size + 1;

    for _ in 0..num_chapters {
        if buffer.len() < offset + 8 {
            break;
        }

        let start_time = u64::from_be_bytes([
            buffer[offset],
            buffer[offset + 1],
            buffer[offset + 2],
            buffer[offset + 3],
            buffer[offset + 4],
            buffer[offset + 5],
            buffer[offset + 6],
            buffer[offset + 7],
        ]);
        offset += 8;
        let start_time = start_time / 10_000; // 100-ns units -> ms

        if buffer.len() < offset + 1 {
            break;
        }
        let title_len = buffer[offset] as u32;
        offset += 1;

        if title_len == 0 || title_len > MAX_TITLE_LEN {
            break;
        }

        let title_len_usize = title_len as usize;
        if buffer.len() < offset + title_len_usize {
            break;
        }

        let title = String::from_utf8_lossy(&buffer[offset..offset + title_len_usize]).to_string();
        offset += title_len_usize;

        chapters.push(Chapter { title, start_time, duration: 0 });
    }

    // Starts only: derive the durations, the last chapter running to
    // `media_duration` when the movie header supplied one.
    derive_durations(&mut chapters, media_duration);

    Ok(chapters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;

    fn track(items: &[(&str, u64)]) -> Vec<mp4ameta::Chapter> {
        items
            .iter()
            .map(|(title, start_ms)| {
                mp4ameta::Chapter::new(Duration::from_millis(*start_ms), *title)
            })
            .collect()
    }

    #[test]
    fn test_chapters_from_mp4ameta_derives_durations() {
        let chapters = chapters_from_mp4ameta(
            &track(&[("Intro", 0), ("One", 2_000), ("Outro", 7_000)]),
            Duration::from_secs(10),
        );

        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "Intro");
        assert_eq!(chapters[0].start_time, 0);
        assert_eq!(chapters[0].duration, 2_000);
        assert_eq!(chapters[1].duration, 5_000);
        // Last chapter runs to the media duration, not to the last start.
        assert_eq!(chapters[2].duration, 3_000);
    }

    #[test]
    fn test_chapters_from_mp4ameta_handles_empty_and_short_media() {
        assert!(chapters_from_mp4ameta(&[], Duration::from_secs(10)).is_empty());

        // Media shorter than the last start must not underflow.
        let chapters = chapters_from_mp4ameta(&track(&[("Late", 5_000)]), Duration::from_secs(1));
        assert_eq!(chapters[0].duration, 0);
    }

    /// v0 `mvhd` content: `duration` media ticks at `timescale` ticks/second.
    fn mvhd(timescale: u32, duration: u32) -> Vec<u8> {
        let mut v = vec![0u8; 12]; // version + flags + two 4-byte timestamps
        v.extend_from_slice(&timescale.to_be_bytes());
        v.extend_from_slice(&duration.to_be_bytes());
        v.extend_from_slice(&[0u8; 8]); // rate, volume, zero padding
        v
    }

    /// v1 `mvhd` content: 8-byte timestamps and an 8-byte `duration`.
    fn mvhd_v1(timescale: u32, duration: u64) -> Vec<u8> {
        let mut v = vec![1u8]; // version
        v.extend_from_slice(&[0u8; 3]); // flags
        v.extend_from_slice(&[0u8; 16]); // two 8-byte timestamps
        v.extend_from_slice(&timescale.to_be_bytes());
        v.extend_from_slice(&duration.to_be_bytes());
        v.extend_from_slice(&[0u8; 8]); // rate, volume, zero padding
        v
    }

    /// Nero `chpl` atom content for starts in milliseconds.
    fn chpl_atom(entries: &[(&str, u64)]) -> Vec<u8> {
        let mut v = vec![0u8; 4]; // version + flags = 0
        v.push(entries.len() as u8);
        for (title, start_ms) in entries {
            v.extend_from_slice(&(*start_ms * 10_000).to_be_bytes());
            v.push(title.len() as u8);
            v.extend_from_slice(title.as_bytes());
        }
        v
    }

    /// ftyp + moov{ mvhd?, udta{ chpl } } fixture written to a temp file.
    fn moov_file(mvhd: Option<&[u8]>, chpl: &[u8]) -> Vec<u8> {
        let mut file = Vec::new();

        file.extend_from_slice(&16u32.to_be_bytes());
        file.extend_from_slice(b"ftypm4b");
        file.extend_from_slice(&[0u8; 5]); // pads to exactly 16 bytes

        let chpl_len = 8 + chpl.len() as u64;
        let udta_len = 8 + chpl_len;
        let mvhd_len = mvhd.map_or(0, |m| 8 + m.len() as u64);
        file.extend_from_slice(&(((8 + mvhd_len + udta_len) as u32).to_be_bytes()));
        file.extend_from_slice(b"moov");
        if let Some(m) = mvhd {
            file.extend_from_slice(&((8 + m.len() as u32).to_be_bytes()));
            file.extend_from_slice(b"mvhd");
            file.extend_from_slice(m);
        }
        file.extend_from_slice(&(udta_len as u32).to_be_bytes());
        file.extend_from_slice(b"udta");
        file.extend_from_slice(&(chpl_len as u32).to_be_bytes());
        file.extend_from_slice(b"chpl");
        file.extend_from_slice(chpl);
        file
    }

    /// Write `bytes` to a temp fixture file and run the direct-atom fallback
    /// on it; the `NamedTempFile` cleans up when dropped.
    fn chapters_from_atom_file(bytes: Vec<u8>) -> Vec<Chapter> {
        let mut temp_file = NamedTempFile::new().expect("create fixture");
        temp_file.write_all(&bytes).expect("write fixture");

        read_chapters_from_atom(temp_file.path()).expect("parse chpl")
    }

    #[test]
    fn test_parse_chpl_atom_last_duration_from_mvhd() {
        let file = moov_file(
            Some(&mvhd(1000, 10_000)),
            &chpl_atom(&[("Intro", 0), ("One", 2_000), ("Outro", 7_000)]),
        );

        let chapters = chapters_from_atom_file(file);
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "Intro");
        assert_eq!(chapters[0].start_time, 0);
        assert_eq!(chapters[0].duration, 2_000);
        assert_eq!(chapters[1].duration, 5_000);
        // Last chapter runs to the media duration, not 0, like the primary path.
        assert_eq!(chapters[2].start_time, 7_000);
        assert_eq!(chapters[2].duration, 3_000);
    }

    #[test]
    fn test_parse_chpl_atom_single_chapter_runs_to_media_duration() {
        let file = moov_file(Some(&mvhd(1000, 10_000)), &chpl_atom(&[("Only", 2_500)]));

        let chapters = chapters_from_atom_file(file);
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "Only");
        assert_eq!(chapters[0].start_time, 2_500);
        assert_eq!(chapters[0].duration, 7_500);
    }

    #[test]
    fn test_parse_chpl_atom_empty_list_and_missing_mvhd_keep_zero() {
        // An empty chpl has no last chapter, so the media duration is unused.
        let file = moov_file(Some(&mvhd(1000, 10_000)), &chpl_atom(&[]));
        assert!(chapters_from_atom_file(file).is_empty());

        // Without an mvhd there is no media duration, so the last chapter keeps
        // a duration of 0 rather than a guess.
        let file = moov_file(None, &chpl_atom(&[("One", 0), ("Two", 5_000)]));
        let chapters = chapters_from_atom_file(file);
        assert_eq!(chapters[1].start_time, 5_000);
        assert_eq!(chapters[1].duration, 0);
    }

    #[test]
    fn test_parse_chpl_atom_short_media_saturates_last_duration() {
        let file = moov_file(Some(&mvhd(1000, 1_000)), &chpl_atom(&[("Late", 5_000)]));

        let chapters = chapters_from_atom_file(file);
        // Media shorter than the last start must not underflow.
        assert_eq!(chapters[0].duration, 0);
    }

    #[test]
    fn test_parse_chpl_atom_v1_mvhd_and_overflowing_sentinel() {
        // A v1 header scales its 64-bit duration like v0 does.
        let file = moov_file(Some(&mvhd_v1(1000, 10_000)), &chpl_atom(&[("Late", 5_000)]));
        let chapters = chapters_from_atom_file(file);
        assert_eq!(chapters[0].duration, 5_000);

        // A v1 sentinel duration cannot be scaled to milliseconds: treat it as
        // unknown rather than overflowing.
        let file = moov_file(Some(&mvhd_v1(1000, u64::MAX)), &chpl_atom(&[("Late", 5_000)]));
        let chapters = chapters_from_atom_file(file);
        assert_eq!(chapters[0].duration, 0);
    }

    fn capped_list() -> Vec<mp4ameta::Chapter> {
        (0..CHAPTER_LIST_MAX_CHAPTERS)
            .map(|i| {
                mp4ameta::Chapter::new(Duration::from_millis(i as u64 * 100), format!("Ch{i}"))
            })
            .collect()
    }

    #[test]
    fn test_list_is_truncated_chpl_true_for_capped_matching_prefix() {
        let list = capped_list();
        let mut track = list.clone();
        track.push(mp4ameta::Chapter::new(Duration::from_millis(25_500), "Tail".to_string()));
        track.push(mp4ameta::Chapter::new(Duration::from_millis(25_600), "Tail2".to_string()));

        assert!(list_is_truncated_chpl(&list, &track));
    }

    #[test]
    fn test_list_is_truncated_chpl_false_for_stale_track_after_replacement() {
        // `embed_chapters` replaces the list (3 new chapters) but leaves an
        // existing 10-chapter track behind. That longer track is stale: the
        // list is not at the cap, so it must stay authoritative.
        let list: Vec<mp4ameta::Chapter> = (0..3)
            .map(|i| mp4ameta::Chapter::new(Duration::from_secs(i), format!("New{i}")))
            .collect();
        let track: Vec<mp4ameta::Chapter> = (0..10)
            .map(|i| mp4ameta::Chapter::new(Duration::from_secs(i), format!("Old{i}")))
            .collect();

        assert!(!list_is_truncated_chpl(&list, &track));
    }

    #[test]
    fn test_list_is_truncated_chpl_false_unless_capped_and_matching() {
        let list = capped_list();
        let mut track = list.clone();
        track.push(mp4ameta::Chapter::new(Duration::from_millis(25_500), "Tail".to_string()));

        // Capped list, identical prefix: only length matters here.
        assert!(list_is_truncated_chpl(&list, &track));

        // Track shorter than the list is never a truncation.
        assert!(!list_is_truncated_chpl(&list, &list));
        assert!(!list_is_truncated_chpl(&list, &list[..10]));

        // A list below the cap is a full list, not a truncated one.
        let shorter = &list[..254];
        assert!(!list_is_truncated_chpl(shorter, &track));

        // A prefix that diverges means the track belongs to another chapter set.
        let mut diverging_track = track.clone();
        diverging_track[100] =
            mp4ameta::Chapter::new(Duration::from_millis(10_000), "Different".to_string());
        assert!(!list_is_truncated_chpl(&list, &diverging_track));

        // Starts within the timescale rounding tolerance still match.
        let mut rounded_track = track.clone();
        rounded_track[7] = mp4ameta::Chapter::new(Duration::from_millis(700), "Ch7".to_string());
        assert!(list_is_truncated_chpl(&list, &rounded_track));

        // Starts beyond the tolerance do not.
        let mut shifted_track = track.clone();
        shifted_track[7] = mp4ameta::Chapter::new(Duration::from_millis(1_000), "Ch7".to_string());
        assert!(!list_is_truncated_chpl(&list, &shifted_track));
    }

    #[test]
    #[ignore = "This is a manual/local integration test. Run with: cargo test -- --ignored"]
    fn test_read_chapters() {
        // Find the test file - it might be in a subdirectory based on path_format
        let possible_paths = [
            PathBuf::from(
                std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string())
                    + "/output/Trailer Park Bikini Vampires [B0FDDCDXQ2].m4b",
            ),
            PathBuf::from(
                std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string())
                    + "/output/Virgil Knightley/Trailer Park Bikini Vampires.m4b",
            ),
        ];

        let test_file = possible_paths.iter()
            .find(|p| p.exists())
            .cloned()
            .expect("No test file found. Run: m4b-merge -i ~/input/'Trailer Park Bikini Vampires [B0FDDCDXQ2]' -o ~/output");

        let chapters = read_chapters(&test_file, None).expect("Failed to read chapters");

        assert!(chapters.len() >= 5, "Expected at least 5 chapters, found {}", chapters.len());

        for chapter in &chapters {
            assert!(!chapter.title.is_empty(), "Chapter title should not be empty");
        }

        assert_eq!(chapters[0].start_time, 0, "First chapter should start at time 0");

        println!("Found {} chapters:", chapters.len());
        for (i, chapter) in chapters.iter().enumerate() {
            println!(
                "  Chapter {}: '{}' (start: {}, duration: {})",
                i + 1,
                chapter.title,
                chapter.start_time,
                chapter.duration
            );
        }
    }

    #[test]
    fn test_generate_chapters_from_durations() {
        let chapters = generate_chapters_from_durations(&[5_000, 7_500, 1_000]);

        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "Chapter 01");
        assert_eq!(chapters[0].start_time, 0);
        assert_eq!(chapters[0].duration, 5_000);
        assert_eq!(chapters[1].start_time, 5_000);
        assert_eq!(chapters[1].duration, 7_500);
        assert_eq!(chapters[2].start_time, 12_500);
        assert_eq!(chapters[2].duration, 1_000);
    }

    #[test]
    fn test_generate_chapters_single_track() {
        let chapters = generate_chapters_from_durations(&[42_000]);

        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "Chapter 01");
        assert_eq!(chapters[0].start_time, 0);
        assert_eq!(chapters[0].duration, 42_000);
    }

    #[test]
    fn test_generate_chapters_empty_input() {
        assert!(generate_chapters_from_durations(&[]).is_empty());
    }

    #[test]
    fn test_generate_chapters_skips_zero_length_tracks() {
        // A zero-length track cannot produce a chapter; the offset only advances
        // by durations that are kept, so later chapters stay on the merged timeline.
        let chapters = generate_chapters_from_durations(&[0, 3_000, 0, 2_000]);

        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].start_time, 0);
        assert_eq!(chapters[0].duration, 3_000);
        assert_eq!(chapters[1].title, "Chapter 02");
        assert_eq!(chapters[1].start_time, 3_000);
    }
}
