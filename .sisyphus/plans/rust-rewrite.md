# m4b-merge Rust Rewrite

## TL;DR

> **Quick Summary**: Rewrite m4b-merge from Python to Rust, replacing m4b-tool (PHP) dependency with direct FFmpeg orchestration. Results in a single static binary (~10-20MB) with only ffmpeg as external dependency.
> 
> **Deliverables**:
> - `m4b-merge` Rust CLI binary with feature parity to Python version
> - Docker image with bundled ffmpeg
> - Test suite verifying output compatibility
> - Migration guide for existing users
> 
> **Estimated Effort**: Medium-Large (4-6 weeks with 1 developer, or 2-3 weeks with prototyping)
> **Parallel Execution**: YES - Phase 1 scaffolding and Phase 2 core can overlap with good interfaces
> **Critical Path**: FFmpeg POC → Core merge logic → Chapter handling → Integration testing

---

## Context

### Original Request
Rewrite m4b-merge from Python to Rust, eliminating all dependencies except ffmpeg (and optionally tone to start). Current architecture chains through m4b-tool (PHP), adding unnecessary complexity.

### Interview Summary
**Key Discussions**:
- Current m4b-merge shells out to m4b-tool (PHP binary)
- m4b-tool wraps ffmpeg, mp4v2, tone, and fdkaac
- User wants standalone Rust binary with ffmpeg-only dependency
- FFmpeg can handle 95% of operations; MP4 chapters need native library

**Research Findings**:
- **FFmpeg capabilities**: concat demuxer for merging, metadata injection, silence detection, format conversion
- **Rust ecosystem**: `ffmpeg-next` for FFmpeg bindings, `mp4ameta` for MP4 metadata/chapters, `reqwest` for HTTP
- **Tone**: C# binary that's optional in current tool; can use FFmpeg fallback
- **MP4 chapters**: Complex - need both QuickTime (ffmpeg) and Nero (mp4ameta) formats

### Metis Review
**Identified Gaps** (addressed in plan):
- MP4 chapter atom complexity requires proof-of-concept first
- Tone dependency creates dual code paths - plan uses FFmpeg-only as primary
- Async runtime may be overkill - starting synchronous, adding async later if needed
- Test strategy needs test fixtures or generation approach
- Cross-compilation for Docker requires musl/static linking consideration

---

## Work Objectives

### Core Objective
Create a Rust CLI tool that replaces the Python m4b-merge with identical functionality, eliminating PHP/m4b-tool dependency and producing a single static binary that only requires ffmpeg.

### Concrete Deliverables
- `m4b-merge` binary executable (Linux amd64/arm64, macOS, Windows)
- `Cargo.toml` with all dependencies configured
- `src/` directory with modular architecture
- `tests/` directory with integration tests
- `Dockerfile` with ffmpeg bundled
- `README.md` with installation and usage

### Definition of Done
- [x] All existing CLI arguments supported
- [x] All path_format templates produce identical output paths
- [x] Output files pass mutagen verification (same as Python version)
- [x] Docker image runs successfully on test inputs
- [x] CI/CD pipeline builds and tests on PR

### Must Have
- Single m4b/m4a/mp3 input → tagged m4b output
- Multiple file merge with chapter generation
- Multi-disc directory support
- Audible API integration (audnexus)
- Cover art embedding
- Metadata tagging (title, author, narrator, series, etc.)
- Configurable output path templates
- Completed directory file moving
- FFmpeg as only external dependency

### Must NOT Have (Guardrails)
- GUI or TUI interface (CLI only)
- Format support beyond m4b/m4a/mp3 (v2.0 feature)
- Automatic ASIN detection from audio fingerprinting
- Chapter auto-detection via silence (experimental flag only)
- Library crate extraction (post-MVP)
- Parallel processing of multiple inputs (sequential only)
- Dry-run mode (post-MVP)

### Branch Strategy
**CRITICAL: All work MUST be done on a new feature branch**

- **Base branch**: `develop` (or `main` if no develop branch exists)
- **Feature branch name**: `feat/rust-rewrite`
- **Workflow**:
  1. Create branch: `git checkout -b feat/rust-rewrite origin/develop`
  2. All Rust code commits go to this branch
  3. Python code in original directories remains untouched on develop
  4. Open PR when ready for review
  5. Merge to develop after approval

