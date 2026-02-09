# m4b-tool to tone Migration Plan

## TL;DR

> **Quick Summary**: Migrate from m4b-tool to a hybrid solution using tone for metadata while addressing mp4v2-utils removal from Ubuntu 22.04+. tone lacks merge/split functionality, so m4b-tool is kept for merge operations. The migration also replaces mp4chaps (from mp4v2-utils) with modern alternatives (mutagen/FFmpeg) for better compatibility.
> 
> **Key Infrastructure Concern**: mp4v2-utils was removed from Ubuntu 22.04+ repositories, affecting native installations. This plan addresses it by replacing mp4chaps dependency.
>
> **Deliverables**:
> - Updated `src/m4b_merge/config.py` to find tone binary
> - Updated `src/m4b_merge/m4b_helper.py` with migrated commands
> - Updated Docker image with tone
> - **Replaced mp4chaps with mutagen/FFmpeg** for Ubuntu 22.04+ compatibility
> - Refactored tests that assert exact file sizes
> - All existing tests passing with equivalent output
> 
> **Estimated Effort**: Medium (2-3 days, including mp4chaps replacement)
> **Parallel Execution**: YES - 2 waves
> **Critical Path**: Test fixes → Command migration → mp4chaps replacement → Docker updates → Full test suite
> **Development Branch**: `feature/migrate-to-tone` (all work on feature branch)

---

## Context

