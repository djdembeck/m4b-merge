use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Path to the m4b-merge binary
fn bin_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_BIN_EXE_m4b-merge"));
    // Fallback for when CARGO_BIN_EXE is not set
    if !path.exists() {
        path = PathBuf::from("target/debug/m4b-merge");
    }
    path
}

/// Check if FFmpeg is available
fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Generate a test MP3 file using FFmpeg
fn generate_test_mp3(path: &Path, duration_secs: u32) {
    let status = Command::new("ffmpeg")
        .args(&[
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=1000:duration={}", duration_secs),
            "-acodec",
            "libmp3lame",
            "-b:a",
            "128k",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-y",
        ])
        .arg(path)
        .status()
        .expect("Failed to run FFmpeg");

    assert!(status.success(), "FFmpeg failed to generate test MP3");
}

/// Generate a test M4A file using FFmpeg
fn generate_test_m4a(path: &Path, duration_secs: u32) {
    let status = Command::new("ffmpeg")
        .args(&[
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=1000:duration={}", duration_secs),
            "-acodec",
            "aac",
            "-b:a",
            "128k",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-y",
        ])
        .arg(path)
        .status()
        .expect("Failed to run FFmpeg");

    assert!(status.success(), "FFmpeg failed to generate test M4A");
}

/// Read chapter titles from an M4B file using ffprobe
fn read_output_chapters(path: &Path) -> Vec<String> {
    let output = Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_chapters"])
        .arg(path)
        .output()
        .expect("Failed to run ffprobe");

    assert!(output.status.success(), "ffprobe failed on {}", path.display());

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("ffprobe output should be JSON");

    json.get("chapters")
        .and_then(|c| c.as_array())
        .map(|chapters| {
            chapters
                .iter()
                .map(|chapter| {
                    chapter
                        .get("tags")
                        .and_then(|t| t.get("title"))
                        .and_then(|t| t.as_str())
                        .unwrap_or_default()
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn test_cli_help() {
    let output = Command::new(bin_path()).arg("--help").output().expect("Failed to run m4b-merge");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("m4b-merge"));
    assert!(stdout.contains("--inputs"));
    assert!(stdout.contains("--output"));
}

#[test]
fn test_cli_version() {
    let output =
        Command::new(bin_path()).arg("--version").output().expect("Failed to run m4b-merge");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), format!("m4b-merge {}", m4b_merge::VERSION));
}

#[test]
fn test_check_ffmpeg() {
    let output =
        Command::new(bin_path()).arg("--check-ffmpeg").output().expect("Failed to run m4b-merge");

    if ffmpeg_available() {
        assert!(output.status.success(), "FFmpeg check should succeed when FFmpeg is available");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("FFmpeg found"));
    } else {
        assert!(!output.status.success(), "FFmpeg check should fail when FFmpeg is not available");
    }
}

#[test]
fn test_cli_no_inputs() {
    let output = Command::new(bin_path()).output().expect("Failed to run m4b-merge");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("No input") || stderr.contains("required"));
}

#[test]
fn test_single_mp3_merge() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let input_dir = temp_dir.path().join("input");
    let output_dir = temp_dir.path().join("output");
    let completed_dir = temp_dir.path().join("completed");

    std::fs::create_dir(&input_dir).unwrap();
    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&completed_dir).unwrap();

    // Generate test MP3
    let input_file = input_dir.join("chapter1.mp3");
    generate_test_mp3(&input_file, 5);

    // Run m4b-merge
    let output = Command::new(bin_path())
        .args(&[
            "-i",
            input_file.to_str().unwrap(),
            "-o",
            output_dir.to_str().unwrap(),
            "--completed-directory",
            completed_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run m4b-merge");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("stdout: {}", stdout);
        eprintln!("stderr: {}", stderr);
    }

    assert!(output.status.success(), "m4b-merge should succeed");

    // Check output file was created
    let output_files: Vec<_> = std::fs::read_dir(&output_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|e| e == "m4b").unwrap_or(false))
        .collect();

    assert!(!output_files.is_empty(), "Output M4B file should be created");
}