This keeps the Rust rewrite isolated until it's ready to replace the Python implementation.

---

## Verification Strategy (MANDATORY)

### Test Decision
- **Infrastructure exists**: NO (new Rust project)
- **Automated tests**: YES (Tests-after)
- **Framework**: `cargo test` with standard Rust testing

### If TDD Enabled
NOT ENABLED - Using tests-after approach due to FFI complexity and need for FFmpeg in tests.

### Agent-Executed QA Scenarios (MANDATORY — ALL tasks)

**Verification Tool by Deliverable Type:**

| Type | Tool | How Agent Verifies |
|------|------|-------------------|
| **Rust/Binary** | Bash | Build, run, check output |
| **Audio Processing** | Bash (ffmpeg, ffprobe) | Verify output files, metadata |
| **API Integration** | Bash (curl) | Mock API responses |
| **Docker** | Bash | Build image, run container |

**Each Scenario Format:**

```
Scenario: [Descriptive name]
  Tool: [Bash]
  Preconditions: [What must be true]
  Steps:
    1. [Exact command with args]
    2. [Assertion with expected value]
  Expected Result: [Concrete outcome]
  Evidence: [Output capture path]
```

**Example QA Scenario:**

```
Scenario: Build produces working binary
  Tool: Bash
  Preconditions: Rust toolchain installed, cargo available
  Steps:
    1. cargo build --release
    2. Assert: target/release/m4b-merge exists
    3. ./target/release/m4b-merge --version
    4. Assert: stdout contains version number
  Expected Result: Binary builds and runs
  Evidence: .sisyphus/evidence/build-success.txt

Scenario: Merge two MP3 files produces valid M4B
  Tool: Bash (ffmpeg for test data, ffprobe for verification)
  Preconditions: Binary built, test-inputs/ directory exists with two MP3s
  Steps:
    1. ./target/release/m4b-merge -i test-inputs/ --skip-api
    2. ffprobe -v error -show_entries format=duration -of csv=p=0 output.m4b
    3. Assert: duration approximately equals sum of input durations
    4. ffprobe -v error -show_entries format_tags -of json output.m4b
    5. Assert: JSON contains metadata tags
  Expected Result: Merged M4B with correct duration and metadata
  Evidence: .sisyphus/evidence/merge-test-output.json
```

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation - Start Immediately):
├── Task 1: Project scaffolding and dependencies
├── Task 2: FFmpeg discovery and wrapper
└── Task 5: Configuration and CLI parsing

Wave 2 (Core Logic - After Wave 1):
├── Task 3: Audio file discovery and validation
├── Task 6: Metadata structures and API client
└── Task 8: Chapter handling (POC first)

Wave 3 (Integration - After Wave 2):
├── Task 4: Merge/conversion logic
├── Task 7: Tagging and file operations
└── Task 9: Integration and polish

Wave 4 (Packaging - After Wave 3):
└── Task 10: Docker and CI/CD

Critical Path: Task 1 → Task 2 → Task 3 → Task 4 → Task 9 → Task 10
Parallel Speedup: ~30% faster than sequential
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 (Scaffold) | None | 2, 5 | - |
| 2 (FFmpeg) | 1 | 3, 4 | 5 |
| 3 (Discovery) | 2 | 4 | 6 |
| 4 (Merge) | 2, 3 | 7 | - |
| 5 (Config) | 1 | 7 | 2 |
| 6 (Metadata) | 1 | 7 | 3 |
| 7 (Tagging) | 4, 5, 6 | 9 | - |
| 8 (Chapters POC) | 2 | 4, 7 | 3, 6 |
| 9 (Integration) | 7 | 10 | - |
| 10 (Docker) | 9 | None | - |

### Agent Dispatch Summary

| Wave | Tasks | Recommended Agents |
|------|-------|-------------------|
| 1 | 1, 2, 5 | task(category="quick", load_skills=[], run_in_background=false) |
| 2 | 3, 6, 8 | task(category="unspecified-high", load_skills=[], run_in_background=false) |
| 3 | 4, 7, 9 | task(category="unspecified-high", load_skills=[], run_in_background=false) |
| 4 | 10 | task(category="quick", load_skills=["axonhub-docker"], run_in_background=false) |

---

## TODOs

