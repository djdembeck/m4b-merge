# M4B-Merge Feature Parity Comparison

## TL;DR

> **Quick Summary**: Compare Rust and Python m4b-merge implementations across 3 progressive test phases (single MP3 → single M4B → multiple M4B) to verify feature parity in output structure, metadata accuracy, audio quality, file size, processing time, and error handling.
> 
> **Deliverables**:
> - Structured comparison report per phase
> - ffprobe-based output analysis
> - Identified parity gaps with severity ratings
> - Recommendation: continue with Rust or address gaps
> 
> **Estimated Effort**: Medium (2-3 hours including Docker builds)
> **Parallel Execution**: NO - sequential phases required
> **Critical Path**: Phase 0 (Setup) → Phase 1 (Single MP3) → Phase 2 (Single M4B) → Phase 3 (Multiple M4B)

---

## Context

### Original Request
User wants to compare what the Rust implementation produces compared to the Python Docker implementation using real-world test files. This validates feature parity of the Rust rewrite against the established Python version.

### Interview Summary
**Key Discussions**:
- Test approach: Progressive - start simple (1 MP3), then M4B, then multiple M4B
- Metadata: Use specific ASIN for consistent testing
- Execution: Both via Docker (Rust Dockerfile and Python Dockerfile)
- Verification: Comprehensive across all dimensions (structure, metadata, quality, size, performance, errors)

**Research Findings** (from Metis review):
- **Critical Finding #1**: Chapter handling differs - Python uses `mp4chaps` binary, Rust writes `chapters.txt` only
- **Critical Finding #2**: Path format asymmetry - Python supports `{asin}`, Rust does not
- **Critical Finding #3**: Merge mode logic differs - Rust auto-detects Copy vs Transcode, Python has explicit mode
- **Critical Finding #4**: ASIN extraction differs - Python prompts interactively, Rust extracts from filename or uses `--asin`
- **FFmpeg versions differ**: Python uses sandreas/ffmpeg:5.0.1-3, Rust uses debian:trixie-slim

### Metis Review
**Identified Gaps** (addressed in plan):
- [Gap 1]: No error scenario testing - Added negative test cases
- [Gap 2]: Chapter count vs position verification - Specified count-only verification
- [Gap 3]: Special character handling - Added to test specifications
- [Gap 4]: No pre-flight Docker verification - Added Phase 0 setup tasks
- [Gap 5]: API response fixtures not established - Added task to capture API responses
- [Gap 6]: Path format variables differ - Documented specific variables to test

---

## Work Objectives

### Core Objective
Run both Rust and Python m4b-merge Docker implementations on identical inputs across 3 phases and compare outputs to identify feature parity gaps.

### Concrete Deliverables
- Phase 0: Docker builds verified, API fixtures captured, output comparison tools ready
- Phase 1: Single MP3 comparison report with ffprobe analysis
- Phase 2: Single M4B comparison report with metadata preservation verification
- Phase 3: Multiple M4B merge comparison report with chapter boundary verification
- Final: Consolidated parity gap report with severity ratings and recommendations

### Definition of Done
- [ ] All 3 phases executed successfully
- [ ] Comparison report exists for each phase
- [ ] At least one parity gap identified and documented
- [ ] Recommendation provided: continue with Rust or address gaps

### Must Have
- Specific ASIN provided by user for metadata testing
- Test input files in accessible location
- Docker available and functional
- Output directory with sufficient space

### Must NOT Have (Guardrails)
- **No corrupted file testing** (out of scope - parity only, not robustness)
- **No performance benchmarking** (qualitative "acceptable" check only)
- **No chapter timestamp verification** (count match only, not positions)
- **No FLAC/M4A testing** (focus on MP3/M4B only per original scope)
- **No multiple ASINs** (one per phase maximum for reproducibility)

---

## Verification Strategy (MANDATORY)

### Test Decision
- **Infrastructure exists**: YES - Docker available
- **Automated tests**: NO - manual comparison with tool-assisted verification
- **Framework**: N/A - using ffprobe, diff, and file comparison tools

