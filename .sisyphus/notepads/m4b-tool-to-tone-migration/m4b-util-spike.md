# m4b-util Spike Test Results

## Installation
```bash
pip install m4b-util
# Installs: m4b-util 2025.4.16
```

## Available Commands

1. **bind** - Convert folder of audio files to m4b (equivalent to m4b-tool merge)
2. **cover** - Manipulate cover images
3. **labels** - Convert between Audacity labels, FFMPEG metadata, and chapters
4. **split** - Split file into smaller pieces
5. **slide** - Slide chapter segments

## Command Comparison: m4b-tool merge vs m4b-util bind

### m4b-tool merge
```bash
m4b-tool merge \
  --output-file=output.m4b \
  --audio-bitrate=64000 \
  --audio-samplerate=44100 \
  --name="Title" \
  --artist="Narrator" \
  --albumartist="Author" \
  --cover=cover.jpg \
  input_folder/
```

### m4b-util bind
```bash
m4b-util bind \
  --output-dir=/output \
  --output-name="Author - Title.m4b" \
  --title="Title" \
  --author="Author" \
  --cover=cover.jpg \
  input_folder/
```

## Key Differences

| Feature | m4b-tool | m4b-util |
|---------|----------|----------|
| Bitrate control | ✅ --audio-bitrate | ❌ Not available |
| Samplerate control | ✅ --audio-samplerate | ❌ Not available |
| Multi-threading | ✅ --jobs | ❌ Not available |
| Chapter reindexing | ✅ --no-chapter-reindexing | ❌ Not applicable |
| Narrator field | ✅ --artist | ❌ Not available (only --author) |
| Subtitle support | ✅ --name combines title+subtitle | ❌ Not available |
| Series metadata | ✅ --series, --series-part | ❌ Not available |
| Genre | ✅ --genre | ❌ Not available |
| Description | ✅ --description | ❌ Not available |

## Assessment

**Status**: NOT a drop-in replacement for m4b-tool merge

**Limitations**:
1. No bitrate/samplerate control (critical for m4b-merge's mediainfo-based workflow)
2. No narrator field (only author)
3. No series, genre, or description metadata
4. No parallel processing
5. Limited metadata support compared to m4b-tool

**Recommendation**: 
- Continue using m4b-tool for merge operations
- m4b-util could be used for simple bindings where full metadata control isn't needed
- Not suitable for m4b-merge's current feature set

## Future Considerations

If tone adds merge/split functionality in the future (issue #86), it would be a better replacement than m4b-util due to tone's superior metadata handling.