- [x] 1. Project Scaffolding and Dependencies

  **What to do**:
  - **Create feature branch first**: `git checkout -b feat/rust-rewrite origin/develop`
  - Initialize Rust project in project root: `cargo init --name m4b-merge`
  - Set up workspace structure: `src/`, `tests/`, `resources/`
  - Add dependencies to Cargo.toml:
    ```toml
    [dependencies]
    clap = { version = "4.0", features = ["derive"] }
    serde = { version = "1.0", features = ["derive"] }
    serde_json = "1.0"
    reqwest = { version = "0.11", features = ["json"] }
    tokio = { version = "1.0", features = ["full"] }
    anyhow = "1.0"
    thiserror = "1.0"
    tracing = "0.1"
    tracing-subscriber = "0.3"
    ffmpeg-next = "6.0"
    mp4ameta = "0.8"
    sanitize-filename = "0.5"
    tempfile = "3.0"
    ```
  - Create basic module structure: `main.rs`, `config.rs`, `audio.rs`, `metadata.rs`, `api.rs`, `chapters.rs`
  - Set up `.gitignore`, `rustfmt.toml`, `clippy.toml`

  **Must NOT do**:
  - Don't add async runtime yet (tokio is there but use sync first)
  - Don't add GUI dependencies
  - Don't add tone dependency yet

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  - **Skills Evaluated but Omitted**: 
    - `git-master`: Not needed for scaffolding
    - `frontend-ui-ux`: Not applicable (CLI tool)

  **Parallelization**:
  - **Can Run In Parallel**: NO (foundation task)
  - **Parallel Group**: Wave 1
  - **Blocks**: Tasks 2, 5
  - **Blocked By**: None

  **References**:
  - https://doc.rust-lang.org/cargo/guide/project-layout.html - Rust project structure
  - https://github.com/sandreas/m4b-tool - Reference for feature parity
  - Current Python: `src/m4b_merge/__main__.py` - CLI argument structure
  - Current Python: `requirements.txt` - Dependencies to replicate

  **Acceptance Criteria**:
  - [x] `cargo build` succeeds
  - [x] `cargo clippy` shows no warnings
  - [x] `cargo test` runs (empty test suite)
  - [x] Binary prints help with `--help`

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Project builds successfully
    Tool: Bash
    Preconditions: Rust toolchain installed
    Steps:
      1. cd /path/to/m4b-merge-rust
      2. cargo build
      3. Assert: exit code 0
      4. cargo clippy -- -D warnings
      5. Assert: exit code 0
    Expected Result: Clean build with no warnings
    Evidence: .sisyphus/evidence/task-1-build.log
  ```

  **Commit**: YES
  - Message: `chore: initial project scaffolding`
  - Files: All new files
  - Pre-commit: `cargo clippy && cargo test`

---

- [x] 2. FFmpeg Discovery and Wrapper

  **What to do**:
  - Implement FFmpeg binary detection (check PATH, common locations)
  - Create `Ffmpeg` struct wrapping subprocess calls
  - Implement version checking: `ffmpeg -version`
  - Implement probe command: `ffprobe -v error -show_format -show_streams -of json`
  - Create typed wrappers for common operations:
    - `get_duration(path) -> Duration`
    - `get_metadata(path) -> AudioMetadata`
    - `detect_silence(path, noise_db, duration) -> Vec<TimeRange>`
  - Implement concat demuxer preparation (generate file list)

  **Must NOT do**:
  - Don't implement actual conversion yet (just discovery/probing)
  - Don't handle all FFmpeg edge cases (basic errors only)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 5)
  - **Parallel Group**: Wave 1
  - **Blocks**: Tasks 3, 4, 8
  - **Blocked By**: Task 1

  **References**:
  - https://docs.rs/ffmpeg-next/latest/ffmpeg_next/ - Rust FFmpeg bindings
  - m4b-tool source: `src/library/M4bTool/Command/AbstractCommand.php` - FFmpeg command building
  - m4b-tool source: `src/library/M4bTool/Command/MergeCommand.php` - Merge-specific FFmpeg usage
  - FFmpeg concat demuxer docs: https://ffmpeg.org/ffmpeg-formats.html#concat

  **Acceptance Criteria**:
  - [x] Detects ffmpeg in PATH
  - [x] Returns helpful error if ffmpeg not found
  - [x] Can probe duration of test audio file
  - [x] Can detect silence boundaries

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: FFmpeg detection works
    Tool: Bash
    Preconditions: FFmpeg installed
    Steps:
      1. ./m4b-merge --check-ffmpeg
      2. Assert: stdout contains "FFmpeg found"
      3. Assert: stdout contains version number
    Expected Result: FFmpeg detected and version displayed
    Evidence: .sisyphus/evidence/task-2-ffmpeg-check.txt

  Scenario: Missing FFmpeg error
    Tool: Bash
    Preconditions: Temporarily move ffmpeg binary
    Steps:
      1. PATH="" ./m4b-merge -i test.mp3
      2. Assert: exit code non-zero
      3. Assert: stderr contains "FFmpeg not found"
    Expected Result: Clear error message about missing dependency
    Evidence: .sisyphus/evidence/task-2-missing-ffmpeg.txt
  ```

  **Commit**: YES
  - Message: `feat: add FFmpeg discovery and basic wrappers`