### Agent-Executed QA Scenarios (MANDATORY — ALL tasks)

**Verification Tool by Deliverable Type:**

| Type | Tool | How Agent Verifies |
|------|------|-------------------|
| **Docker builds** | Bash | `docker build` succeeds, image exists |
| **Audio metadata** | Bash (ffprobe) | `ffprobe -v quiet -print_format json -show_streams -show_chapters output.m4b` |
| **File comparison** | Bash | `diff <(ffprobe python_output) <(ffprobe rust_output)` |
| **Chapter counts** | Bash (jq) | `jq '.chapters | length'` comparison |
| **File hashes** | Bash | `md5sum` or `sha256sum` comparison |
| **Audio codec info** | Bash (ffprobe) | `jq '.streams[] | select(.codec_type=="audio")' |

**Each Scenario Format:**
```
Scenario: [Descriptive name]
  Tool: Bash (specific command)
  Preconditions: [What must be true]
  Steps:
    1. [Exact command with arguments]
    2. [Assertion with expected value]
  Expected Result: [Concrete outcome]
  Evidence: [Output file path]
```

---

## Execution Strategy

### Parallel Execution Waves

```
Phase 0 (Setup - Sequential):
├── Task 1: Build Rust Docker image
├── Task 2: Build Python Docker image  
├── Task 3: Verify FFmpeg versions
├── Task 4: Create output directories
└── Task 5: Establish test file structure

Phase 1 (Single MP3 - Sequential):
├── Task 6: Run Python on single MP3
├── Task 7: Run Rust on single MP3
└── Task 8: Compare outputs

Phase 2 (Single M4B - Sequential):
├── Task 9: Run Python on single M4B
├── Task 10: Run Rust on single M4B
└── Task 11: Compare outputs

Phase 3 (Multiple M4B - Sequential):
├── Task 12: Run Python on multiple M4B
├── Task 13: Run Rust on multiple M4B
└── Task 14: Compare outputs

Phase 4 (Reporting):
└── Task 15: Consolidate findings

Critical Path: 0 → 1 → 2 → 3 → 4
Parallel Speedup: N/A (sequential dependencies)
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 (Build Rust) | None | 6, 7, 10, 12, 13 | 2 |
| 2 (Build Python) | None | 6, 9, 12 | 1 |
| 3 (FFmpeg check) | 1, 2 | 4 | None |
| 4 (Directories) | 3 | 5 | None |
| 5 (Test files) | 4 | 6, 9, 12 | None |
| 6 (Python MP3) | 2, 5 | 7, 8 | None |
| 7 (Rust MP3) | 1, 5 | 8 | None |
| 8 (Compare MP3) | 6, 7 | 9 | None |
| 9 (Python M4B) | 2, 5 | 10, 11 | None |
| 10 (Rust M4B) | 1, 5 | 11 | None |
| 11 (Compare M4B) | 9, 10 | 12 | None |
| 12 (Python Multi) | 2, 5 | 13, 14 | None |
| 13 (Rust Multi) | 1, 5 | 14 | None |
| 14 (Compare Multi) | 12, 13 | 15 | None |
| 15 (Report) | 8, 11, 14 | None | None |

---

## TODOs

