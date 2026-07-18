# =============================================================================
# Build stage - Compile the Rust binary (Alpine/musl, static binary)
# =============================================================================
FROM rust:1.96-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    cmake \
    perl

# Copy only Cargo files first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Create stub sources (lib + bin) so cargo can compile *only* the dependencies.
# The real source is layered on top afterward; cargo then rebuilds just our crates.
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs

# Pre-build dependencies. BuildKit cache mounts persist the cargo registry and
# the build target across builds, so unchanged dependencies are never recached
# or recompiled. Sharing=locked is safe here (single buildx invocation).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release --locked

# Replace stubs with the real source code
RUN rm -rf src
COPY . .

# Final build: only the project crates recompile. Bump source mtimes after
# COPY so cargo's fingerprint (mtime-based) invalidates the project crate --
# the stub build recorded src/*.rs older than what COPY delivered, so without
# this cargo would skip the rebuild and ship the stub binary. Dependencies are
# untouched (they live in the cargo registry, not /app/src). Copy the produced
# binary out of the cache mount into the image layer for the runtime stage.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    find src -type f -exec touch {} + && \
    cargo build --release --locked && \
    cp target/release/m4b-merge /usr/local/bin/m4b-merge

# =============================================================================
# Runtime stage - Minimal Alpine image with FFmpeg
# =============================================================================
FROM alpine:3.22 AS runtime

# Install runtime dependencies
RUN apk add --no-cache ffmpeg ca-certificates

# Create non-root user
RUN addgroup -S appgroup && adduser -S -G appgroup appuser

# Create necessary directories
RUN mkdir -p /input /output /config && \
    chown -R appuser:appgroup /input /output /config

# Copy the binary from builder (copied out of the cache mount in the builder
# stage so it persists in the image layer)
COPY --from=builder /usr/local/bin/m4b-merge /usr/local/bin/m4b-merge

# Set environment variables
ENV HOME=/home/appuser

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /home/appuser

# Default command
ENTRYPOINT ["m4b-merge"]