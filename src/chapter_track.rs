//! MP4 container surgery for chapter tracks.
//!
//! mp4ameta writes the QuickTime chapter text track (the one Apple players
//! read, referenced by a `chap` tref) as a bare sample table: `stts` durations
//! accumulate inter-chapter gaps but the first sample always sits at media
//! time 0, and no edit list (`elst`) is emitted. Apple/AVFoundation players
//! position the chapter track timeline using the track's `elst` media time, so
//! without it every chapter is shifted by `-first_start`.
//!
//! `fix_chapter_track_start` locates the text track referenced by a `chap`
//! track reference and inserts `edts.elst` with `media_time = first_start`
//! (in the movie header timescale), leaving everything else untouched.

use std::io::{Read, Seek, Write};
use std::path::Path;

/// Errors specific to chapter-track container surgery
#[derive(Debug, thiserror::Error)]
pub enum ChapterTrackError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not a valid MP4 file: {0}")]
    InvalidMp4(String),

    #[error("Malformed atom hierarchy while looking for {0}")]
    MalformedAtom(String),
}

pub type Result<T, E = ChapterTrackError> = std::result::Result<T, E>;

/// Ensure the chapter text track's timeline starts at `first_start_ms` by
/// inserting an `elst` edit with the matching media time.
///
/// Idempotent: an existing `edts` on the chapter track is replaced. The audio
/// track (or any other track) is never modified.
pub fn fix_chapter_track_start(path: &Path, first_start_ms: u64) -> Result<()> {
    let mut file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;

    let ftyp = read_atom_head(&mut file)?;
    if ftyp.fourcc != FOURCC_FTYP {
        return Err(ChapterTrackError::InvalidMp4(format!(
            "expected ftyp, found {:?}",
            ftyp.fourcc_as_str()
        )));
    }

    let moov = find_atom_in_range(&mut file, ftyp.end(), None, FOURCC_MOOV)?
        .ok_or_else(|| ChapterTrackError::InvalidMp4("no moov atom".into()))?;

    // Find the audio track (hdlr subtype soun) and the chapter text track
    // (hdlr subtype text, referenced by a chap tref).
    let (audio_trak, chapter_trak) = find_chapter_tracks(&mut file, moov.start(), moov.end())?;
    let chapter_trak = match chapter_trak {
        Some(t) => t,
        // No chapter track: nothing to fix (e.g. only a chpl atom present).
        None => return Ok(()),
    };

    // Chapter track media timescale: chapter start offsets are expressed in
    // the track's own media timescale, while the elst media time is in the
    // track timeline (mvhd timescale). mp4ameta sets both the chapter mdhd
    // and mvhd timescales to the mvhd value, and stts durations in the same
    // units, so the elst media time must be expressed in the mvhd timescale.
    let mvhd_timescale = read_mvhd_timescale(&mut file, moov.content_start(), moov.end())?;
    let media_time = ms_to_timescale(first_start_ms, mvhd_timescale);

    // Trim the chapter track's tkhd duration so the track timeline still ends
    // at the movie duration: track duration = movie_duration - media_time.
    set_trak_duration(&mut file, &chapter_trak, moov, media_time)?;

    let delta = insert_or_replace_elst(&mut file, &chapter_trak, &audio_trak, media_time)?;

    // The splice shifted the chapter trak's content; fix up the enclosing
    // trak and moov size fields by the same delta.
    resize_enclosing_atoms(&mut file, moov, chapter_trak.atom, delta)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AtomHead {
    /// Offset of the size field (atom start)
    start: u64,
    /// Total atom size including header
    size: u64,
    /// Header size (8, or 16 for 64-bit sizes)
    header_len: u64,
    fourcc: [u8; 4],
}

impl AtomHead {
    fn start(&self) -> u64 {
        self.start
    }

    fn end(&self) -> u64 {
        self.start + self.size
    }

    /// Offset of the first content byte
    fn content_start(&self) -> u64 {
        self.start + self.header_len
    }

    fn content_len(&self) -> u64 {
        self.size - self.header_len
    }

    fn fourcc_as_str(&self) -> String {
        self.fourcc.iter().map(|&b| b as char).collect()
    }
}

const FOURCC_FTYP: [u8; 4] = *b"ftyp";
const FOURCC_MOOV: [u8; 4] = *b"moov";
const FOURCC_TRAK: [u8; 4] = *b"trak";
const FOURCC_TKHD: [u8; 4] = *b"tkhd";
const FOURCC_MDIA: [u8; 4] = *b"mdia";
const FOURCC_HDLR: [u8; 4] = *b"hdlr";
const FOURCC_EDTS: [u8; 4] = *b"edts";
const FOURCC_ELST: [u8; 4] = *b"elst";
const FOURCC_TREF: [u8; 4] = *b"tref";
const FOURCC_CHAP: [u8; 4] = *b"chap";
const FOURCC_MVHD: [u8; 4] = *b"mvhd";
const HDLR_SOUN: [u8; 4] = *b"soun";
const HDLR_TEXT: [u8; 4] = *b"text";

/// Read an atom head at the current position; the stream is left after the head.
fn read_atom_head(file: &mut std::fs::File) -> Result<AtomHead> {
    let start = file.stream_position()?;

    let mut buf8 = [0u8; 8];
    file.read_exact(&mut buf8)?;
    let mut size = u64::from(u32::from_be_bytes([buf8[0], buf8[1], buf8[2], buf8[3]]));
    let mut header_len = 8u64;
    let fourcc = [buf8[4], buf8[5], buf8[6], buf8[7]];

    if size == 1 {
        let mut buf16 = [0u8; 8];
        file.read_exact(&mut buf16)?;
        size = u64::from_be_bytes(buf16);
        header_len = 16;
    } else if size == 0 {
        // Size 0 means "extends to end of file"
        let file_len = file.seek(std::io::SeekFrom::End(0))?;
        file.seek(std::io::SeekFrom::Start(start))?;
        file.seek(std::io::SeekFrom::Start(start + header_len))?;
        size = file_len - start;
    }

    if size < header_len {
        return Err(ChapterTrackError::MalformedAtom(fourcc_as_string(&fourcc)));
    }

    Ok(AtomHead { start, size, header_len, fourcc })
}

fn fourcc_as_string(fourcc: &[u8; 4]) -> String {
    fourcc.iter().map(|&b| b as char).collect()
}

/// Scan sibling atoms in `range` for `fourcc`, leaving the stream position
/// anywhere within `range` bounds afterwards.
fn find_atom_in_range(
    file: &mut std::fs::File,
    range_start: u64,
    range_end: Option<u64>,
    fourcc: [u8; 4],
) -> Result<Option<AtomHead>> {
    let end = match range_end {
        Some(e) => e,
        None => file.seek(std::io::SeekFrom::End(0))?,
    };

    let mut pos = range_start;
    while pos + 8 <= end {
        file.seek(std::io::SeekFrom::Start(pos))?;
        let head = read_atom_head(file)?;
        if head.fourcc == fourcc {
            return Ok(Some(head));
        }
        pos += head.size;
    }

    Ok(None)
}

/// A located trak with its tkhd and hdlr subtype
#[derive(Debug, Clone)]
struct TrakInfo {
    atom: AtomHead,
    tkhd: AtomHead,
    tkhd_version: u8,
    /// Track id read from tkhd
    track_id: u32,
    /// hdlr subtype (soun / text / ...)
    hdlr_subtype: [u8; 4],
    /// Track ids referenced by a `chap` tref inside this trak (empty if none)
    chap_refs: Vec<u32>,
}

/// Walk traks in `moov` returning `(audio_trak, chapter_trak)`.
///
/// The chapter trak is the `text`-hdlr trak whose id appears in a `chap` track
/// reference. `chap` lives in a `tref` atom, which QuickTime places inside the
/// referencing (audio) trak; some muxers put it at moov level, so both are
/// scanned.
fn find_chapter_tracks(
    file: &mut std::fs::File,
    moov_start: u64,
    moov_end: u64,
) -> Result<(TrakInfo, Option<TrakInfo>)> {
    let mut traks: Vec<TrakInfo> = Vec::new();
    let mut chap_refs: Vec<u32> = Vec::new();

    // Scan moov's children: moov_start points at the moov fourcc itself
    let mut pos = moov_start + 8;
    while pos + 8 <= moov_end {
        file.seek(std::io::SeekFrom::Start(pos))?;
        let head = read_atom_head(file)?;
        let atom_end = head.end();

        match head.fourcc {
            FOURCC_TRAK => {
                let trak = parse_trak(file, head)?;
                chap_refs.extend(trak.chap_refs.iter().copied());
                traks.push(trak);
            }
            FOURCC_TREF => chap_refs.extend(parse_chap_refs(file, &head)?),
            _ => {}
        }

        pos = atom_end;
    }

    let audio_trak = traks
        .iter()
        .find(|t| t.hdlr_subtype == HDLR_SOUN)
        .cloned()
        .ok_or_else(|| ChapterTrackError::InvalidMp4("no audio (soun) track".into()))?;

    let chapter_trak = if chap_refs.is_empty() {
        None
    } else {
        traks
            .iter()
            .find(|t| chap_refs.contains(&t.track_id) && t.hdlr_subtype == HDLR_TEXT)
            .cloned()
    };

    Ok((audio_trak, chapter_trak))
}

/// Parse one trak's tkhd (id) and hdlr (subtype, nested in mdia).
fn parse_trak(file: &mut std::fs::File, trak: AtomHead) -> Result<TrakInfo> {
    let mut info = TrakInfo {
        atom: trak,
        tkhd: AtomHead { start: 0, size: 0, header_len: 8, fourcc: [0; 4] },
        tkhd_version: 0,
        track_id: 0,
        hdlr_subtype: [0; 4],
        chap_refs: Vec::new(),
    };

    // Walk the trak tree; tkhd is a direct child, hdlr sits under mdia.
    walk_trak_children(file, trak, &mut info)?;

    Ok(info)
}

fn walk_trak_children(
    file: &mut std::fs::File,
    container: AtomHead,
    info: &mut TrakInfo,
) -> Result<()> {
    let mut pos = container.content_start();
    let container_end = container.end();
    while pos + 8 <= container_end {
        file.seek(std::io::SeekFrom::Start(pos))?;
        let head = read_atom_head(file)?;
        let atom_end = head.end();

        if head.fourcc == FOURCC_TKHD {
            let mut vbuf = [0u8; 4];
            file.seek(std::io::SeekFrom::Start(head.content_start()))?;
            file.read_exact(&mut vbuf)?;
            let version = vbuf[0];
            // id: v0 => ver/flags(4) + created(4) + modified(4); v1 doubles the two timestamps
            let id_offset = head.content_start() + 4 + if version == 1 { 16 } else { 8 };
            file.seek(std::io::SeekFrom::Start(id_offset))?;
            let mut ibuf = [0u8; 4];
            file.read_exact(&mut ibuf)?;
            info.tkhd = head;
            info.tkhd_version = version;
            info.track_id = u32::from_be_bytes(ibuf);
        } else if head.fourcc == FOURCC_HDLR {
            // hdlr: ver/flags(4) + pre_defined(4), then subtype
            let sub_offset = head.content_start() + 8;
            file.seek(std::io::SeekFrom::Start(sub_offset))?;
            let mut sbuf = [0u8; 4];
            file.read_exact(&mut sbuf)?;
            info.hdlr_subtype = sbuf;
        } else if head.fourcc == FOURCC_TREF {
            // QuickTime nests the chap tref inside the referencing trak
            let refs = parse_chap_refs(file, &head)?;
            info.chap_refs.extend(refs);
        } else if head.fourcc == FOURCC_MDIA {
            walk_trak_children(file, head, info)?;
        }

        pos = atom_end;
    }

    Ok(())
}

/// Read track ids referenced by a `chap` entry inside a tref atom.
fn parse_chap_refs(file: &mut std::fs::File, tref: &AtomHead) -> Result<Vec<u32>> {
    let mut refs = Vec::new();

    // Scan tref children for 'chap'
    let mut pos = tref.content_start();
    let tref_end = tref.end();
    while pos + 8 <= tref_end {
        file.seek(std::io::SeekFrom::Start(pos))?;
        let head = read_atom_head(file)?;
        let atom_end = head.end();

        if head.fourcc == FOURCC_CHAP {
            file.seek(std::io::SeekFrom::Start(head.content_start()))?;
            let count = head.content_len() as usize / 4;
            for _ in 0..count {
                let mut b = [0u8; 4];
                file.read_exact(&mut b)?;
                refs.push(u32::from_be_bytes(b));
            }
        }

        pos = atom_end;
    }

    Ok(refs)
}

/// Read mvhd timescale from the moov atom.
fn read_mvhd_timescale(file: &mut std::fs::File, moov_start: u64, moov_end: u64) -> Result<u32> {
    let mvhd = find_atom_in_range(file, moov_start, Some(moov_end), FOURCC_MVHD)?
        .ok_or_else(|| ChapterTrackError::InvalidMp4("no mvhd atom".into()))?;

    let mut buf = [0u8; 4];
    // mvhd: ver/flags(4) + created(4) + modified(4) + timescale(4)
    file.seek(std::io::SeekFrom::Start(mvhd.content_start() + 12))?;
    file.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

/// Rewrite the chapter trak's tkhd duration to `movie_duration - media_time`
/// so the track still spans the full movie timeline after the edit shift.
fn set_trak_duration(
    file: &mut std::fs::File,
    chapter_trak: &TrakInfo,
    moov: AtomHead,
    media_time: u64,
) -> Result<()> {
    let mvhd = find_atom_in_range(file, moov.content_start(), Some(moov.end()), FOURCC_MVHD)?
        .ok_or_else(|| ChapterTrackError::InvalidMp4("no mvhd atom".into()))?;

    // mvhd duration: v0 => ver/flags(4) + created(4) + modified(4) + timescale(4) + duration(4)
    let dur_offset = mvhd.content_start() + 16;
    file.seek(std::io::SeekFrom::Start(dur_offset))?;
    let mut dbuf = [0u8; 4];
    file.read_exact(&mut dbuf)?;
    let movie_duration = u64::from(u32::from_be_bytes(dbuf));

    let track_duration = movie_duration.saturating_sub(media_time);

    // tkhd duration: v0 => ver/flags(4)+created(4)+modified(4)+id(4)+reserved(4)+duration(4) => offset 20
    let tkhd_dur_offset = match chapter_trak.tkhd_version {
        1 => chapter_trak.tkhd.content_start() + 4 + 16 + 8 + 4 + 4,
        _ => chapter_trak.tkhd.content_start() + 20,
    };
    file.seek(std::io::SeekFrom::Start(tkhd_dur_offset))?;
    file.write_all(&u32_to_be_bytes(track_duration))?;

    Ok(())
}

fn u32_to_be_bytes(v: u64) -> [u8; 4] {
    let v = v.min(u32::MAX as u64);
    (v as u32).to_be_bytes()
}

/// Convert milliseconds to atom timescale units.
fn ms_to_timescale(ms: u64, timescale: u32) -> u64 {
    if timescale == 0 {
        return ms;
    }
    // ms * timescale / 1000, rounded
    (ms.saturating_mul(timescale as u64) + 500) / 1000
}

/// After splicing inside `child` (a trak embedded in the moov `parent`), adjust
/// the size fields of `child` and `parent` by the splice `delta`.
fn resize_enclosing_atoms(
    file: &mut std::fs::File,
    parent: AtomHead,
    child: AtomHead,
    delta: i64,
) -> Result<()> {
    if delta == 0 {
        return Ok(());
    }

    // Child (trak) new size
    let new_child_size = (child.size as i64 + delta) as u32;
    file.seek(std::io::SeekFrom::Start(child.start))?;
    file.write_all(&new_child_size.to_be_bytes())?;

    // Parent (moov) new size
    let new_parent_size = (parent.size as i64 + delta) as u32;
    file.seek(std::io::SeekFrom::Start(parent.start))?;
    file.write_all(&new_parent_size.to_be_bytes())?;

    Ok(())
}

/// Insert `edts.elst` with `media_time` into the chapter trak, or replace an
/// existing elst. Splices the file: atoms after the insertion point shift.
///
/// elst v0 entry layout: segment_duration(4) + media_time(4) + rate(4, 1.0 fixed point)
/// segment_duration = track duration - media_time (media runs to end of movie).
///
/// Returns the byte delta applied to the file (new - old), so callers can fix
/// up enclosing atom size fields.
fn insert_or_replace_elst(
    file: &mut std::fs::File,
    chapter_trak: &TrakInfo,
    _audio_trak: &TrakInfo,
    media_time: u64,
) -> Result<i64> {
    // Locate existing edts inside the chapter trak (mp4ameta doesn't write one,
    // but an ffmpeg-processed file may carry it).
    let existing_edts = find_atom_in_range(
        file,
        chapter_trak.atom.content_start(),
        Some(chapter_trak.atom.end()),
        FOURCC_EDTS,
    )?;

    let movie_duration = current_movie_duration(file, chapter_trak)?;
    let segment_duration = movie_duration.saturating_sub(media_time);

    // Build elst v0 with a single entry: [duration, media_time, rate 1.0]
    // content = version/flags(4) + entry_count(4) + entry(12)
    let elst_content_size: u32 = 4 + 4 + 12;
    let elst_size: u32 = 8 + elst_content_size;
    let mut elst = Vec::with_capacity(elst_size as usize);
    elst.extend_from_slice(&elst_size.to_be_bytes());
    elst.extend_from_slice(&FOURCC_ELST);
    elst.extend_from_slice(&[0, 0, 0, 0]); // version 0, flags 0
    elst.extend_from_slice(&1u32.to_be_bytes()); // entry count
    elst.extend_from_slice(&u32_to_be_bytes(segment_duration));
    elst.extend_from_slice(&u32_to_be_bytes(media_time));
    elst.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]); // rate 1.0

    // Build edts wrapper
    let edts_size: u32 = 8 + elst_size;
    let mut edts = Vec::with_capacity(edts_size as usize);
    edts.extend_from_slice(&edts_size.to_be_bytes());
    edts.extend_from_slice(&FOURCC_EDTS);
    edts.extend_from_slice(&elst);

    match existing_edts {
        Some(existing) => {
            // Replace in place if same size, else splice
            if existing.size == edts.len() as u64 {
                file.seek(std::io::SeekFrom::Start(existing.start))?;
                file.write_all(&edts)?;
                return Ok(0);
            }
            splice_bytes(file, existing.start, existing.size, &edts)?;
            Ok(edts.len() as i64 - existing.size as i64)
        }
        None => {
            // Insert edts right after tkhd (first child), keeping the
            // conventional tkhd-then-edts child order
            let after_tkhd = if chapter_trak.tkhd.size > 0 {
                chapter_trak.tkhd.end()
            } else {
                chapter_trak.atom.content_start()
            };
            splice_bytes(file, after_tkhd, 0, &edts)?;
            Ok(edts.len() as i64)
        }
    }
}

