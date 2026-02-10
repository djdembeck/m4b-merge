# Fix gosu Deprecation - Learnings

## Summary
Replaced deprecated `gosu` with `su-exec` in Python Dockerfile and entrypoint.sh.

## Changes Made

### docker/Dockerfile (lines 29-31)
**Before:**
```dockerfile
# Add gosu for easy step-down from root
RUN apk add --no-cache --update --upgrade --repository=http://dl-cdn.alpinelinux.org/alpine/edge/testing \
    gosu
```

**After:**
```dockerfile
# Add su-exec for easy step-down from root (gosu deprecated in Alpine 3.15+)
RUN apk add --no-cache --update --upgrade \
    su-exec
```

### docker/entrypoint.sh (line 11)
**Before:**
```bash
gosu "$USER_ID":"$GROUP_ID" "$@"
```

**After:**
```bash
su-exec "$USER_ID":"$GROUP_ID" "$@"
```

## Key Findings
1. `gosu` was installed from Alpine's `edge/testing` repository, which is deprecated/removed in Alpine 3.15+
2. `su-exec` is available in the standard Alpine main repository (v3.15)
3. Both tools serve the same purpose: step down from root to a non-root user
4. `su-exec` syntax is identical to `gosu`: `su-exec UID:GID command`

## Verification
- su-exec package installed successfully: `su-exec (0.2-r1)`
- Binary tested and working: `su-exec 1000:1000 echo 'su-exec works'` ✓

## Notes
- The Dockerfile build failed later due to missing `requirements.txt` - this is a pre-existing issue unrelated to the su-exec change
- The su-exec installation step completed successfully