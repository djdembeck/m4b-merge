# Architectural Decisions

## Decision 1: Hybrid Migration (Not Full Replacement)

**Context**: tone lacks merge/split functionality

**Decision**: Keep m4b-tool for merge, use tone for metadata only

**Rationale**:
- tone is metadata-only (no merge/split commands)
- m4b-tool maintenance continues until tone reaches parity
- Minimizes risk while gaining tone benefits

## Decision 2: Optional mp4chaps with mutagen Fallback

**Context**: mp4v2-utils removed from Ubuntu 22.04+ repositories

**Decision**: Make mp4chaps optional, implement mutagen-based fallback

**Rationale**:
- Native Ubuntu installs shouldn't require external packages
- Docker builds still work (use sandreas/mp4v2 image)
- mutagen is pure Python, already a dependency

## Decision 3: Range-Based Test Assertions

**Context**: Tests asserted exact file sizes

**Decision**: Replace with range checks (±10%) plus metadata verification

**Rationale**:
- Different tools produce slightly different outputs
- Exact byte matching is fragile
- Metadata correctness matters more than exact file size