#[test]
fn test_single_m4b_merge() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let input_dir = temp_dir.path().join("input");
    let output_dir = temp_dir.path().join("output");
    let completed_dir = temp_dir.path().join("completed");

    std::fs::create_dir(&input_dir).unwrap();
    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&completed_dir).unwrap();

    // Generate a test M4B file (M4B is an M4A with .m4b extension)
    let input_file = input_dir.join("chapter1.m4b");
    let status = Command::new("ffmpeg")
        .args(&[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=5",
            "-acodec",
            "aac",
            "-b:a",
            "128k",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-y",
        ])
        .arg(&input_file)
        .status()
        .expect("Failed to run FFmpeg");
    assert!(status.success(), "FFmpeg should generate test M4B");

    // Run m4b-merge
    let output = Command::new(bin_path())
        .args(&[
            "-i",
            input_file.to_str().unwrap(),
            "-o",
            output_dir.to_str().unwrap(),
            "--completed-directory",
            completed_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run m4b-merge");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("stdout: {}", stdout);
        eprintln!("stderr: {}", stderr);
    }

    assert!(output.status.success(), "m4b-merge should succeed");

    // Check output file was created
    let output_files: Vec<_> = std::fs::read_dir(&output_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|e| e == "m4b").unwrap_or(false))
        .collect();

    assert!(!output_files.is_empty(), "Output M4B file should be created");
}

#[test]
fn test_multiple_mp3_merge() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let input_dir = temp_dir.path().join("input/audiobook");
    let output_dir = temp_dir.path().join("output");
    let completed_dir = temp_dir.path().join("completed");

    std::fs::create_dir_all(&input_dir).unwrap();
    std::fs::create_dir(&output_dir).unwrap();
    std::fs::create_dir(&completed_dir).unwrap();

    // Generate multiple test MP3s
    for i in 1..=3 {
        let input_file = input_dir.join(format!("chapter{:02}.mp3", i));
        generate_test_mp3(&input_file, 3);
    }

    // Run m4b-merge on the directory
    let output = Command::new(bin_path())
        .args(&[
            "-i",
            input_dir.to_str().unwrap(),
            "-o",
            output_dir.to_str().unwrap(),
            "--completed-directory",
            completed_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run m4b-merge");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("stdout: {}", stdout);
        eprintln!("stderr: {}", stderr);
    }

    assert!(output.status.success(), "m4b-merge should succeed");

    // Check output file was created
    let output_files: Vec<_> = std::fs::read_dir(&output_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|e| e == "m4b").unwrap_or(false))
        .collect();

    assert!(!output_files.is_empty(), "Output M4B file should be created");

    // Chapterless inputs get one generated chapter per input track.
    let chapters = read_output_chapters(&output_files[0].path());
    assert_eq!(chapters.len(), 3, "One generated chapter per input track");
    assert_eq!(chapters[0], "Chapter 01");
    assert_eq!(chapters[1], "Chapter 02");
    assert_eq!(chapters[2], "Chapter 03");
}

#[test]
fn test_m4a_copy_merge() {
    if !ffmpeg_available() {
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let input_dir = temp_dir.path().join("input/audiobook");
    let output_dir = temp_dir.path().join("output");

    std::fs::create_dir_all(&input_dir).unwrap();
    std::fs::create_dir(&output_dir).unwrap();

    // Generate test M4A files (should use copy mode)
    for i in 1..=2 {
        let input_file = input_dir.join(format!("track{:02}.m4a", i));
        generate_test_m4a(&input_file, 3);
    }

    // Run m4b-merge
    let output = Command::new(bin_path())
        .args(&["-i", input_dir.to_str().unwrap(), "-o", output_dir.to_str().unwrap()])
        .output()
        .expect("Failed to run m4b-merge");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        eprintln!("stdout: {}", stdout);
        eprintln!("stderr: {}", stderr);
    }

    assert!(output.status.success(), "m4b-merge should succeed");

    // Check output file was created
    let output_files: Vec<_> = std::fs::read_dir(&output_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|e| e == "m4b").unwrap_or(false))
        .collect();

    assert!(!output_files.is_empty(), "Output M4B file should be created");
}

#[test]
fn test_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    let input_dir = temp_dir.path().join("input");
    let output_dir = temp_dir.path().join("output");

    std::fs::create_dir(&input_dir).unwrap();
    std::fs::create_dir(&output_dir).unwrap();

    // Create a dummy file (won't actually be processed in dry-run)
    let input_file = input_dir.join("test.txt");
    std::fs::write(&input_file, "dummy").unwrap();

    // Run m4b-merge in dry-run mode
    let output = Command::new(bin_path())
        .args(&["--dry-run", "-i", input_dir.to_str().unwrap(), "-o", output_dir.to_str().unwrap()])
        .output()
        .expect("Failed to run m4b-merge");

    assert!(output.status.success(), "Dry run should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Dry run"));
    assert!(stdout.contains("Inputs:"));
}
#[test]
fn test_dry_run_with_audio() {
    if !ffmpeg_available() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let input_dir = temp_dir.path().join("input");
    let output_dir = temp_dir.path().join("output");

    std::fs::create_dir(&input_dir).unwrap();
    std::fs::create_dir(&output_dir).unwrap();

    // Generate a real MP3 file
    let mp3_path = input_dir.join("test.mp3");
    generate_test_mp3(&mp3_path, 5);

    // Run m4b-merge in dry-run mode with actual audio
    let output = Command::new(bin_path())
        .args(&["--dry-run", "-i", input_dir.to_str().unwrap(), "-o", output_dir.to_str().unwrap()])
        .output()
        .expect("Failed to run m4b-merge");

    assert!(output.status.success(), "Dry run with audio should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("DRY RUN")
            || stdout.contains("Dry run")
            || stderr.contains("DRY RUN")
            || stderr.contains("Dry run"),
        "Output should mention dry run mode"
    );

    // Verify no output M4B file was created
    let m4b_files = std::fs::read_dir(&output_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".m4b"))
        .count();
    assert_eq!(m4b_files, 0, "Dry run should not create any output files");
}

