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
fn generate_test_mp3(path: &PathBuf, duration_secs: u32) {
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
fn generate_test_m4a(path: &PathBuf, duration_secs: u32) {
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
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
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
#[ignore = "requires FFmpeg"]
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
            "--completed_directory",
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
#[ignore = "requires FFmpeg"]
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
            "--completed_directory",
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
#[ignore = "requires FFmpeg"]
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