---

- [x] 3. Audio File Discovery and Validation

  **What to do**:
  - Implement file discovery for input paths (file or directory)
  - Support file extensions: `.mp3`, `.m4a`, `.m4b`
  - Sort files naturally (01.mp3, 02.mp3, 10.mp3 not 01, 10, 02)
  - Handle multi-disc directories (subdirectories with CD1, CD2, etc.)
  - Validate files are readable audio files
  - Extract basic metadata: bitrate, sample rate, duration, codec
  - Group files by directory for batch processing

  **Must NOT do**:
  - Don't validate audio content deeply (just headers)
  - Don't handle exotic formats (WMA, OGG, etc.)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 6)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 4
  - **Blocked By**: Tasks 2

  **References**:
  - Current Python: `src/m4b_merge/helpers.py` - File discovery logic
  - Current Python: `src/m4b_merge/m4b_helper.py:__init__` - Input validation
  - Natural sort algorithm: https://github.com/jeremija/natural-sort

  **Acceptance Criteria**:
  - [x] Discovers all audio files in directory
  - [x] Sorts naturally (not lexicographically)
  - [x] Handles multi-disc subdirectories
  - [x] Returns error for empty directories
  - [x] Validates files are readable

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: File discovery finds all supported files
    Tool: Bash
    Preconditions: Test directory with mixed files
    Steps:
      1. Create test-inputs/ with: 01.mp3, 02.mp3, 10.mp3, readme.txt, cover.jpg
      2. ./m4b-merge --dry-run -i test-inputs/
      3. Assert: stdout lists 01.mp3, 02.mp3, 10.mp3 (in that order)
      4. Assert: stdout does not list readme.txt or cover.jpg
    Expected Result: Only audio files discovered, sorted naturally
    Evidence: .sisyphus/evidence/task-3-discovery.log
  ```

  **Commit**: YES
  - Message: `feat: add audio file discovery and validation`

---

- [x] 4. Merge and Conversion Logic

  **What to do**:
  - Implement FFmpeg concat demuxer for merging multiple files
  - Generate concat file list with proper escaping
  - Support match-bitrate mode (detect source bitrate, use for output)
  - Implement conversion: MP3 → M4B (AAC codec)
  - Pass-through for M4A/M4B (no re-encoding)
  - Handle temporary file management
  - Implement progress reporting (parse FFmpeg stderr)
  - Support num_cpus for parallel encoding (if applicable)

  **Must NOT do**:
  - Don't implement complex filter graphs yet (basic concat only)
  - Don't add format support beyond m4b output

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on discovery and FFmpeg)
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 7
  - **Blocked By**: Tasks 2, 3

  **References**:
  - m4b-tool: `src/library/M4bTool/Command/MergeCommand.php` - Merge implementation
  - FFmpeg concat: https://ffmpeg.org/ffmpeg-formats.html#concat
  - Current Python: `src/m4b_merge/m4b_helper.py:merge_multiple_files`

  **Acceptance Criteria**:
  - [x] Merges multiple files into single M4B
  - [x] Preserves audio quality (no re-encoding for M4A/M4B)
  - [x] Matches source bitrate for MP3 conversions
  - [x] Produces valid M4B file (ffprobe can read)
  - [x] Progress displayed during conversion

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Merge produces valid output file
    Tool: Bash
    Preconditions: Test MP3 files exist
    Steps:
      1. ./m4b-merge -i test-inputs/ -o output.m4b --skip-api
      2. ffprobe -v error output.m4b
      3. Assert: exit code 0
      4. ffprobe -show_entries format=duration -of csv=p=0 output.m4b
      5. Assert: duration matches expected
    Expected Result: Valid M4B file created with correct duration
    Evidence: .sisyphus/evidence/task-4-merge-test.m4b
  ```

  **Commit**: YES
  - Message: `feat: implement audio merging and conversion`