#[test]
fn test_region_rejects_invalid_value() {
    let input_dir = TempDir::new().unwrap();

    let output = Command::new(bin_path())
        .args([
            "--dry-run",
            "-i",
            input_dir.path().to_str().unwrap(),
            "--metadata-source",
            "audnexus",
            "--region",
            "xx",
        ])
        .output()
        .expect("Failed to run m4b-merge");

    assert!(!output.status.success(), "Invalid --region should be rejected by clap");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value"), "stderr should report the invalid value");
    assert!(stderr.contains("xx"), "stderr should name the rejected value");
}

#[test]
fn test_region_accepted_with_audnexus() {
    let input_dir = TempDir::new().unwrap();
    std::fs::write(input_dir.path().join("test.txt"), "dummy").unwrap();

    let output = Command::new(bin_path())
        .args([
            "--dry-run",
            "-i",
            input_dir.path().to_str().unwrap(),
            "--metadata-source",
            "audnexus",
            "--region",
            "de",
        ])
        .output()
        .expect("Failed to run m4b-merge");

    assert!(output.status.success(), "Valid --region should be accepted end-to-end");
}

/// Single-request loopback Audnexus mock: serves `status_line` + `body` for the
/// first request and returns the request URIs it saw (as sent, path + query).
/// The connection closes after each reply.
fn audnexus_mock(status_line: &str, body: &str) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let status_line = status_line.to_string();
    let body = body.to_string();
    let handle = std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking listener");
        let first_request_deadline = Instant::now() + Duration::from_secs(15);
        // A handled request ends the loop once the stream is quiet for this
        // long (no retry follows for the 4xx the tests serve; any retried
        // request arrives within the client's backoff, under 1s here).
        let quiescent = Duration::from_millis(250);
        let mut uris = Vec::new();
        let mut last_request: Option<Instant> = None;
        'accept: loop {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("listener accept failed: {e}"),
                }
                let now = Instant::now();
                if now >= first_request_deadline {
                    panic!("no request reached the listener");
                }
                if last_request.map(|last| now.duration_since(last) >= quiescent) == Some(true) {
                    break 'accept;
                }
            };
            last_request = Some(Instant::now());
            let mut buf = [0u8; 512];
            let mut request_bytes: Vec<u8> = Vec::new();
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        request_bytes.extend_from_slice(&buf[..n]);
                        if request_bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => panic!("read request: {e}"),
                }
            }
            let requested_line = BufReader::new(&request_bytes[..])
                .lines()
                .next()
                .and_then(|l| l.ok())
                .unwrap_or_default();
            let uri = requested_line.split_whitespace().nth(1).unwrap_or_default().to_string();
            uris.push(uri);
            let _ = stream.write_all(
                format!(
                    "{status_line}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.shutdown(std::net::Shutdown::Write);
        }
        uris
    });
    (format!("http://{addr}"), handle)
}

#[test]
fn test_region_option_reaches_audnexus_request() {
    if !ffmpeg_available() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let input_dir = temp_dir.path().join("input");
    let output_dir = temp_dir.path().join("output");
    std::fs::create_dir_all(&input_dir).unwrap();
    std::fs::create_dir_all(&output_dir).unwrap();
    let input_file = input_dir.join("chapter1.mp3");
    generate_test_mp3(&input_file, 2);

    // The mock 404s the lookup with the audnexus wrong-region body; a 404 is
    // a localized metadata failure (no retry), so the run proceeds without
    // metadata and we can inspect the region-tagged request it made.
    let (base, server) =
        audnexus_mock("HTTP/1.1 404 Not Found", "{\"error\":\"REGION_UNAVAILABLE\"}");

    let output = Command::new(bin_path())
        .args([
            "-i",
            input_file.to_str().unwrap(),
            "-o",
            output_dir.to_str().unwrap(),
            "-a",
            "B08XYZ1234",
            "--metadata-source",
            "audnexus",
            "--api-url",
            &base,
            "--region",
            "de",
        ])
        .output()
        .expect("Failed to run m4b-merge");

    let uris = server.join().expect("mock server join");
    assert!(!uris.is_empty(), "no request reached the audnexus mock");
    assert!(
        uris[0].contains("/books/B08XYZ1234?region=de"),
        "the CLI's --region must ride the audnexus request URL, got: {uris:?}"
    );
    // Tracing (the WARN for the region-unavailable lookup) writes to stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("'de'") && stdout.contains("B08XYZ1234"),
        "the CLI error should name region de and the metadata id, stdout: {stdout}"
    );
}

