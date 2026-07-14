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

# Create the src directory structure to prevent build errors
RUN mkdir -p src && echo "fn main() {}" > src/main.rs

# Build dependencies
RUN cargo fetch --locked

# Copy the rest of the source code
COPY . .

# Build the release binary
RUN cargo build --release --locked

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

# Copy the binary from builder
COPY --from=builder /app/target/release/m4b-merge /usr/local/bin/m4b-merge

# Set environment variables
ENV HOME=/home/appuser

# Switch to non-root user
USER appuser

# Set working directory
WORKDIR /home/appuser

# Default command
ENTRYPOINT ["m4b-merge"]