---

- [x] 5. Configuration and CLI Parsing

  **What to do**:
  - Implement CLI with clap derive macros
  - Support all existing arguments:
    - `-i, --inputs`: Input files/directories
    - `-o, --output`: Output directory
    - `--api_url`: Audnexus API URL
    - `--completed_directory`: Move source files here
    - `--num_cpus`: Parallel processing limit
    - `--log_level`: Logging verbosity
    - `-p, --path_format`: Output naming template
  - Implement path_format template parser (author, narrator, series_name, etc.)
  - Add configuration file support (optional TOML config)
  - Implement default value handling

  **Must NOT do**:
  - Don't add new CLI arguments beyond existing ones
  - Don't implement interactive prompts yet

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 2)
  - **Parallel Group**: Wave 1
  - **Blocks**: Task 7
  - **Blocked By**: Task 1

  **References**:
  - Current Python: `src/m4b_merge/__main__.py` - CLI argument definitions
  - clap derive docs: https://docs.rs/clap/latest/clap/_derive/index.html
  - Current Python path_format: supports {author}, {narrator}, {series_name}, {series_position}, {subtitle}, {title}, {year}

  **Acceptance Criteria**:
  - [x] All existing CLI arguments supported
  - [x] Help text matches Python version
  - [x] Path format templates work correctly
  - [x] Invalid arguments produce helpful errors

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: CLI accepts all arguments
    Tool: Bash
    Preconditions: Binary built
    Steps:
      1. ./m4b-merge --help
      2. Assert: help text contains all expected arguments
      3. ./m4b-merge -i test.mp3 -o /output --dry-run
      4. Assert: dry-run completes without error
    Expected Result: All CLI arguments recognized
    Evidence: .sisyphus/evidence/task-5-cli-help.txt
  ```

  **Commit**: YES
  - Message: `feat: implement CLI argument parsing`

---

- [x] 6. Metadata Structures and API Client

  **What to do**:
  - Define structs for audio metadata:
    ```rust
    struct AudioMetadata {
        title: String,
        subtitle: Option<String>,
        authors: Vec<String>,
        narrators: Vec<String>,
        series_name: Option<String>,
        series_position: Option<String>,
        description: String,
        genres: Vec<String>,
        year: Option<u32>,
        asin: Option<String>,
        cover_url: Option<String>,
        chapters: Vec<Chapter>,
    }
    ```
  - Implement audnexus API client with reqwest
  - Support ASIN lookup endpoint
  - Handle API errors (404, rate limiting, timeouts)
  - Parse API response JSON into structs
  - Implement retry logic with exponential backoff

  **Must NOT do**:
  - Don't cache API responses yet (post-MVP)
  - Don't add parallel API requests (sequential is fine)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 3)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 7
  - **Blocked By**: Task 1

  **References**:
  - Current Python: `src/m4b_merge/audible_helper.py` - API integration
  - audnexus API docs: https://audnex.us/
  - reqwest docs: https://docs.rs/reqwest/latest/reqwest/

  **Acceptance Criteria**:
  - [x] Can fetch metadata for valid ASIN
  - [x] Handles API errors gracefully
  - [x] Retries on transient failures
  - [x] Parses all expected fields from response

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: API client fetches metadata
    Tool: Bash
    Preconditions: Internet connection, valid test ASIN
    Steps:
      1. ./m4b-merge -i test.mp3 --asin B08XYZ123 --dry-run
      2. Assert: API call succeeds
      3. Assert: metadata displayed in logs
    Expected Result: Metadata fetched from audnexus
    Evidence: .sisyphus/evidence/task-6-api-fetch.log
  ```

  **Commit**: YES
  - Message: `feat: add audnexus API client and metadata structures`

---