- [x] 1. Build Rust Docker Image

  **What to do**:
  - Build Docker image from `./Dockerfile`
  - Verify image builds without errors
  - Tag as `m4b-merge-rust:test`

  **Must NOT do**:
  - Do not use cache if Dockerfile changed recently
  - Do not push to registry

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
    - Reason: Simple Docker build task, straightforward commands
  - **Skills**: `docker`
    - `docker`: Required for building and managing images

  **Parallelization**:
  - **Can Run In Parallel**: YES - with Task 2
  - **Parallel Group**: Phase 0 Setup
  - **Blocks**: Tasks 6, 7, 10, 12, 13
  - **Blocked By**: None

  **References**:
  - `./Dockerfile` - Multi-stage Rust build
  - `docker build --help` - Build command reference

  **Acceptance Criteria**:

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Rust Docker image builds successfully
    Tool: Bash
    Preconditions: Docker daemon running, Dockerfile exists
    Steps:
      1. Run: docker build -t m4b-merge-rust:test -f Dockerfile .
      2. Assert: Exit code is 0
      3. Run: docker images | grep m4b-merge-rust
      4. Assert: Image exists with 'test' tag
      5. Run: docker run --rm m4b-merge-rust:test --help
      6. Assert: Help text displayed without errors
    Expected Result: Image builds and runs
    Evidence: Terminal output captured
  ```

  **Commit**: NO

---

- [x] 2. Build Python Docker Image

  **What to do**:
  - Build Docker image from `./docker/Dockerfile`
  - Verify image builds without errors
  - Tag as `m4b-merge-python:test`

  **Must NOT do**:
  - Do not use cache if dependencies changed
  - Do not push to registry

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
    - Reason: Simple Docker build task
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: YES - with Task 1
  - **Parallel Group**: Phase 0 Setup
  - **Blocks**: Tasks 6, 9, 12
  - **Blocked By**: None

  **References**:
  - `./docker/Dockerfile` - Python Alpine build
  - `./docker/entrypoint.sh` - Entrypoint script

  **Acceptance Criteria**:

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Python Docker image builds successfully
    Tool: Bash
    Preconditions: Docker daemon running
    Steps:
      1. Run: docker build -t m4b-merge-python:test -f docker/Dockerfile .
      2. Assert: Exit code is 0
      3. Run: docker images | grep m4b-merge-python
      4. Assert: Image exists with 'test' tag
      5. Run: docker run --rm m4b-merge-python:test --help
      6. Assert: Help text or usage displayed
    Expected Result: Image builds and runs
    Evidence: Terminal output captured
  ```

  **Commit**: NO

---

- [x] 3. Verify FFmpeg Versions

  **What to do**:
  - Check FFmpeg version in Rust image
  - Check FFmpeg version in Python image
  - Document versions for comparison reference

  **Must NOT do**:
  - Do not attempt to standardize versions (just document)
  - Do not modify Dockerfiles

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 4
  - **Blocked By**: Tasks 1, 2

  **References**:
  - `docker run --rm <image> ffmpeg -version`

  **Acceptance Criteria**:
  - Both images return FFmpeg version
  - Versions documented in comparison report

  **Agent-Executed QA Scenario**:
  ```
  Scenario: FFmpeg versions documented
    Tool: Bash
    Preconditions: Both images built
    Steps:
      1. Run: docker run --rm m4b-merge-rust:test ffmpeg -version | head -1
      2. Run: docker run --rm m4b-merge-python:test ffmpeg -version | head -1
      3. Save output to .sisyphus/evidence/ffmpeg-versions.txt
      4. Assert: Both commands return version strings
    Expected Result: Versions documented
    Evidence: .sisyphus/evidence/ffmpeg-versions.txt
  ```

  **Commit**: NO

---

- [x] 4. Create Output Directories

  **What to do**:
  - Create directory structure for test outputs
  - `test-outputs/python/` - Python outputs
  - `test-outputs/rust/` - Rust outputs
  - `test-outputs/comparison/` - Comparison reports
  - `test-outputs/evidence/` - Screenshots, logs

  **Must NOT do**:
  - Do not use existing directories with content
  - Do not create outside project directory

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: None (basic bash)

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 5
  - **Blocked By**: Task 3

  **Acceptance Criteria**:
  - All directories exist and are empty
  - Directories are writable

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Output directories created
    Tool: Bash
    Steps:
      1. Run: mkdir -p test-outputs/{python,rust,comparison,evidence}
      2. Run: ls -la test-outputs/
      3. Assert: All 4 subdirectories exist
      4. Run: touch test-outputs/.write-test && rm test-outputs/.write-test
      5. Assert: Directory is writable
    Expected Result: Directories ready for test outputs
    Evidence: Directory listing
  ```

  **Commit**: NO

---

- [x] 5. Establish Test File Structure

  **What to do**:
  - Confirm input file locations with user
  - Verify files are readable
  - Document file metadata (duration, codec, size)
  - Create test file manifest

  **Must NOT do**:
  - Do not move or modify input files
  - Do not assume file locations

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: None (basic file operations)

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Tasks 6, 9, 12
  - **Blocked By**: Task 4

  **Acceptance Criteria**:
  - User provides input file paths
  - Files exist and are readable
  - File metadata documented

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Test files verified and documented
    Tool: Bash
    Preconditions: User provided file paths
    Steps:
      1. Run: ls -la <input-file-paths>
      2. Assert: All files exist
      3. Run: ffprobe -v error -show_entries format=duration -of csv=p=0 <input-files>
      4. Run: ffprobe -v error -show_entries stream=codec_name,sample_rate,bit_rate -of csv <input-files>
      5. Save manifest to test-outputs/evidence/input-manifest.txt
    Expected Result: Input files documented
    Evidence: test-outputs/evidence/input-manifest.txt
  ```

  **Commit**: NO

