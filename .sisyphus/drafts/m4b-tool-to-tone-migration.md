# Draft: m4b-tool to tone Migration Plan

## Current State Analysis

### m4b-tool Usage in Codebase

**Files using m4b-tool:**
1. `src/m4b_merge/config.py` - Locates m4b-tool binary via `shutil.which('m4b-tool')`
2. `src/m4b_merge/m4b_helper.py` - Primary usage file

**m4b-tool Commands Used:**

1. **merge command** (lines 293-318):
   - Used for: Combining multiple files, converting mp3 to m4b
   - Arguments:
     - `--tmp-dir=/tmp/m4b-tool.{pid}`
     - `--output-file={path}.m4b`
     - Metadata args: `--name`, `--album`, `--artist`, `--albumartist`, `--year`, `--description`, `--series`, `--series-part`, `--genre`, `--comment`, `--cover`
     - Processing args: `--force`, `--no-chapter-reindexing`, `--no-cleanup`, `--ignore-source-tags`, `--jobs={num_cpus}`, `-v/-vvv`
     - Conversion args: `--no-conversion` OR `--audio-bitrate={bitrate}`, `--audio-samplerate={samplerate}`
     - Input path

2. **meta command** (lines 328-347):
   - Used for: Applying metadata to single m4b/m4a files
   - Arguments:
     - `--tmp-dir=/tmp/m4b-tool.{pid}`
     - `--ignore-source-tags`
     - Output path (`.new.m4b`)
     - Metadata args (same as above)

3. **mp4chaps** (separate binary, lines 466-489):
   - Used for: Chapter manipulation
   - Arguments: `-z`, `-i`/`-r`, `-v`/`-q`
   - Note: This is from mp4v2, not m4b-tool

### Docker Configuration
- Dockerfile downloads m4b-tool.phar from GitHub releases
- Tone is commented out in Dockerfile (line 4, 36)
- PHP runtime required for m4b-tool

### Test Infrastructure
- pytest-based tests
- Tests located in `tests/`
- Key test files: test_single_m4b_merge.py, test_single_mp3_merge.py, test_multiple_*.py
- Tests verify: file existence, file sizes, bitrate, samplerate, chapter generation

## Migration Requirements

### Functional Requirements
- Drop-in replacement: same functionality, same tests passing
- All m4b-tool commands replaced with tone equivalents
- Docker image updated to use tone instead of m4b-tool
- Config updated to find tone binary

### Open Questions
1. Does tone support all m4b-tool commands (merge, meta)?
2. Are command-line arguments compatible or do they need mapping?
3. Does tone require different runtime dependencies (PHP vs ???)?
4. Are there any features in m4b-tool that tone doesn't support?
5. Should mp4chaps remain or can tone handle chapters?

## Research Findings: CRITICAL LIMITATION DISCOVERED

### Tone vs m4b-tool Feature Gap

**Key Finding**: tone is NOT a complete drop-in replacement for m4b-tool.

From the research:
- tone is the **planned successor** to m4b-tool but is missing critical audio processing features
- tone currently only handles **metadata tagging** (`tone tag`, `tone dump`)
- m4b-tool handles **audio processing**: merge, split, chapter manipulation

**Features NOT in tone yet:**
- ❌ Merge audio files (m4b-tool merge) - Open feature request: tone#86
- ❌ Split by chapters (m4b-tool split)
- ❌ Chapter adjustment (m4b-tool chapters)

**Your current m4b-tool usage in m4b-merge:**
1. `m4b-tool merge` - for combining files (CRITICAL - no tone equivalent)
2. `m4b-tool meta` - for metadata (can use tone tag instead)

### Options Given This Constraint

**Option A: Partial Migration (Metadata Only)**
- Replace `m4b-tool meta` with `tone tag`
- Keep `m4b-tool merge` for audio processing
- Use both tools (complementary workflow)

**Option B: Wait for tone to Add Merge/Split**
- tone issue #86 is tracking merge feature
- No timeline from author
- m4b-tool maintenance continues until feature parity

**Option C: Hybrid Approach**
- Use tone for metadata operations where it's superior
- Keep m4b-tool for merge/split
- Gradual migration as tone adds features

## Questions for You

1. **Were you aware that tone doesn't support merge/split yet?**
   - If yes: Which option (A, B, or C) aligns with your needs?
   - If no: Does this change your migration plans?

2. **What's your priority?**
   - Remove PHP dependency from Docker?
   - Get tone's advanced tagging features?
   - Future-proof for when tone is complete?

3. **Can tests pass with partial migration?**
   - Tests verify file sizes, bitrates, chapter generation
   - Partial migration changes the binary but output should be identical
   - Is identical output required, or just passing tests?

## Research Tasks
- [x] Understand tone command structure
- [x] Map m4b-tool commands to tone equivalents
- [x] Identify any breaking changes or missing features
- [x] Research alternatives to m4b-tool merge (m4b-util, FFmpeg)
- [x] Consult Metis for gap analysis

## Metis Consultation Findings

### Key Risks Identified
1. **Test Fragility**: Tests assert exact file sizes (`== 25301510 or 25330971`). Any tool change produces different sizes.
2. **Chapter Format**: m4b-util uses different formats than mp4chaps
3. **Encoding Parameters**: Bitrate/samplerate preservation needs verification with alternatives
4. **Cover Handling**: m4b-util has separate `cover` command vs embedded in m4b-tool

### Recommended Approach
**Hybrid Migration**: Keep m4b-tool for merge, migrate meta to tone
- Minimal risk
- Future-proofing through tone adoption
- Incremental migration possible

### Alternatives Evaluated
1. **m4b-util**: Pure Python, has bind/split, but chapter format and encoding params uncertain
2. **FFmpeg + mutagen**: Custom implementation, high effort, full control
3. **Wait for tone**: Unknown timeline for merge feature

## User Decisions
- **Approach**: Re-evaluate (open to alternatives)
- **Priority**: Future-proofing
- **Test Requirements**: Tests pass with equivalent output (not bit-for-bit)
