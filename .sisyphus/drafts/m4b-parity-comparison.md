# Draft: M4B-Merge Feature Parity Comparison

## Test Strategy Overview

**Goal**: Compare Rust vs Python implementations for feature parity

**Progressive Test Phases**:
1. Single MP3 file
2. Single M4B file
3. Multiple M4B files

**Test Input**: User will provide a specific ASIN for metadata lookup

**Verification Dimensions**:
- [ ] Output file structure (directories, naming)
- [ ] Metadata accuracy (tags, chapters, cover art)
- [ ] Audio quality (bit rate, sample rate, encoding)
- [ ] File size consistency
- [ ] Processing time comparison
- [ ] Error handling behavior

## Test Execution Plan

### Environment Setup
- Build Rust binary locally OR use Rust Dockerfile
- Run Python Docker container
- Prepare test output directories

### Comparison Methodology
For each phase:
1. Run Rust implementation on input
2. Run Python Docker on same input
3. Compare outputs systematically
4. Document differences

### Metrics to Capture
- Execution time
- Exit codes
- Console output (stdout/stderr)
- Output file list with sizes
- File hashes (for binary comparison)
- Metadata extraction (ffprobe)
- Audio codec info

## Questions to Resolve
- [ ] Does user have a specific ASIN in mind?
- [ ] Where are the test input files located?
- [ ] Should we use local Rust binary or Docker?
- [ ] Where should output comparisons be saved?
