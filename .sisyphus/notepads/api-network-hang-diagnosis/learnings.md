# API Network Hang Diagnosis - Learnings

## Problem
API calls appeared to hang when running inside Docker containers, while working fine from the host with curl.

## Diagnosis Process

### Step 1: Added Detailed Tracing
- Instrumented `fetch_book_once()` with trace logs at every step
- Added logs for client creation, request building, sending, and response handling
- Rebuilt Docker image with diagnostic logging

### Step 2: Testing with Tracing
Ran the container with trace logging enabled (`RUST_LOG=info`) to observe the request flow.

## Root Cause Found

**The issue was NOT a network hang - it was SSL certificate verification failure.**

The container was missing the `ca-certificates` package, causing TLS handshake failures:

```
error:0A000086:SSL routines:tls_post_process_server_certificate:certificate verify failed
(unable to get local issuer certificate)
```

### Why It Appeared to Hang
The retry logic (3 retries with exponential backoff + jitter) made it seem like the request was hanging, when actually it was:
1. Attempting connection → SSL failure (fast)
2. Waiting 1s + jitter
3. Retrying → SSL failure (fast)
4. Waiting 2s + jitter
5. Retrying → SSL failure (fast)

This cycle looked like a hang to the user.

## Solution

Added `ca-certificates` to the Docker runtime stage:

```dockerfile
RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    libstdc++6 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean
```

## Verification

After the fix:
- Requests now succeed and receive HTTP responses from the API
- The 500 errors seen in testing were from the test ASIN not existing (expected behavior)
- Real ASINs now work correctly

## Key Insights

1. **DNS Resolution**: Working fine (api.audnex.us resolved)
2. **TCP Connection**: Working fine (connection established)
3. **TLS/SSL**: Failed due to missing CA certificates
4. **HTTP Layer**: Working fine after fix

## Prevention

Always include `ca-certificates` in minimal Docker images that make HTTPS requests.
Common pattern for Debian-based images:
```dockerfile
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
```
