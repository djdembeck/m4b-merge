# =============================================================================
# Build stage - Compile the Rust binary (Alpine/musl, static binary)
# =============================================================================
FROM rust:1.96-alpine AS builder

WORKDIR /app

# Release identity is compiled into both CLI and API user-agent.
ARG VERSION
ARG GIT_COMMIT

# Install build dependencies (binutils provides strip for the single
# builder-side stripping of the shipped binary)
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    cmake \
    perl \
    binutils

# Copy the frozen source and lockfile; cache mounts below retain dependencies.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY build.rs ./build.rs

# Build with persistent BuildKit cache mounts so the cargo registry, git
# checkouts, and compiled artifacts survive across builds; unchanged
# dependencies are never refetched or recompiled. The git cache mount matters
# for the pinned mp4ameta patch in Cargo.toml: without it every image build
# re-clones that repository. sharing=locked is safe here (single buildx
# invocation).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    M4B_MERGE_VERSION="$VERSION" M4B_MERGE_SOURCE="$GIT_COMMIT" cargo build --release --locked && \
    cp target/release/m4b-merge /usr/local/bin/m4b-merge && \
    strip --strip-all /usr/local/bin/m4b-merge

# =============================================================================
# Runtime stage - Minimal Alpine image with FFmpeg
# =============================================================================
FROM alpine:3.24 AS runtime

# Accept the version arg for image metadata
ARG VERSION=dev
ARG GIT_COMMIT=unknown
LABEL org.opencontainers.image.version=$VERSION \
      org.opencontainers.image.revision=$GIT_COMMIT

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