- [x] 7. Tagging and File Operations

  **What to do**:
  - Implement metadata tagging using mp4ameta
  - Write standard tags: title, artist, album, genre, year, comment
  - Write extended tags: narrator, series info, ASIN
  - Download and embed cover art
  - Write chapters.txt file alongside output
  - Implement file moving to completed_directory
  - Handle file overwrites (configurable behavior)

  **Must NOT do**:
  - Don't add tone integration yet (FFmpeg-only for now)
  - Don't add complex path sanitization beyond current behavior

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on merge, config, metadata)
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 9
  - **Blocked By**: Tasks 4, 5, 6

  **References**:
  - mp4ameta docs: https://docs.rs/mp4ameta/latest/mp4ameta/
  - Current Python: `src/m4b_merge/m4b_helper.py:write_metadata`
  - Current Python: `src/m4b_merge/m4b_helper.py:move_completed_files`

  **Acceptance Criteria**:
  - [x] Tags written to output M4B
  - [x] Cover art embedded
  - [x] chapters.txt written
  - [x] Source files moved to completed_directory
  - [x] Output files pass mutagen verification

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Tags written correctly
    Tool: Bash
    Preconditions: Merged M4B file exists
    Steps:
      1. ffprobe -show_entries format_tags -of json output.m4b
      2. Assert: tags contain title, artist, album
      3. python3 -c "import mutagen; print(mutagen.File('output.m4b').tags)"
      4. Assert: all expected tags present
    Expected Result: Metadata correctly embedded
    Evidence: .sisyphus/evidence/task-7-tags.json
  ```

  **Commit**: YES
  - Message: `feat: implement tagging and file operations`

---

- [x] 8. Chapter Handling Proof of Concept

  **What to do**:
  - **CRITICAL: Build POC before implementing in main codebase**
  - Create separate `chapter-poc/` directory
  - Implement reading chapters from M4B using mp4ameta
  - Implement writing chapters to M4B (both QuickTime and Nero formats)
  - Test chapter compatibility with:
    - iTunes/Apple Books
    - VLC
    - Common Android audiobook apps
  - Document chapter format differences
  - If mp4ameta insufficient, research alternatives:
    - Direct atom manipulation
    - FFmpeg chapter injection
    - Different crate

  **Must NOT do**:
  - Don't integrate into main code until POC validated
  - Don't spend more than 2-3 days on POC

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
  - **Reason**: Complex MP4 atom manipulation, requires deep technical investigation

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 3, 6)
  - **Parallel Group**: Wave 2
  - **Blocks**: Tasks 4, 7
  - **Blocked By**: Task 2

  **References**:
  - m4b-tool: `src/library/M4bTool/Chapter/ChapterHandler.php` - Chapter logic
  - MP4 chapter formats: https://github.com/wez/atomicparsley/blob/master/src/CHAPTER.md
  - mp4ameta chapter example: https://github.com/11Tuvork28/mp4ameta/blob/main/examples/chapters.rs

  **Acceptance Criteria**:
  - [x] POC can read chapters from existing M4B
  - [x] POC can write chapters readable by iTunes
  - [x] POC can write chapters readable by VLC
  - [x] Chapter timestamps accurate to millisecond

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Chapter POC validates approach
    Tool: Bash
    Preconditions: Test M4B with known chapters
    Steps:
      1. cd chapter-poc && cargo run -- read test.m4b
      2. Assert: outputs chapter list
      3. cargo run -- write input.m4b chapters.txt output.m4b
      4. ffprobe -show_chapters output.m4b
      5. Assert: chapters detected
      6. Open in iTunes/VLC and verify
    Expected Result: Chapters work in major players
    Evidence: .sisyphus/evidence/task-8-chapter-poc/
  ```

  **Commit**: YES (as separate POC commit)
  - Message: `poc: chapter handling proof of concept`

---

