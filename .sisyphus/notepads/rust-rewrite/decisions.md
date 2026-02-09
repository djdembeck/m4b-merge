# Architectural Decisions

## FFmpeg Discovery Strategy
**Decision:** Use subprocess calls via `std::process::Command` instead of ffmpeg-next crate

**Rationale:**
- ffmpeg-next requires system FFmpeg development libraries which may not be available
- Subprocess approach matches how m4b-tool (the reference implementation) works
- JSON output from ffprobe is easier to parse than C bindings
- Can easily swap to ffmpeg-next later if needed

## Binary Discovery Order
1. Try `which` command to find in PATH
2. Fall back to common installation directories
3. Return error if not found

## Error Handling Strategy
- Custom error type `FFmpegError` using `thiserror`
- Distinguish between different failure modes for better diagnostics
- Chain errors with `#[from]` where appropriate

## Module Structure
- Created `lib.rs` to expose modules for testing
- Binary (`main.rs`) uses library for actual functionality
- This enables testing of internal modules

## CLI Design
- Added `--check-ffmpeg` flag for diagnostic output
- Shows version, build info, and available libraries
- Exits with code 0 on success, 1 on failure

---

## Task 4: Audio Merging and Conversion

### Decision: Use FFmpeg concat demuxer instead of complex filter graphs
**Rationale:**
- Concat demuxer is designed specifically for file concatenation
- Better quality preservation for copy mode
- Simpler command structure
- Matches original m4b-merge behavior
- Avoids complex filter syntax that is error-prone

**Alternative considered:** Using `-filter_complex concat` filter
- Rejected: More complex, requires encoding even for same-codec files

### Decision: Auto-detect merge mode from file formats
**Rationale:**
- User doesn't need to specify mode manually
- Copy mode for M4A/M4B (no quality loss)
- Transcode mode for MP3 or mixed formats
- Sensible defaults reduce user friction

**Implementation:**
```rust
pub fn from_files(files: &[AudioFile]) -> MergeMode {
    if files.iter().all(|f| matches!(f.format, M4A|M4B)) {
        MergeMode::Copy
    } else {
        MergeMode::Transcode
    }
}
```

### Decision: Use standard bitrate values (64, 96, 128, 192, 256, 320)
**Rationale:**
- AAC encoding has discrete "sweet spots" at these bitrates
- Avoids odd values that might not be well-supported
- Matches typical audiobook bitrates
- Users expect these standard values

### Decision: Implement progress via trait rather than channels
**Rationale:**
- More flexible - users can implement any progress UI
- Simpler API (no channel setup/teardown)
- No async complexity needed
- Easier to test

**Example usage:**
```rust
pub trait ProgressHandler: Send + Sync {
    fn on_progress(&self, progress: MergeProgress);
}
```

### Decision: Use tempfile crate for concat list management
**Rationale:**
- Automatic cleanup on drop
- Cross-platform temp directory handling
- Secure file creation
- No manual path management

**Cleanup strategy:**
1. NamedTempFile provides RAII cleanup
2. Explicit removal after merge (for early failure)
3. Cleaned up on both success and failure paths

### Decision: Parse FFmpeg stderr for progress
**Rationale:**
- FFmpeg outputs progress to stderr (even on success)
- More detailed than exit codes alone
- Provides real-time feedback without pipes
- Industry standard approach

**Pattern:**
```rust
let progress_regex = Regex::new(
    r"time=(\d+:\d+:\d+\.\d+)\s+.*?speed=\s*([\d.]+)x"
)?;
```

### Decision: Builder pattern for MergeJob construction
**Rationale:**
- Many optional parameters (mode, bitrate, threads)
- Better ergonomics than function overloading
- Self-documenting API
- Easy to extend with new options

**API:**
```rust
let job = MergeJobBuilder::new()
    .input_files(files)
    .output("/path/to/output.m4b")
    .bitrate(192)
    .threads(4)
    .build()?;
```

### Decision: Arc<FFmpeg> for shared ownership
**Rationale:**
- Merger may be used across multiple jobs
- FFmpeg instance is immutable after creation
- Thread-safe for parallel processing
- Avoids cloning the FFmpeg struct