### Original Request
User wants to migrate from m4b-tool (https://github.com/sandreas/m4b-tool) to tone (https://github.com/sandreas/tone) with drop-in replacement functionality and passing tests.

### Interview Summary
**Key Discussions**:
- User was not aware that tone lacks merge/split functionality
- User wants to "re-evaluate approach" given this constraint
- Primary motivation: Future-proofing (tone is planned successor)
- Test requirement: Tests must pass with equivalent output (not bit-for-bit identical)

### Research Findings
**Critical Discovery**: tone is metadata-only and lacks merge/split commands. The author confirms m4b-tool will be maintained until tone reaches feature parity.

**Infrastructure Concern**: mp4v2-utils (provides mp4chaps) was removed from Ubuntu 22.04+ repositories. This affects:
- Native Ubuntu installations without Docker
- Future Docker base image updates
- Long-term maintainability

**m4b-merge Current Usage**:
- `m4b-tool merge` - Multiple files/single MP3 to m4b (lines 293-318, 360-389)
- `m4b-tool meta` - Single m4b/m4a metadata editing (lines 328-347)
- `mp4chaps` - Chapter management (lines 466-489) - **REQUIRES mp4v2-utils**

**Alternatives Researched**:
1. **Hybrid** (Recommended): Keep m4b-tool for merge, use tone for metadata
2. **m4b-util**: Python-based alternative with bind/split commands
3. **FFmpeg + mutagen**: Custom implementation
4. **Replace mp4chaps**: Use FFmpeg or mutagen for chapter handling (addresses Ubuntu concern)

### Metis Review
**Identified Gaps** (addressed in plan):
- Test fragility: Tests assert exact file sizes - need refactoring
- Chapter format: m4b-util uses different formats than mp4chaps
- Encoding params: Bitrate/samplerate preservation needs verification
- Cover handling: Different workflow in m4b-util vs m4b-tool
- **mp4v2 availability**: mp4v2-utils removed from Ubuntu 22.04+ - need mp4chaps replacement

---

## Work Objectives

### Core Objective
Migrate m4b-merge to use tone for metadata operations while maintaining merge functionality through either m4b-tool (hybrid) or m4b-util (pure Python).

### Concrete Deliverables
- [ ] `src/m4b_merge/config.py` - Updated to locate tone binary
- [ ] `src/m4b_merge/m4b_helper.py` - Migrated `meta` command to tone
- [ ] `docker/Dockerfile` - Updated with tone installation
- [ ] Test files - Refactored assertions for equivalent output (not exact sizes)
- [ ] All 5 test files passing: test_audible.py, test_single_m4b_merge.py, test_single_mp3_merge.py, test_multiple_m4b_merge.py, test_multiple_mp3_merge.py

### Definition of Done
- [ ] `pytest` runs without errors
- [ ] All tests pass with equivalent output
- [ ] Docker image builds successfully
- [ ] m4b-merge CLI works end-to-end with new tooling

### Must Have
- [ ] Merge functionality preserved (m4b-tool or m4b-util)
- [ ] Metadata editing migrated to tone
- [ ] Chapter handling working (with or without mp4chaps)
- [ ] Bitrate/samplerate detection preserved
- [ ] All tests passing
- [ ] **No dependency on mp4v2-utils for modern Ubuntu**

### Must NOT Have (Guardrails)
- [ ] Do NOT break existing CLI interface
- [ ] Do NOT remove m4b-tool merge until alternative is verified
- [ ] Do NOT change output file format/structure
- [ ] Avoid AI slop: Don't add unnecessary abstractions or over-engineer

### Development Workflow
- [ ] **Create feature branch**: `git checkout -b feature/migrate-to-tone`
- [ ] All commits made to feature branch (not main)
- [ ] Final merge via PR after all tests pass

---

## Verification Strategy

### Test Decision
- **Infrastructure exists**: YES (pytest)
- **Automated tests**: YES (existing tests must pass)
- **Framework**: pytest

**Test Refactoring Required**: Current tests assert exact file sizes (e.g., `os.path.getsize(output_path) == 25301510 or 25330971`). These must be updated to check file existence + rough size range or metadata correctness.

### Agent-Executed QA Scenarios (MANDATORY)

**Scenario 1: Single m4b metadata update**
Tool: Bash (pytest)
Preconditions: Test audio file exists at tests/media_files/test.m4b
Steps:
  1. Run: `pytest tests/test_single_m4b_merge.py::TestMerge::test_merge -v`
  2. Assert: Exit code 0
  3. Assert: Output contains "PASSED"
  4. Assert: Output file exists at ~/output/Andy Weir/Project Hail Mary/Project Hail Mary.m4b
Expected Result: Test passes, output file created with metadata
Evidence: pytest output captured

**Scenario 2: Single MP3 conversion**
Tool: Bash (pytest)
Preconditions: Test MP3 file exists
Steps:
  1. Run: `pytest tests/test_single_mp3_merge.py -v`
  2. Assert: Exit code 0
  3. Assert: All tests PASSED
  4. Assert: Output m4b file exists
Expected Result: All tests pass, MP3 converted to m4b
Evidence: pytest output captured

**Scenario 3: Multiple file merge**
Tool: Bash (pytest)
Preconditions: Test audio files in directory
Steps:
  1. Run: `pytest tests/test_multiple_m4b_merge.py tests/test_multiple_mp3_merge.py -v`
  2. Assert: Exit code 0
  3. Assert: All tests PASSED
Expected Result: Multiple files merged correctly
Evidence: pytest output captured

**Scenario 4: Docker build**
Tool: Bash
Preconditions: Docker installed
Steps:
  1. Run: `docker build -f docker/Dockerfile -t m4b-merge:test .`
  2. Assert: Exit code 0
  3. Run: `docker run --rm m4b-merge:test tone --version`
  4. Assert: tone version output displayed
Expected Result: Docker image builds and tone is available
Evidence: Docker build output captured

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 0 (Setup):
└── Task 0: Create feature branch `feature/migrate-to-tone` and verify baseline

Wave 1 (After Setup):
├── Task 1: Refactor tests to use equivalent output assertions
└── Task 2: Update config.py to find tone binary

Wave 2 (After Wave 1):
├── Task 3: Migrate m4b-tool meta command to tone
├── Task 4: Update Docker image with tone
└── Task 5: Verify m4b-util as merge alternative (optional spike)

Wave 3 (After Wave 2):
├── Task 6: Replace mp4chaps with modern alternative (CRITICAL for Ubuntu 22.04+)
└── Task 7: Run full test suite and fix any issues

Wave 4 (Final):
└── Task 8: Integration testing and documentation

Critical Path: Task 1 → Task 3 → Task 7 → Task 8

### Development Workflow
All work must be done on feature branch: `feature/migrate-to-tone`
```bash
# Start work
git checkout -b feature/migrate-to-tone

# Work through tasks, committing regularly
# ...

# Final merge via PR after all tests pass
git push origin feature/migrate-to-tone
# Create PR, merge to main after CI passes
```
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|------------|--------|---------------------|
| 1 | None | 3, 4, 5 | 2 |
| 2 | None | 3 | 1 |
| 3 | 1, 2 | 6, 7 | 4, 5 |
| 4 | 1 | 7 | 3, 5 |
| 5 | None | None | 1, 2, 3, 4 |
| 6 | 3 | 7 | 4 |
| 7 | 3, 4, 6 | 8 | None |
| 8 | 7 | None | None |

---

## TODOs

### Setup (Before Task 1)
- [ ] **0. Create feature branch and setup environment**
  
  **What to do**:
  - Create feature branch: `git checkout -b feature/migrate-to-tone`
  - Ensure clean working directory
  - Verify tests currently pass on main: `pytest tests/`
  - Setup development environment
  
  **Acceptance Criteria**:
  - [ ] On branch `feature/migrate-to-tone`
  - [ ] Clean git status
  - [ ] Baseline tests pass
  
  **Commit**: NO (branch setup)

- [ ] 1. Refactor tests to use equivalent output assertions

  **What to do**:
  - Update `tests/test_single_m4b_merge.py` - Change line 77 exact size assertion to file existence + size range
  - Update `tests/test_single_mp3_merge.py` - Similar size assertion changes
  - Update `tests/test_multiple_m4b_merge.py` - File size assertions
  - Update `tests/test_multiple_mp3_merge.py` - File size assertions
  - Add assertions for metadata correctness (use mutagen to verify tags)

  **Must NOT do**:
  - Do not remove existing test logic, only update assertions
  - Do not add new test dependencies without verification

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  - Reason: Simple test refactoring, no complex logic

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1
  - **Blocks**: Task 3
  - **Blocked By**: None

  **References**:
  - `tests/test_single_m4b_merge.py:76-77` - Current size assertion pattern
  - `tests/test_single_mp3_merge.py:76-77` - Same pattern
  - `tests/test_multiple_m4b_merge.py:82-83` - Multiple file test
  - `tests/test_multiple_mp3_merge.py:82-83` - MP3 multiple test

  **Acceptance Criteria**:
  - [ ] Tests no longer assert exact file sizes
  - [ ] Tests verify file existence and rough size (±10%)
  - [ ] Tests verify metadata correctness using mutagen
  - [ ] `pytest tests/` runs without assertion errors

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Refactored tests pass
    Tool: Bash (pytest)
    Preconditions: Code changes applied
    Steps:
      1. Run: pytest tests/test_single_m4b_merge.py -v
      2. Assert: Exit code 0
      3. Assert: "test_merge PASSED" in output
      4. Run: pytest tests/test_single_mp3_merge.py -v
      5. Assert: All tests PASSED
    Expected Result: All tests pass with new assertions
    Evidence: pytest output captured
  ```

  **Commit**: YES
  - Message: `test: refactor assertions for equivalent output instead of exact file sizes`
  - Files: `tests/test_*.py`

- [ ] 2. Update config.py to find tone binary

  **What to do**:
  - Add tone binary discovery alongside m4b-tool in `src/m4b_merge/config.py`
  - Add error handling if tone not found (or make optional during transition)
  - Keep m4b-tool discovery for now (hybrid approach)

  **Must NOT do**:
  - Do not remove m4b-tool discovery yet
  - Do not fail if tone not found (graceful fallback)

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  - Reason: Simple config addition

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1
  - **Blocks**: Task 3
  - **Blocked By**: None

  **References**:
  - `src/m4b_merge/config.py:16-29` - Current binary discovery pattern
  - tone documentation: Check if binary is named `tone` or `tone-x64`

  **Acceptance Criteria**:
  - [ ] `config.tone_bin` variable exists
  - [ ] Binary discovery uses `shutil.which('tone')`
  - [ ] Graceful handling if tone not found
  - [ ] Import succeeds without errors

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Config loads tone binary
    Tool: Bash (Python REPL)
    Preconditions: tone installed or mocked
    Steps:
      1. Run: python -c "from m4b_merge import config; print(config.tone_bin)"
      2. Assert: No ImportError raised
      3. Assert: config.tone_bin is either path or None (not AttributeError)
    Expected Result: Config module loads with tone support
    Evidence: Python output captured
  ```

  **Commit**: YES (group with Task 3)
  - Message: `feat: add tone binary discovery to config`
  - Files: `src/m4b_merge/config.py`

- [ ] 3. Migrate m4b-tool meta command to tone

  **What to do**:
  - Update `merge_single_aac()` in `m4b_helper.py` (lines 325-358)
  - Replace `m4b-tool meta` with `tone tag`
  - Map m4b-tool arguments to tone equivalents:
    - `--name` → `--meta-title` or `--title`
    - `--album` → `--meta-album` or `--album`
    - `--artist` → `--meta-artist` or `--artist`
    - `--albumartist` → `--meta-album-artist` or similar
    - `--year` → `--meta-year` or `--date`
    - `--description` → `--meta-description` or `--comment`
    - `--series` → check tone documentation
    - `--series-part` → check tone documentation
    - `--genre` → `--meta-genre`
    - `--comment` → `--meta-comment`
    - `--cover` → `--cover` or `--meta-cover`
  - Remove m4b-tool-specific flags that don't apply to tone

  **Must NOT do**:
  - Do not change merge commands (those stay as m4b-tool)
  - Do not change command structure/flow

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: []
  - Reason: Command mapping, requires testing

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Wave 1)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 6, 7
  - **Blocked By**: Task 1, 2

  **References**:
  - `src/m4b_merge/m4b_helper.py:325-358` - merge_single_aac method
  - `src/m4b_merge/m4b_helper.py:132-162` - metadata_args construction
  - tone README: Check exact flag names

  **Acceptance Criteria**:
  - [ ] `tone tag` command replaces `m4b-tool meta`
  - [ ] All metadata fields mapped correctly
  - [ ] Cover image embedding works
  - [ ] Command logged at debug level

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Metadata applied with tone
    Tool: Bash (pytest)
    Preconditions: Test file exists
    Steps:
      1. Run: pytest tests/test_single_m4b_merge.py::TestMerge::test_merge -v -s
      2. Assert: Exit code 0
      3. Assert: "tone" appears in debug output
      4. Verify: Output file has metadata using mutagen
    Expected Result: tone command executed, metadata applied
    Evidence: pytest output, mutagen verification
  ```

  **Commit**: YES
  - Message: `feat: migrate m4b-tool meta to tone tag for metadata operations`
  - Files: `src/m4b_merge/m4b_helper.py`

- [ ] 4. Update Docker image with tone and address mp4v2

  **What to do**:
  - Update `docker/Dockerfile` to install tone
  - Uncomment or add tone COPY/install (line 36)
  - Keep m4b-tool for now (hybrid approach)
  - **For mp4v2**: Keep using `sandreas/mp4v2:2.1.1` image in Docker
    - Docker builds are not affected by Ubuntu package removal
    - Multi-stage build already handles this correctly
  - Consider if mp4chaps can be removed from Docker after Task 6

  **Must NOT do**:
  - Do not remove m4b-tool from Docker yet
  - Do not break existing Docker builds
  - Do not switch to Ubuntu-provided mp4v2 (not available in 22.04+)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: []
  - Reason: Dockerfile modifications

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Wave 1)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 7
  - **Blocked By**: Task 1

  **References**:
  - `docker/Dockerfile:1-79` - Current Dockerfile
  - `docker/Dockerfile:5` - mp4v2 multi-stage build (works correctly)
  - `docker/Dockerfile:36` - Commented tone COPY line
  - `docker/Dockerfile:41-48` - m4b-tool download/install

  **Acceptance Criteria**:
  - [ ] Docker image builds successfully
  - [ ] tone binary available in container
  - [ ] m4b-tool still available in container
  - [ ] mp4chaps available (from sandreas/mp4v2 image)
  - [ ] Container entrypoint works

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Docker image has tone and mp4v2
    Tool: Bash (docker)
    Preconditions: Docker daemon running
    Steps:
      1. Run: docker build -f docker/Dockerfile -t m4b-merge:test .
      2. Assert: Build completes with exit code 0
      3. Run: docker run --rm m4b-merge:test tone --help
      4. Assert: tone help output displayed
      5. Run: docker run --rm m4b-merge:test m4b-tool --version
      6. Assert: m4b-tool version displayed
      7. Run: docker run --rm m4b-merge:test mp4chaps --help
      8. Assert: mp4chaps help displayed (from sandreas image)
    Expected Result: All binaries available, no Ubuntu package dependency
    Evidence: Docker build output, container output
  ```

  **Commit**: YES (group with Task 3)
  - Message: `docker: add tone to Docker image`
  - Files: `docker/Dockerfile`

