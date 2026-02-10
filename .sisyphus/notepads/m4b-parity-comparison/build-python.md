# Python Docker Image Build - Task 2

**Date:** 2026-02-09
**Status:** FAILED

## Build Command
```bash
docker build -t m4b-merge-python:test -f docker/Dockerfile .
```

## Build Output

### Warnings (non-blocking)
- JSONArgsRecommended: JSON arguments recommended for CMD
- FromAsCasing: 'as' and 'FROM' keywords' casing do not match (lines 1, 5, 6)
- LegacyKeyValueFormat: ENV statements using legacy format (lines 9, 10, 51, 52)

### Error (blocking)
```
ERROR: unable to select packages:
  gosu (no such package):
    required by: world[gosu]
```

**Location:** Dockerfile line 30-31
```dockerfile
RUN apk add --no-cache --update --upgrade --repository=http://dl-cdn.alpinelinux.org/alpine/edge/testing \
    gosu
```

## Root Cause
The `gosu` package is no longer available in the Alpine edge testing repository for Alpine 3.15. This is a known deprecation - Alpine Linux has moved `gosu` out of the testing repository and it may not be available on older stable releases like 3.15.

## Impact
- Image cannot be built
- All downstream tasks dependent on this image are blocked

## Resolution Required
The Dockerfile needs to be updated to either:
1. Use `su-exec` instead of `gosu` (Alpine's recommended replacement)
2. Use a newer Alpine version where `gosu` is available
3. Install `gosu` from an alternative source

**Note:** Per task constraints, Dockerfile was not modified. This issue requires plan owner intervention.