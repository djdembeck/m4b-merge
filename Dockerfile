# =============================================================================
# Build stage - Compile the Rust binary (Alpine/musl, static binary)
# =============================================================================
FROM rust:1.96-alpine AS builder

WORKDIR /app

# Version stamp — passed by the release workflow as --build-arg VERSION=vX.Y.Z
# for image metadata labeling only. The binary version itself comes from the
# frozen source manifests (env!("CARGO_PKG_VERSION") in Cargo.toml).
ARG VERSION=devel

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    cmake \
    perl

# Copy the frozen source and lockfile; cache mounts below retain dependencies.
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build with persistent BuildKit cache mounts so the cargo registry and
# compiled artifacts survive across builds; unchanged dependencies are never
# recompiled.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release --locked && \
    cp target/release/m4b-merge /usr/local/bin/m4b-merge

# =============================================================================
# Runtime stage - Minimal Alpine image with FFmpeg
# =============================================================================
FROM alpine:3.22 AS runtime

# Accept the version arg for image metadata
ARG VERSION=devel
LABEL org.opencontainers.image.version=$VERSION

# Install runtime dependencies
RUN apk add --no-cache ffmpeg ca-certificates

# Create non-root user
RUN addgroup -S appgroup && adduser -S -G appgroup appuser

# Create necessary directories
RUN mkdir -p /input /output /config && \
    chown -R appuser:appgroup /input /output /config

# Copy the binary from the builder stage
COPY --from=builder /usr/local/bin/m4b-merge /usr/local/bin/m4b-merge

# Set environment variables
ENV HOME=/home/appuser
ENV APP_VERSION=$VERSION

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /home/appuser

# Default command
ENTRYPOINT ["m4b-merge"]