- [ ] 5. Verify m4b-util as merge alternative (optional spike)

  **What to do**:
  - Install m4b-util: `pip install m4b-util`
  - Test `m4b-util bind` with sample files
  - Compare output with m4b-tool merge
  - Check chapter format compatibility
  - Verify bitrate/samplerate preservation
  - Document findings

  **Must NOT do**:
  - Do not commit m4b-util integration unless verified
  - Do not remove m4b-tool integration

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: []
  - Reason: Spike/research task

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Can run anytime
  - **Blocks**: None (spike only)
  - **Blocked By**: None

  **References**:
  - m4b-util PyPI: https://pypi.org/project/m4b-util/
  - `m4b-util bind` command documentation

  **Acceptance Criteria**:
  - [ ] m4b-util installed and working
  - [ ] Test merge produces valid m4b file
  - [ ] Output quality comparable to m4b-tool
  - [ ] Document: command differences, limitations

  **Agent-Executed QA Scenario**:
  ```
  Scenario: m4b-util spike test
    Tool: Bash
    Preconditions: Test audio files available
    Steps:
      1. Run: pip install m4b-util
      2. Run: m4b-util bind tests/media_files/ --title "Test" -e mp3
      3. Assert: Output m4b file created
      4. Verify: File plays correctly, metadata present
    Expected Result: m4b-util viable alternative documented
    Evidence: Command output, file verification
  ```

  **Commit**: NO (spike task, findings go to docs)

