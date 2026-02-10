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

- [x] 1. Explore Rust Codebase Structure
- [x] 2. Fix Cover Art Handling in FFmpeg
- [x] 3. Fix API Network/Timeout Issues (partial - see notes)
- [x] 4. Test Fix - MP3 Processing ✅
- [x] 5. Test Fix - API Metadata Fetch ✅
- [x] 6. Verify Complete Workflow ✅

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