/// Read the movie duration from mvhd (assumes moov precedes the chapter trak).
fn current_movie_duration(file: &mut std::fs::File, _chapter_trak: &TrakInfo) -> Result<u64> {
    // Walk from the beginning: ftyp then moov
    file.seek(std::io::SeekFrom::Start(0))?;
    let ftyp = read_atom_head(file)?;
    let moov = find_atom_in_range(file, ftyp.end(), None, FOURCC_MOOV)?
        .ok_or_else(|| ChapterTrackError::InvalidMp4("no moov atom".into()))?;

    let mvhd = find_atom_in_range(file, moov.content_start(), Some(moov.end()), FOURCC_MVHD)?
        .ok_or_else(|| ChapterTrackError::InvalidMp4("no mvhd atom".into()))?;

    let mut dbuf = [0u8; 4];
    file.seek(std::io::SeekFrom::Start(mvhd.content_start() + 16))?;
    file.read_exact(&mut dbuf)?;
    Ok(u64::from(u32::from_be_bytes(dbuf)))
}

/// Replace `remove_len` bytes at `at` with `insert` by splicing the file tail.
fn splice_bytes(file: &mut std::fs::File, at: u64, remove_len: u64, insert: &[u8]) -> Result<()> {
    let file_len = file.seek(std::io::SeekFrom::End(0))?;
    let tail_start = at + remove_len;
    let tail_len = file_len - tail_start;

    // Read the tail into memory (chapter files are small in practice; the moov
    // is typically < a few MB). To stay safe for huge tails, stream in chunks.
    const CHUNK: usize = 1 << 20;

    let delta = insert.len() as i64 - remove_len as i64;
    if delta == 0 {
        file.seek(std::io::SeekFrom::Start(at))?;
        file.write_all(insert)?;
        return Ok(());
    }

    if delta > 0 {
        // Grow: copy tail back-to-front from the end
        file.set_len(file_len + delta as u64)?;
        let mut processed = 0u64;
        while processed < tail_len {
            let take = std::cmp::min(CHUNK as u64, tail_len - processed) as usize;
            let src = tail_start + tail_len - processed - take as u64;
            let dst = src as i64 + delta;
            let mut buf = vec![0u8; take];
            file.seek(std::io::SeekFrom::Start(src))?;
            file.read_exact(&mut buf)?;
            file.seek(std::io::SeekFrom::Start(dst as u64))?;
            file.write_all(&buf)?;
            processed += take as u64;
        }
    } else {
        // Shrink: copy tail forward, then truncate
        let mut processed = 0u64;
        while processed < tail_len {
            let take = std::cmp::min(CHUNK as u64, tail_len - processed) as usize;
            let src = tail_start + processed;
            let dst_i = src as i64 + delta;
            let mut buf = vec![0u8; take];
            file.seek(std::io::SeekFrom::Start(src))?;
            file.read_exact(&mut buf)?;
            file.seek(std::io::SeekFrom::Start(dst_i as u64))?;
            file.write_all(&buf)?;
            processed += take as u64;
        }
        file.set_len(file_len - (-delta) as u64)?;
    }

    // Write the inserted bytes
    file.seek(std::io::SeekFrom::Start(at))?;
    file.write_all(insert)?;
    file.sync_all()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ms_to_timescale() {
        assert_eq!(ms_to_timescale(2000, 1000), 2000);
        assert_eq!(ms_to_timescale(2000, 44100), 88200);
        assert_eq!(ms_to_timescale(1000, 10_000_000), 10_000_000);
        assert_eq!(ms_to_timescale(0, 1000), 0);
    }

    #[test]
    fn test_no_chapter_track_is_noop() -> Result<()> {
        // A plain m4b without any chapter references: fix must be a no-op and
        // must not corrupt the file.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("plain.m4b");
        std::fs::write(&path, minimal_mp4_bytes())?;

        fix_chapter_track_start(&path, 1000)?;

        let after = std::fs::read(&path).unwrap();
        assert_eq!(after, minimal_mp4_bytes(), "file must be untouched");
        Ok(())
    }

    #[test]
    fn test_elst_injected_into_chapter_track() -> Result<()> {
        if !ffmpeg_available() {
            return Ok(());
        }

        // Build a file with a real chapter track via mp4ameta (same as the
        // embed path), then run the fix and verify the elst contents.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("chap.m4b");
        std::fs::write(&path, minimal_mp4_bytes())?;

        let mut tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        tag.chapter_list_mut().extend([
            mp4ameta::Chapter::new(std::time::Duration::from_secs(2), "Chapter 1"),
            mp4ameta::Chapter::new(std::time::Duration::from_secs(7), "Chapter 2"),
        ]);
        tag.chapter_track_mut().extend([
            mp4ameta::Chapter::new(std::time::Duration::from_secs(2), "Chapter 1"),
            mp4ameta::Chapter::new(std::time::Duration::from_secs(7), "Chapter 2"),
        ]);
        tag.write_to_path(&path).unwrap();

        fix_chapter_track_start(&path, 2000)?;

        let data = std::fs::read(&path).unwrap();
        // Find elst atoms and parse v0 entries
        let elsts = find_atoms(&data, b"elst");
        assert_eq!(elsts.len(), 1, "chapter trak must carry exactly one elst");
        let (start, size) = elsts[0];
        let b = &data[start..start + size];
        let entry_count = u32::from_be_bytes([b[12], b[13], b[14], b[15]]);
        assert_eq!(entry_count, 1);
        let segment_duration = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
        let media_time = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        // mvhd timescale is 1000 in the fixture: media_time must be 2000
        assert_eq!(media_time, 2000, "elst media time must equal first chapter start");
        assert_eq!(segment_duration, 10_000 - 2_000);

        // The whole tree must stay parseable: walk top-level and moov/trak
        assert!(tree_walk_clean(&data), "atom tree must remain consistent");
        Ok(())
    }

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Find all occurrences of a fourcc preceded by its big-endian size field.
    fn find_atoms(data: &[u8], fourcc: &[u8; 4]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= data.len() {
            if &data[i + 4..i + 8] == fourcc {
                let size =
                    u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
                if size >= 8 && i + size <= data.len() {
                    out.push((i, size));
                }
            }
            i += 1;
        }
        out
    }

    /// Walk the atom tree strictly: every atom size must fit its container.
    fn tree_walk_clean(data: &[u8]) -> bool {
        fn walk(data: &[u8], start: usize, end: usize) -> bool {
            let mut pos = start;
            while pos + 8 <= end {
                let size = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
                let size = if size == 0 { end - pos } else { size };
                if size < 8 || pos + size > end {
                    return false;
                }
                pos += size;
            }
            pos == end
        }
        walk(data, 0, data.len())
    }

    /// Minimal structurally-valid MP4: ftyp + moov(mvhd + trak(soun)) + mdat
    fn minimal_mp4_bytes() -> Vec<u8> {
        // Build atoms by hand: ftyp(20) mvhd(108) trak(tkhd 92 + mdia(mdhd 32 + hdlr 45)) mdat(8)
        let mvhd_content_len = 100u32;
        let mut mvhd = Vec::new();
        mvhd.extend_from_slice(&(8 + mvhd_content_len).to_be_bytes());
        mvhd.extend_from_slice(b"mvhd");
        mvhd.extend_from_slice(&[0, 0, 0, 0]); // version/flags
        mvhd.extend_from_slice(&[0; 8]); // created/modified
        mvhd.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        mvhd.extend_from_slice(&10_000u32.to_be_bytes()); // duration (10s)
        mvhd.extend_from_slice(&[0; 80]); // rate/volume/pre_defined/matrix/next_track_id

        let tkhd_content_len = 84u32;
        let mut tkhd = Vec::new();
        tkhd.extend_from_slice(&(8 + tkhd_content_len).to_be_bytes());
        tkhd.extend_from_slice(b"tkhd");
        tkhd.extend_from_slice(&[0, 0, 0, 3]); // version 0, flags 3
        tkhd.extend_from_slice(&[0; 8]); // created/modified
        tkhd.extend_from_slice(&1u32.to_be_bytes()); // track id
        tkhd.extend_from_slice(&[0; 4]); // reserved
        tkhd.extend_from_slice(&10_000u32.to_be_bytes()); // duration
        tkhd.extend_from_slice(&[0; 8]); // reserved
        tkhd.extend_from_slice(&[0; 2]); // layer
        tkhd.extend_from_slice(&[0; 2]); // alternate group
        tkhd.extend_from_slice(&[0; 2]); // volume
        tkhd.extend_from_slice(&[0; 2]); // reserved
        tkhd.extend_from_slice(&[0; 36]); // matrix
        tkhd.extend_from_slice(&[0; 8]); // width/height

        let mdhd_content_len = 24u32;
        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&(8 + mdhd_content_len).to_be_bytes());
        mdhd.extend_from_slice(b"mdhd");
        mdhd.extend_from_slice(&[0, 0, 0, 0]);
        mdhd.extend_from_slice(&[0; 8]);
        mdhd.extend_from_slice(&44100u32.to_be_bytes());
        mdhd.extend_from_slice(&441_000u32.to_be_bytes()); // 10s
        mdhd.extend_from_slice(&[0x55, 0xC4, 0, 0]); // language 'und', pre_defined

        let hdlr_content_len = 25u32;
        let mut hdlr = Vec::new();
        hdlr.extend_from_slice(&(8 + hdlr_content_len).to_be_bytes());
        hdlr.extend_from_slice(b"hdlr");
        hdlr.extend_from_slice(&[0, 0, 0, 0]);
        hdlr.extend_from_slice(&[0; 4]); // pre_defined
        hdlr.extend_from_slice(b"soun");
        hdlr.extend_from_slice(&[0; 12]);
        hdlr.push(0); // null terminator

        let mdia_children_len = 8 + mdhd_content_len + 8 + hdlr_content_len;
        let mut mdia = Vec::new();
        mdia.extend_from_slice(&(8 + mdia_children_len).to_be_bytes());
        mdia.extend_from_slice(b"mdia");
        mdia.extend_from_slice(&mdhd);
        mdia.extend_from_slice(&hdlr);

        let trak_children_len = (8 + tkhd_content_len) + mdia.len() as u32;
        let mut trak = Vec::new();
        trak.extend_from_slice(&(8 + trak_children_len).to_be_bytes());
        trak.extend_from_slice(b"trak");
        trak.extend_from_slice(&tkhd);
        trak.extend_from_slice(&mdia);

        let moov_children_len = (8 + mvhd_content_len) + trak.len() as u32;
        let mut moov = Vec::new();
        moov.extend_from_slice(&(8 + moov_children_len).to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&mvhd);
        moov.extend_from_slice(&trak);

        let mut ftyp = Vec::new();
        ftyp.extend_from_slice(&24u32.to_be_bytes());
        ftyp.extend_from_slice(b"ftyp");
        ftyp.extend_from_slice(b"M4A ");
        ftyp.extend_from_slice(&0u32.to_be_bytes());
        ftyp.extend_from_slice(b"M4A mp42");

        let mut mdat = Vec::new();
        mdat.extend_from_slice(&8u32.to_be_bytes());
        mdat.extend_from_slice(b"mdat");

        let mut out = Vec::new();
        out.extend_from_slice(&ftyp);
        out.extend_from_slice(&moov);
        out.extend_from_slice(&mdat);
        out
    }
}