- [ ] 6. Replace mp4chaps with modern alternative

  **What to do**:
  - Address mp4v2-utils removal from Ubuntu 22.04+
  - Research alternatives:
    - **Option A**: Use `tone` for chapter metadata (if supported)
    - **Option B**: Use FFmpeg with ffmetadata format for chapters
    - **Option C**: Use Python `mutagen` library for MP4 chapter manipulation
    - **Option D**: Keep mp4chaps but build from source in Docker
  - Implement chosen solution in `fix_chapters()` method
  - Update chapter file format handling if needed
  - Test with Audible chapter data

  **Must NOT do**:
  - Do not break chapter handling on existing Docker builds
  - Do not require mp4v2-utils for Ubuntu 22.04+ users
  - Do not change chapter format without backward compatibility

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: []
  - Reason: Integration verification, requires testing alternatives

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 3)
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 7
  - **Blocked By**: Task 3

  **References**:
  - `src/m4b_merge/m4b_helper.py:431-489` - fix_chapters method
  - `src/m4b_merge/m4b_helper.py:466-489` - mp4chaps invocation
  - FFmpeg chapter documentation: https://ffmpeg.org/ffmpeg-formats.html#Metadata
  - mutagen MP4 chapter support: https://mutagen.readthedocs.io/en/latest/api/mp4.html
  - tone chapter support (if any)

  **Acceptance Criteria**:
  - [ ] Chapter handling works without mp4v2-utils on Ubuntu 22.04+
  - [ ] Docker build still works (can use mp4v2 from sandreas image)
  - [ ] Audible chapters imported correctly
  - [ ] Chapter file format compatible or migrated
  - [ ] Native pip installations work without external dependencies

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Chapter handling without mp4v2-utils
    Tool: Bash (pytest)
    Preconditions: Test files with chapters, no mp4chaps binary
    Steps:
      1. Verify: mp4chaps not in PATH (which mp4chaps returns nothing)
      2. Run: pytest tests/test_single_m4b_merge.py::TestMerge::test_chapter_generation -v
      3. Assert: Exit code 0
      4. Verify: Output has correct chapter count and titles
    Expected Result: Chapters work without mp4v2-utils
    Evidence: pytest output, chapter file content
  ```

  **Commit**: YES
  - Message: `feat: replace mp4chaps with mutagen/FFmpeg for Ubuntu 22.04+ compatibility`
  - Files: `src/m4b_merge/m4b_helper.py`, `requirements.txt` (if new deps)

- [ ] 7. Run full test suite and fix any issues

  **What to do**:
  - Run: `pytest tests/ -v`
  - Fix any failing tests
  - Address any integration issues
  - Verify all output files have correct metadata

  **Must NOT do**:
  - Do not skip failing tests
  - Do not modify tests to pass without fixing root cause

  **Recommended Agent Profile**:
  - **Category**: `unspecified-low`
  - **Skills**: []
  - Reason: Integration testing

  **Parallelization**:
  - **Can Run In Parallel**: NO (must be sequential)
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 8
  - **Blocked By**: Task 3, 4, 6

  **References**:
  - All test files in `tests/`
  - `pytest` configuration

  **Acceptance Criteria**:
  - [ ] `pytest tests/` exits with code 0
  - [ ] All test files pass
  - [ ] No warnings or errors in output

  **Agent-Executed QA Scenario**:
  ```
  Scenario: Full test suite passes
    Tool: Bash (pytest)
    Preconditions: All code changes applied
    Steps:
      1. Run: pytest tests/ -v --tb=short
      2. Assert: Exit code 0
      3. Assert: "passed" count equals total test count
      4. Assert: No "FAILED" or "ERROR" in output
    Expected Result: All tests pass
    Evidence: Full pytest output
  ```

  **Commit**: NO (fixes committed as part of respective tasks)

- [ ] 8. Integration testing and documentation

  **What to do**:
  - Test complete workflow: input → output
  - Test with Docker: `docker run m4b-merge -i /input/test.m4b`
  - Update README if commands changed
  - Document any breaking changes
  - Update CHANGELOG

  **Must NOT do**:
  - Do not skip documentation
  - Do not leave undocumented changes

  **Recommended Agent Profile**:
  - **Category**: `writing`
  - **Skills**: []
  - Reason: Documentation

  **Parallelization**:
  - **Can Run In Parallel**: NO (final task)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Task 7

  **References**:
  - `README.md` - Usage section
  - `CHANGELOG.md` - Version history
  - `docker/Dockerfile` - Build instructions

  **Acceptance Criteria**:
  - [ ] End-to-end workflow tested
  - [ ] README updated if needed
  - [ ] CHANGELOG updated
  - [ ] Docker workflow verified

  **Agent-Executed QA Scenario**:
  ```
  Scenario: End-to-end workflow
    Tool: Bash (docker)
    Preconditions: Docker image built
    Steps:
      1. Run: docker run -v $(pwd)/tests/media_files:/input m4b-merge:test m4b-merge -i /input/test.m4b
      2. Assert: Process completes without errors
      3. Assert: Output file created at expected location
      4. Verify: Output has correct metadata
    Expected Result: Complete workflow functional
    Evidence: Command output, file verification
  ```

  **Commit**: YES
  - Message: `docs: update README and CHANGELOG for tone migration`
  - Files: `README.md`, `CHANGELOG.md`

---

## Commit Strategy

| After Task | Message | Files | Verification |
|------------|---------|-------|--------------|
| 1 | `test: refactor assertions for equivalent output` | tests/*.py | pytest passes |
| 2+3+4 | `feat: migrate m4b-tool meta to tone` | config.py, m4b_helper.py, Dockerfile | pytest passes |
| 6 | `refactor: update chapter handling` | m4b_helper.py | chapter tests pass |
| 8 | `docs: update documentation` | README.md, CHANGELOG.md | - |

---

## Success Criteria

### Verification Commands
```bash
# Test suite
pytest tests/ -v
# Expected: All tests pass

# Docker build
docker build -f docker/Dockerfile -t m4b-merge:test .
# Expected: Build succeeds

# Binary verification
docker run --rm m4b-merge:test tone --version
docker run --rm m4b-merge:test m4b-tool --version
# Expected: Both show versions

# Integration test
docker run -v $(pwd)/tests/media_files:/input m4b-merge:test m4b-merge -i /input/test.m4b
# Expected: Process completes, output created
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] Docker image builds
- [ ] Documentation updated
