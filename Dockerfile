# =============================================================================
# Build stage - Compile the Rust binary
# =============================================================================
FROM rustlang/rust:nightly-slim AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy only Cargo files first for better layer caching
COPY Cargo.toml Cargo.lock ./

# Create the src directory structure to prevent build errors
RUN mkdir -p src && echo "// src lib.rs placeholder" > src/lib.rs

# Build dependencies
RUN cargo fetch --locked || cargo update

# Copy the rest of the source code
COPY . .

# Build the release binary
RUN cargo build --release --locked

# =============================================================================
# Runtime stage - Minimal image with FFmpeg
# =============================================================================
FROM debian:bookworm-slim AS runtime

# Install FFmpeg and other runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user
RUN groupadd --gid 1000 appgroup && \
    useradd --uid 1000 --gid appgroup --shell /bin/bash --create-home appuser

# Create necessary directories
RUN mkdir -p /input /output /config && \
    chown -R appuser:appgroup /input /output /config

# Copy the binary from builder
COPY --from=builder /app/target/release/m4b-merge /usr/local/bin/m4b-merge

# Set environment variables
ENV UID=1000
ENV GID=1000
ENV HOME=/home/appuser

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /home/appuser

# Default command
ENTRYPOINT ["m4b-merge"]