- [x] 9. Integration and End-to-End Testing

  **What to do**:
  - Wire all modules together in main.rs
  - Implement full workflow: discovery → API → merge → tag → move
  - Create integration test suite
  - Generate test audio files (use FFmpeg)
  - Create test fixtures with known outputs
  - Verify output matches Python version (byte-level not required, metadata must match)
  - Add error handling and recovery
  - Implement logging with tracing

  **Must NOT do**:
  - Don't aim for 100% test coverage (focus on critical paths)
  - Don't optimize performance yet (correctness first)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (integration task)
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 10
  - **Blocked By**: Task 7

  **References**:
  - Current Python tests: `tests/` directory
  - Rust testing book: https://doc.rust-lang.org/book/ch11-00-testing.html

  **Acceptance Criteria**:
  - [x] Full workflow runs end-to-end
  - [x] Integration tests pass
  - [x] Error cases handled gracefully
  - [x] Logging useful for debugging

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: End-to-end workflow
    Tool: Bash
    Preconditions: Test inputs prepared
    Steps:
      1. ./m4b-merge -i test-inputs/ -o output/ --completed-directory done/
      2. Assert: output/Author/Book/Book.m4b exists
      3. Assert: done/ contains original files
      4. ffprobe output file and verify metadata
    Expected Result: Complete workflow succeeds
    Evidence: .sisyphus/evidence/task-9-e2e/
  ```

  **Commit**: YES
  - Message: `feat: integrate all modules and add tests`

---

- [x] 10. Docker and CI/CD

  **What to do**:
  - Create Dockerfile with:
    - Static Rust binary build stage
    - Runtime stage with ffmpeg installed
    - Non-root user for security
  - Set up GitHub Actions workflow:
    - Build and test on PR
    - Build release binaries for Linux/macOS/Windows
    - Build and push Docker image
  - Add cross-compilation support
  - Create release automation

  **Must NOT do**:
  - Don't optimize Docker image size yet (functionality first)
  - Don't add complex deployment pipelines

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: ["axonhub-docker"]

  **Parallelization**:
  - **Can Run In Parallel**: NO (final task)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Task 9

  **References**:
  - Current Docker: `Dockerfile` - Reference structure
  - Rust in Docker: https://hub.docker.com/_/rust
  - GitHub Actions Rust: https://github.com/actions-rs

  **Acceptance Criteria**:
  - [x] Docker image builds successfully
  - [x] Docker image runs correctly
  - [x] CI passes on PR
  - [x] Release binaries built automatically

  **Agent-Executed QA Scenarios**:

  ```
  Scenario: Docker image works
    Tool: Bash
    Preconditions: Docker installed
    Steps:
      1. docker build -t m4b-merge:test .
      2. docker run -v $(pwd)/test-inputs:/input m4b-merge:test -i /input --help
      3. Assert: help text displayed
      4. docker run -v $(pwd)/test-inputs:/input -v $(pwd)/output:/output m4b-merge:test -i /input -o /output
      5. Assert: output files created
    Expected Result: Docker container runs correctly
    Evidence: .sisyphus/evidence/task-10-docker.log
  ```

  **Commit**: YES
  - Message: `ci: add Docker and GitHub Actions workflow`

---

## Commit Strategy

| After Task | Message | Files | Verification |
|------------|---------|-------|--------------|
| 1 | `chore: initial project scaffolding` | Cargo.toml, src/, .gitignore | cargo build |
| 2 | `feat: add FFmpeg discovery and basic wrappers` | src/ffmpeg.rs | cargo test ffmpeg |
| 3 | `feat: add audio file discovery and validation` | src/discovery.rs | cargo test discovery |
| 4 | `feat: implement audio merging and conversion` | src/merge.rs | cargo test merge |
| 5 | `feat: implement CLI argument parsing` | src/config.rs, src/main.rs | cargo test config |
| 6 | `feat: add audnexus API client and metadata structures` | src/api.rs, src/metadata.rs | cargo test api |
| 7 | `feat: implement tagging and file operations` | src/tagging.rs, src/files.rs | cargo test tagging |
| 8 | `poc: chapter handling proof of concept` | chapter-poc/ | Manual verification |
| 9 | `feat: integrate all modules and add tests` | src/main.rs, tests/ | cargo test |
| 10 | `ci: add Docker and GitHub Actions workflow` | Dockerfile, .github/ | docker build |

---

## Success Criteria

### Verification Commands

```bash
# Build
cargo build --release

# Tests
cargo test

# Clippy
cargo clippy -- -D warnings

# Format check
cargo fmt -- --check

# Binary help
./target/release/m4b-merge --help

# FFmpeg check
./target/release/m4b-merge --check-ffmpeg

# Docker
docker build -t m4b-merge:test .
docker run m4b-merge:test --help
```

### Final Checklist
- [x] All "Must Have" features implemented
- [x] All "Must NOT Have" items excluded
- [x] Integration tests pass
- [x] Docker image builds and runs
- [x] Output files compatible with Python version
- [x] Documentation complete
- [x] CI/CD pipeline working