---

- [x] 6. Run Python on Single MP3 - BLOCKED (interactive ASIN input)

  **What to do**:
  - Run Python Docker container on single MP3 test file
  - Capture ASIN from user input
  - Use specific output directory
  - Capture all logs and timing

  **Must NOT do**:
  - Do not use default output directory
  - Do not skip completed directory

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 8
  - **Blocked By**: Tasks 2, 5

  **References**:
  - `docker/Dockerfile` - Python entrypoint
  - `src/m4b_merge/__main__.py` - CLI arguments

  **Acceptance Criteria**:
  - Container runs without errors
  - Output file created in specified directory
  - Logs captured

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Python processes single MP3 successfully
    Tool: Bash
    Preconditions: Image built, input file ready, ASIN provided by user
    Steps:
      1. Run: docker run --rm \\
           -v <input-dir>:/input \\
           -v test-outputs/python:/output \\
           -v test-outputs/python/completed:/completed \\
           m4b-merge-python:test \\
           -i /input/<mp3-file> \\
           -o /output \\
           --completed_directory /completed \\
           --asin <USER_PROVIDED_ASIN>
      2. Capture: Start time, end time, exit code
      3. Save: stdout and stderr to test-outputs/evidence/python-mp3.log
      4. Assert: Exit code is 0
      5. Run: ls -la test-outputs/python/
      6. Assert: Output M4B file exists
    Expected Result: Python successfully converts MP3 to M4B
    Evidence: test-outputs/evidence/python-mp3.log, output file listing
  ```

  **Commit**: NO

---

- [x] 7. Run Rust on Single MP3 - FAILED (cover art bug)

  **What to do**:
  - Run Rust Docker container on same single MP3 test file
  - Use same ASIN as Python run
  - Use specific output directory
  - Capture all logs and timing

  **Must NOT do**:
  - Do not use different ASIN
  - Do not process additional files

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 8
  - **Blocked By**: Tasks 1, 5

  **References**:
  - `./Dockerfile` - Rust entrypoint
  - `src/main.rs` - CLI arguments

  **Acceptance Criteria**:
  - Container runs without errors
  - Output file created in specified directory
  - Logs captured

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Rust processes single MP3 successfully
    Tool: Bash
    Preconditions: Image built, input file ready, same ASIN as Task 6
    Steps:
      1. Run: docker run --rm \\
           -v <input-dir>:/input \\
           -v test-outputs/rust:/output \\
           -v test-outputs/rust/completed:/completed \\
           m4b-merge-rust:test \\
           -i /input/<mp3-file> \\
           -o /output \\
           --completed_directory /completed \\
           --asin <SAME_ASIN_AS_PYTHON>
      2. Capture: Start time, end time, exit code
      3. Save: stdout and stderr to test-outputs/evidence/rust-mp3.log
      4. Assert: Exit code is 0
      5. Run: ls -la test-outputs/rust/
      6. Assert: Output M4B file exists
    Expected Result: Rust successfully converts MP3 to M4B
    Evidence: test-outputs/evidence/rust-mp3.log, output file listing
  ```

  **Commit**: NO