#[test]
fn test_descriptive_marker_does_not_mask_real_metadata_id() {
    if !ffmpeg_available() {
        return;
    }
    // "Unabridged" is 10 alphanumeric characters, so it is ASIN-shaped. A
    // group named `Book [Unabridged] [B08XYZ1234]` must fall through to the
    // real ASIN instead of looking up the marker and merging without metadata.
    let temp_dir = TempDir::new().unwrap();
    let group_dir = temp_dir.path().join("Book [Unabridged] [B08XYZ1234]");
    std::fs::create_dir_all(&group_dir).unwrap();
    generate_test_mp3(&group_dir.join("chapter1.mp3"), 2);

    // Both candidates 404 (a localized metadata failure, no retry), so the
    // recorded URIs prove the lookup order and the merge still completes.
    let (base, server) = audnexus_mock("HTTP/1.1 404 Not Found", "not found");

    let output = Command::new(bin_path())
        .args([
            "-i",
            group_dir.to_str().unwrap(),
            "-o",
            temp_dir.path().to_str().unwrap(),
            "--metadata-source",
            "audnexus",
            "--api-url",
            &base,
        ])
        .output()
        .expect("Failed to run m4b-merge");

    let uris = server.join().expect("mock server join");
    assert_eq!(
        uris,
        vec!["/books/Unabridged?region=us".to_string(), "/books/B08XYZ1234?region=us".to_string()],
        "the descriptive marker must not mask the real ASIN"
    );
    assert!(
        output.status.success(),
        "a localized metadata failure must not fail the merge, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn test_non_us_region_warns_for_audiobookdb() {
    if !ffmpeg_available() {
        return;
    }
    // The id lives in the group directory name, so discovery extracts it and
    // the lookup actually reaches the mock (a 404 → IdNotFound → localized
    // metadata failure, no retry). The request list is asserted so the test
    // cannot pass while the mock receives nothing.
    let run = |region: &str| {
        let (base, server) = audnexus_mock("HTTP/1.1 404 Not Found", "not found");
        let dir = TempDir::new().unwrap();
        let group_dir = dir.path().join("book [B08XYZ1234]");
        std::fs::create_dir_all(&group_dir).unwrap();
        generate_test_mp3(&group_dir.join("chapter1.mp3"), 2);
        let output = Command::new(bin_path())
            .args([
                "-i",
                group_dir.to_str().unwrap(),
                "-o",
                dir.path().to_str().unwrap(),
                "--api-url",
                &base,
                "--region",
                region,
            ])
            .output()
            .expect("Failed to run m4b-merge");
        let uris = server.join().expect("mock server join");
        assert!(!uris.is_empty(), "no request reached the audnexus mock");
        assert!(
            uris.iter().all(|uri| uri.contains("B08XYZ1234")),
            "every lookup must target the group's metadata id, got: {uris:?}"
        );
        output
    };

    // --region de with the default (AudiobookDB) source: the CLI warns that
    // the region is ignored for AudiobookDB lookups. Tracing writes to stdout.
    let de = run("de");
    let de_stdout = String::from_utf8_lossy(&de.stdout);
    assert!(
        de.status.success(),
        "a localized metadata failure must not fail the merge, stdout: {de_stdout}"
    );
    // Case-insensitive: the assertion targets the warning's content, not the
    // product-name casing.
    let de_stdout_lower = de_stdout.to_lowercase();
    assert!(
        de_stdout_lower.contains("audiobookdb") && de_stdout_lower.contains("region"),
        "the warning should name audiobookdb and region, stdout: {de_stdout}"
    );

    // --region us (the default): no such warning.
    let us = run("us");
    let us_stdout = String::from_utf8_lossy(&us.stdout);
    assert!(
        us.status.success(),
        "a localized metadata failure must not fail the merge, stdout: {us_stdout}"
    );
    assert!(
        !us_stdout.contains("region"),
        "no region warning for the default US region, stdout: {us_stdout}"
    );
}
