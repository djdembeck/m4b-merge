# Migration Learnings

## 2025-02-09: m4b-tool to tone Migration

### Key Technical Decisions

1. **Hybrid Approach Chosen**
   - Keep m4b-tool for merge operations (tone lacks merge/split)
   - Use tone for metadata tagging (where it excels)
   - Fallback mechanism ensures backward compatibility

2. **mp4v2-utils Ubuntu 22.04+ Issue Resolved**
   - Made mp4chaps optional in config.py
   - Implemented mutagen-based chapter fallback
   - Docker unaffected (uses sandreas/mp4v2 image)

3. **Test Refactoring Strategy**
   - Replaced exact file size assertions with range checks (±10%)
   - Added mutagen metadata verification
   - Tests now verify functionality, not implementation details

### Command Mappings (m4b-tool → tone)

| m4b-tool | tone | Notes |
|----------|------|-------|
| --name | --meta-title | Title field |
| --album | --meta-album | Album field |
| --artist | --meta-artist | Narrator |
| --albumartist | --meta-album-artist | Author |
| --year | --meta-recording-date | Year/date |
| --description | --meta-description | Description |
| --series | --meta-movement-name | Series name |
| --series-part | --meta-part | Series position |
| --genre | --meta-genre | Genre |
| --comment | --meta-comment | Comment |
| --cover | --meta-cover-file | Cover image |

### Files Modified

1. config.py - Binary discovery (tone, optional mp4chaps)
2. m4b_helper.py - merge_single_aac() tone integration, mutagen chapters
3. Dockerfile - Added tone stage
4. tests/*.py - Range assertions, metadata verification
5. README.md, CHANGELOG.md - Documentation

### Verification Steps

- Module imports without mp4chaps/tone (graceful degradation)
- tone integration works when binary available
- mutagen chapter writing functional
- Docker image builds with all binaries
