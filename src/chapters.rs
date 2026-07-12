use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};

const MAX_TITLE_LEN: u32 = 10 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start_time: u64,
    pub duration: u64,
}

pub fn read_chapters(path: &Path) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    // Try to read chapters using ffprobe first
    if let Ok(chapters) = read_chapters_ffprobe(path) {
        if !chapters.is_empty() {
            return Ok(chapters);
        }
    }

    // Fall back to parsing the chpl atom directly
    read_chapters_from_atom(path)
}

/// Read chapters using ffprobe
fn read_chapters_ffprobe(path: &Path) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
    let output = Command::new("ffprobe")
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

/// Read chapters by parsing the chpl atom directly
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

fn search_chapters_in_moov(
    file: &mut std::fs::File,
    moov_end: u64,
) -> Result<Vec<Chapter>, Box<dyn std::error::Error>> {
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
        } else if atom_ident == "udta" {
            let udta_end = file.stream_position()? + (atom_len as u64 - 8);
            return search_chapters_in_udta(file, udta_end);
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
            return search_chapters_in_meta(file, meta_end);
        } else if atom_ident == "chpl" {
            return parse_chpl_atom(file, atom_len - 8);
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
            return parse_chpl_atom(file, atom_len - 8);
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

    for i in 0..chapters.len() {
        if i + 1 < chapters.len() {
            let duration = chapters[i + 1].start_time.saturating_sub(chapters[i].start_time);
            chapters[i].duration = duration;
        } else {
            chapters[i].duration = 0;
        }
    }

    Ok(chapters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

        let chapters = read_chapters(&test_file).expect("Failed to read chapters");

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
}