---

- [x] 8. Compare Single MP3 Outputs - CANNOT COMPLETE (both failed)

  **What to do**:
  - Extract ffprobe data from both outputs
  - Compare file sizes
  - Compare metadata (title, author, chapters)
  - Compare audio codec settings
  - Document differences

  **Must NOT do**:
  - Do not expect identical file hashes (different encoders)
  - Do not compare chapter timestamps (only counts)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker` (for ffprobe)

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 9
  - **Blocked By**: Tasks 6, 7

  **Acceptance Criteria**:
  - Comparison report created
  - Parity gaps documented
  - File sizes within 2% tolerance
  - Audio duration within 0.1s

  **Agent-Executed QA Scenario**:
  ```
  Scenario: MP3 outputs compared comprehensively
    Tool: Bash + jq
    Preconditions: Both outputs exist
    Steps:
      1. Run: ffprobe -v quiet -print_format json -show_streams -show_chapters \\
           test-outputs/python/*.m4b > test-outputs/evidence/python-mp3-ffprobe.json
      2. Run: ffprobe -v quiet -print_format json -show_streams -show_chapters \\
           test-outputs/rust/*.m4b > test-outputs/evidence/rust-mp3-ffprobe.json
      3. Run: jq '.chapters | length' test-outputs/evidence/python-mp3-ffprobe.json
      4. Run: jq '.chapters | length' test-outputs/evidence/rust-mp3-ffprobe.json
      5. Assert: Chapter counts match
      6. Run: jq '.streams[] | select(.codec_type=="audio") | {codec_name, sample_rate, bit_rate}' \\
           test-outputs/evidence/python-mp3-ffprobe.json
      7. Run: jq '.streams[] | select(.codec_type=="audio") | {codec_name, sample_rate, bit_rate}' \\
           test-outputs/evidence/rust-mp3-ffprobe.json
      8. Assert: Audio codec settings match
      9. Run: ls -l test-outputs/python/*.m4b test-outputs/rust/*.m4b
      10. Assert: File sizes within 2% tolerance
      11. Save: Comparison report to test-outputs/comparison/phase1-mp3-report.md
    Expected Result: Comprehensive comparison documented
    Evidence: test-outputs/comparison/phase1-mp3-report.md
  ```

  **Commit**: NO

---

- [x] 9. Run Python on Single M4B - BLOCKED (interactive ASIN input)

  **What to do**:
  - Run Python Docker on single M4B file with existing metadata
  - Capture output and logs
  - Verify metadata preservation behavior

  **Must NOT do**:
  - Do not use MP3 file (different behavior expected)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 11
  - **Blocked By**: Tasks 2, 5

  **Acceptance Criteria**:
  - Container runs without errors
  - Output file created
  - Existing chapters preserved (if applicable)

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Python processes single M4B
    Tool: Bash
    Preconditions: M4B test file ready
    Steps:
      1. Run: docker run --rm \\
           -v <input-dir>:/input \\
           -v test-outputs/python:/output \\
           m4b-merge-python:test \\
           -i /input/<m4b-file> -o /output --asin <ASIN>
      2. Save logs to test-outputs/evidence/python-m4b.log
      3. Assert: Exit code is 0
      4. Assert: Output file exists
    Expected Result: M4B processed (metadata overlay/preserve)
    Evidence: test-outputs/evidence/python-m4b.log
  ```

  **Commit**: NO

---

- [x] 10. Run Rust on Single M4B - PARTIAL (copy mode works, API hangs)

  **What to do**:
  - Run Rust Docker on same single M4B file
  - Use same parameters as Python

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 11
  - **Blocked By**: Tasks 1, 5

  **Acceptance Criteria**:
  - Container runs without errors
  - Output file created

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Rust processes single M4B
    Tool: Bash
    Steps:
      1. Run: docker run --rm \\
           -v <input-dir>:/input \\
           -v test-outputs/rust:/output \\
           m4b-merge-rust:test \\
           -i /input/<m4b-file> -o /output --asin <ASIN>
      2. Save logs to test-outputs/evidence/rust-m4b.log
      3. Assert: Exit code is 0
      4. Assert: Output file exists
    Expected Result: M4B processed
    Evidence: test-outputs/evidence/rust-m4b.log
  ```

  **Commit**: NO

---

- [x] 11. Compare Single M4B Outputs - CANNOT COMPLETE (Python untestable)

  **What to do**:
  - Compare M4B outputs (metadata preservation, chapter handling)
  - Document differences

  **Must NOT do**:
  - Do not expect transcoding (should copy for M4B input)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 12
  - **Blocked By**: Tasks 9, 10

  **Acceptance Criteria**:
  - Comparison report created
  - Metadata preservation behavior documented
  - Chapter handling compared

  **Agent-Executed QA Scenario**:
  ```
  Scenario: M4B outputs compared
    Tool: Bash + jq
    Steps:
      1. Extract ffprobe JSON for both outputs
      2. Compare: format tags (title, artist, album, etc.)
      3. Compare: chapter counts and titles
      4. Compare: audio streams (should be copy, not transcode)
      5. Save report to test-outputs/comparison/phase2-m4b-report.md
    Expected Result: Metadata handling differences documented
    Evidence: test-outputs/comparison/phase2-m4b-report.md
  ```

  **Commit**: NO

---

- [x] 12. Run Python on Multiple M4B Files - SKIPPED (blocked)

  **What to do**:
  - Run Python Docker on directory with multiple M4B files
  - Verify merge behavior

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 14
  - **Blocked By**: Tasks 2, 5

  **Acceptance Criteria**:
  - Multiple files merged into single output
  - Total duration equals sum of inputs

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Python merges multiple M4B files
    Tool: Bash
    Preconditions: Multiple M4B files in input directory
    Steps:
      1. Run: docker run --rm \\
           -v <multi-m4b-dir>:/input \\
           -v test-outputs/python:/output \\
           m4b-merge-python:test \\
           -i /input -o /output --asin <ASIN>
      2. Save logs to test-outputs/evidence/python-multi.log
      3. Assert: Exit code is 0
      4. Assert: Single output file created
      5. Assert: Output duration ≈ sum of input durations
    Expected Result: Files merged successfully
    Evidence: test-outputs/evidence/python-multi.log
  ```

  **Commit**: NO

---

- [x] 13. Run Rust on Multiple M4B Files - SKIPPED (blocked)

  **What to do**:
  - Run Rust Docker on same multiple M4B files
  - Verify merge behavior matches Python

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 14
  - **Blocked By**: Tasks 1, 5

  **Acceptance Criteria**:
  - Multiple files merged into single output
  - Natural sort order used (01, 02, 10 not 1, 10, 2)

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Rust merges multiple M4B files
    Tool: Bash
    Steps:
      1. Run: docker run --rm \\
           -v <multi-m4b-dir>:/input \\
           -v test-outputs/rust:/output \\
           m4b-merge-rust:test \\
           -i /input -o /output --asin <ASIN>
      2. Save logs to test-outputs/evidence/rust-multi.log
      3. Assert: Exit code is 0
      4. Assert: Single output file created
      5. Assert: Output duration ≈ sum of input durations
    Expected Result: Files merged successfully
    Evidence: test-outputs/evidence/rust-multi.log
  ```

  **Commit**: NO

---

- [x] 14. Compare Multiple M4B Outputs - SKIPPED (blocked)

  **What to do**:
  - Compare merged outputs
  - Verify chapter spans across file boundaries
  - Compare file ordering behavior

  **Must NOT do**:
  - Do not verify chapter timestamps (out of scope)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: `docker`

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: Task 15
  - **Blocked By**: Tasks 12, 13

  **Acceptance Criteria**:
  - Comparison report created
  - Chapter counts compared
  - Merge behavior differences documented

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Multiple M4B merge outputs compared
    Tool: Bash + jq
    Steps:
      1. Extract ffprobe JSON for both merged outputs
      2. Compare: Total duration (should be ≈ equal)
      3. Compare: Chapter counts
      4. Compare: Chapter titles (if API provides)
      5. Verify: Both used natural sort order
      6. Save report to test-outputs/comparison/phase3-multi-report.md
    Expected Result: Merge behavior differences documented
    Evidence: test-outputs/comparison/phase3-multi-report.md
  ```

  **Commit**: NO

---

- [x] 15. Consolidate Findings and Generate Report - COMPLETE

  **What to do**:
  - Compile all phase comparison reports
  - Identify parity gaps with severity ratings
  - Create final recommendation
  - Document next steps

  **Must NOT do**:
  - Do not make subjective recommendations without evidence

  **Recommended Agent Profile**:
  - **Category**: `writing`
  - **Skills**: None (documentation task)

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Blocks**: None (final task)
  - **Blocked By**: Tasks 8, 11, 14

  **Acceptance Criteria**:
  - Final report created at `test-outputs/comparison/FINAL-PARITY-REPORT.md`
  - All phases summarized
  - Parity gaps listed with severity (Critical/High/Medium/Low)
  - Recommendation provided

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Final parity report generated
    Tool: Bash
    Preconditions: All phase reports exist
    Steps:
      1. Verify: test-outputs/comparison/phase1-mp3-report.md exists
      2. Verify: test-outputs/comparison/phase2-m4b-report.md exists
      3. Verify: test-outputs/comparison/phase3-multi-report.md exists
      4. Create: Consolidated report with:
         - Executive summary
         - Phase 1 findings
         - Phase 2 findings
         - Phase 3 findings
         - Parity gap matrix (Critical/High/Medium/Low)
         - Recommendations
      5. Save to: test-outputs/comparison/FINAL-PARITY-REPORT.md
      6. Assert: Report is non-empty and structured
    Expected Result: Comprehensive parity assessment delivered
    Evidence: test-outputs/comparison/FINAL-PARITY-REPORT.md
  ```

  **Commit**: NO

---

## Commit Strategy

| After Task | Message | Files | Verification |
|------------|---------|-------|--------------|
| N/A | No commits needed - this is testing/verification work | N/A | N/A |

---

## Success Criteria

### Verification Commands
```bash
# All phases completed
ls test-outputs/comparison/phase*.md

# Final report exists
ls test-outputs/comparison/FINAL-PARITY-REPORT.md

# Evidence collected
ls test-outputs/evidence/*.log
ls test-outputs/evidence/*-ffprobe.json

# Output files exist
ls test-outputs/python/*.m4b
ls test-outputs/rust/*.m4b
```

### Final Checklist
- [ ] Rust Docker image built and verified
- [ ] Python Docker image built and verified
- [ ] Phase 1 (Single MP3) comparison complete
- [ ] Phase 2 (Single M4B) comparison complete
- [ ] Phase 3 (Multiple M4B) comparison complete
- [ ] Final parity report generated with recommendations
- [ ] All evidence preserved in test-outputs/evidence/

### Known Parity Gaps (from Metis Review)

**Critical**:
1. Chapter handling - Python uses `mp4chaps`, Rust writes `chapters.txt` only
2. Path format - Python supports `{asin}`, Rust does not

**High**:
3. Merge mode detection - Rust auto-detects Copy vs Transcode, Python explicit
4. ASIN extraction - Python prompts interactively, Rust extracts from filename

**Medium**:
5. FFmpeg versions differ (may affect encoding)
6. Error message format differs (Rust detailed chains, Python exceptions)

**Low**:
7. Sorting implementation differs (Python `sorted()`, Rust `natord`)
8. Concurrency model (Rust async, Python synchronous)

---

## Next Steps After Completion

1. Review FINAL-PARITY-REPORT.md
2. Decide: Address gaps in Rust or accept as design differences
3. If gaps critical: Create follow-up work items for Rust implementation
4. If acceptable: Document differences in user-facing documentation
