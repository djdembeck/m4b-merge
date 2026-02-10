# Fix Rust Critical Bugs

## TL;DR

> Fix the two critical bugs preventing Rust from achieving feature parity:
> 1. MP3 cover art handling (FFmpeg h264 transcoding bug)
> 2. API network hangs from Docker containers
> 
> **Estimated Effort**: Medium (2-4 hours)
> **Priority**: CRITICAL (blocking MP3 processing and metadata fetch)

---

## Context

From the feature parity comparison, two critical bugs were discovered:

### Bug 1: Cover Art Handling
- **Issue**: Rust attempts to transcode mjpeg cover art to h264 video
- **Error**: `Could not write header (incorrect codec parameters)` 
- **Root Cause**: M4B containers don't support h264 video streams
- **Impact**: Cannot process ANY MP3 files with embedded cover art

### Bug 2: API Network Hang
- **Issue**: API calls to Audnexus hang indefinitely from Docker containers
- **Symptom**: Stuck at "Fetching metadata for ASIN: ..."
- **Works**: API works fine from host machine
- **Impact**: Cannot fetch metadata (chapters, tags, cover art)

---

## Work Objectives

### Core Objective
Fix both critical bugs so Rust can:
1. Successfully process MP3 files with embedded cover art
2. Fetch metadata from Audnexus API within Docker containers

### Concrete Deliverables
- Fixed FFmpeg command for cover art handling
- Fixed API client with proper timeouts and error handling
- Successfully process test MP3 file
- Successfully fetch metadata with ASIN

---

## TODOs

- [ ] 1. Explore Rust Codebase Structure
  
  **What to do**:
  - Find FFmpeg command generation code
  - Find API client implementation
  - Understand current cover art handling
  - Understand current API request flow

  **Files to check**:
  - src/audio/ffmpeg.rs - FFmpeg command building
  - src/api/ - API client code
  - src/merge.rs - Merge logic
  - src/processor.rs - Processing orchestration

  **Acceptance Criteria**:
  - [ ] Located FFmpeg command building code
  - [ ] Located API client code
  - [ ] Documented current cover art flow
  - [ ] Documented current API flow

  **Commit**: NO (exploration only)

---

- [ ] 2. Fix Cover Art Handling in FFmpeg

  **What to do**:
  - Modify FFmpeg command to handle mjpeg cover art correctly
  - Extract cover art as still image (not video stream)
  - Embed cover art in M4B metadata (mp4 container compatible)
  - Test with the Star Wars MP3 file

  **Technical Details**:
  - Current: Attempts `-c:v h264` transcoding
  - Fix: Use `-c copy` for video (mjpeg) or extract separately
  - M4B uses mjpeg for cover art, not h264

  **Acceptance Criteria**:
  - [ ] FFmpeg command modified
  - [ ] MP3 with cover art processes successfully
  - [ ] Output M4B has embedded cover art
  - [ ] File plays correctly

  **Commit**: YES - "fix(ffmpeg): correct cover art handling for MP3 to M4B conversion"

---

- [ ] 3. Fix API Network/Timeout Issues

  **What to do**:
  - Add timeout to API client requests
  - Add proper error handling for network failures
  - Check if it's DNS resolution or request timeout
  - Test API calls from within Docker container

  **Technical Details**:
  - Current: reqwest client likely has no timeout set
  - Fix: Add `.timeout(Duration::from_secs(30))` to requests
  - Add retry logic with exponential backoff
  - Log network errors clearly

  **Acceptance Criteria**:
  - [ ] API client has timeout configured
  - [ ] API calls from Docker container succeed
  - [ ] Metadata is fetched and applied
  - [ ] Clear error messages on failure

  **Commit**: YES - "fix(api): add timeouts and error handling for network requests"

---

- [ ] 4. Test Fix - MP3 Processing

  **What to do**:
  - Rebuild Rust Docker image
  - Run test with Star Wars MP3 file
  - Verify output is created successfully
  - Verify cover art is preserved

  **Command**:
  ```bash
  docker build -t m4b-merge-rust:test -f Dockerfile .
  docker run --rm \
    -v "/home/djdembeck/input/M Chen - Star Wars Brotherhood.MP3":/input \
    -v /home/djdembeck/projects/github/m4b-merge/test-outputs/rust:/output \
    m4b-merge-rust:test \
    -i /input -o /output
  ```

  **Acceptance Criteria**:
  - [ ] Container runs without FFmpeg errors
  - [ ] Output M4B file created
  - [ ] Cover art present in output
  - [ ] Audio plays correctly

  **Commit**: NO (testing)

---

- [ ] 5. Test Fix - API Metadata Fetch

  **What to do**:
  - Run test with ASIN flag
  - Verify API call succeeds
  - Verify metadata is applied to output
  - Check chapters, tags, cover art from API

  **Command**:
  ```bash
  docker run --rm \
    -v "/home/djdembeck/input/M Chen - Star Wars Brotherhood.MP3":/input \
    -v /home/djdembeck/projects/github/m4b-merge/test-outputs/rust:/output \
    m4b-merge-rust:test \
    -i /input -o /output --asin B09HR33QHH
  ```

  **Acceptance Criteria**:
  - [ ] API call completes without hanging
  - [ ] Metadata fetched from Audnexus
  - [ ] Tags applied to output file
  - [ ] Chapters present (if available)
  - [ ] Cover art from API embedded

  **Commit**: NO (testing)

---

- [ ] 6. Verify Complete Workflow

  **What to do**:
  - Test single MP3 with ASIN
  - Test single M4B with ASIN  
  - Test multiple M4B merge with ASIN
  - Compare outputs to expected behavior

  **Acceptance Criteria**:
  - [ ] All test files process successfully
  - [ ] Metadata applied correctly
  - [ ] No FFmpeg errors
  - [ ] No API hangs
  - [ ] Output files playable

  **Commit**: NO (testing)

---

## Commit Strategy

| Task | Message | Files |
|------|---------|-------|
| 2 | fix(ffmpeg): correct cover art handling for MP3 to M4B conversion | src/audio/ffmpeg.rs, src/merge.rs |
| 3 | fix(api): add timeouts and error handling for network requests | src/api/*.rs |

---

## Success Criteria

### Verification Commands
```bash
# Test MP3 processing
docker run --rm \
  -v "/home/djdembeck/input/M Chen - Star Wars Brotherhood.MP3":/input \
  -v /home/djdembeck/projects/github/m4b-merge/test-outputs/rust:/output \
  m4b-merge-rust:test \
  -i /input -o /output --asin B09HR33QHH

# Verify output
ls -lh test-outputs/rust/
ffprobe -v error -show_streams test-outputs/rust/*.m4b

# Check metadata
ffprobe -v error -show_format test-outputs/rust/*.m4b | grep -E "(title|artist|album)"
```

### Definition of Done
- [ ] MP3 with embedded cover art processes successfully
- [ ] API metadata fetch works from Docker container
- [ ] Both fixes committed with clear messages
- [ ] Test files processed without errors
- [ ] Feature parity achieved for basic workflows

---

## Technical Notes

### Cover Art Handling
M4B/AAC files should contain cover art as:
- mjpeg (still image) in video stream, OR
- Metadata tag (ID3/MP4 atoms)

NOT as h264 video (which is what current code attempts).

### API Timeouts
reqwest default timeout is None (infinite). Must explicitly set:
```rust
let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(30))
    .build()?;
```

### Testing Approach
1. Fix code
2. Rebuild Docker image
3. Test with actual files
4. Verify output with ffprobe
5. Iterate until working
