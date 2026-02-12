# m4b-merge

A blazing fast CLI tool for merging audiobook files into sorted, tagged M4B files.

## Features

- **Multi-format support** — Input MP3, M4A, or M4B files; output is always M4B
- **Smart merging** — Preserves original bitrate and sample rate
- **Metadata fetching** — Automatic metadata lookup from [Audnexus](https://github.com/laxamentumtech/audnexus) via ASIN
- **Chapter preservation** — Maintains chapter markers from source M4B files
- **Cover art** — Embeds high-resolution (2000x2000+) cover art
- **Customizable output** — Configurable path templates
- **Dry-run mode** — Preview operations without making changes
- **Parallel processing** — Process multiple files concurrently

## Requirements

- [FFmpeg](https://ffmpeg.org/) (must be in PATH)

## Installation

### From Source

```bash
cargo install m4b-merge
```

### Pre-built Binaries

Download the latest release for your platform from the [releases page](https://github.com/djdembeck/m4b-merge/releases).

### Docker

#### Quick Start

```bash
docker run --rm \
  -v /path/to/input:/input \
  -v /path/to/output:/output \
  ghcr.io/djdembeck/m4b-merge:latest \
  -i /input/file.mp3
```

#### Basic Docker Usage

```bash
# Run with input/output directories
docker run --rm \
  -v /my/audiobooks:/input \
  -v /my/merged:/output \
  ghcr.io/djdembeck/m4b-merge:latest \
  -i /input/book.mp3 \
  -o /output

# With ASIN for metadata
docker run --rm \
  -v /my/audiobooks:/input \
  -v /my/merged:/output \
  ghcr.io/djdembeck/m4b-merge:latest \
  -i /input/book.mp3 \
  -a B0123456789
```

#### Named Container (persistent volumes)

```bash
# Create container with named volumes
docker create --name m4b-merge \
  -v /my/audiobooks:/input \
  -v /my/merged:/output \
  ghcr.io/djdembeck/m4b-merge:latest

# Run commands
docker start m4b-merge -i /input/book.mp3 -a B0123456789
docker start m4b-merge -i /input/*.mp3
```

#### Running as Current User

By default, Docker runs as root. To preserve file ownership:

```bash
docker run --rm \
  -u $(id -u):$(id -g) \
  -v /path/to/input:/input \
  -v /path/to/output:/output \
  ghcr.io/djdembeck/m4b-merge:latest \
  -i /input/file.mp3
```

Or set UID/GID explicitly:

```bash
docker run --rm \
  -e UID=1000 \
  -e GID=1000 \
  -v /path/to/input:/input \
  -v /path/to/output:/output \
  ghcr.io/djdembeck/m4b-merge:latest \
  -i /input/file.mp3
```

#### Docker Volume Layout

```
/input   # Your source audio files (mp3, m4a, m4b)
/output  # Merged M4B files go here
```

#### Building Docker Image Locally

```bash
docker build -t m4b-merge:latest .
```

### Verify FFmpeg

```bash
m4b-merge --check-ffmpeg
```

## Usage

### Basic Usage

```bash
# Single file
m4b-merge -i input/file.mp3

# Multiple files
m4b-merge -i input/file1.mp3 input/file2.mp3

# Directory (processes all audio files)
m4b-merge -i input_folder/
```

### With Metadata Lookup

```bash
# Provide ASIN for automatic metadata fetch
m4b-merge -i input/file.mp3 -a B0123456789

# Custom Audnexus API endpoint
m4b-merge -i input/file.mp3 -a B0123456789 --api_url https://api.audnex.us
```

### Output Options

```bash
# Custom output directory
m4b-merge -i input/file.mp3 -o /path/to/output

# Custom path format template
m4b-merge -i input/file.mp3 -p "{author}/{title} - {series_name} {series_position}"

# Move completed files to directory
m4b-merge -i input/file.mp3 --completed_directory /path/to/done
```

### Other Options

```bash
# Dry run (preview what would happen)
m4b-merge -i input/file.mp3 --dry-run

# Use multiple CPUs
m4b-merge -i input/file.mp3 --num_cpus 4

# Verbose logging
m4b-merge -i input/file.mp3 --log-level debug
```

## Path Format Variables

| Variable | Description |
|----------|-------------|
| `{author}` | Author name |
| `{narrator}` | Narrator name |
| `{title}` | Book title |
| `{subtitle}` | Book subtitle |
| `{series_name}` | Series name |
| `{series_position}` | Series position number |
| `{year}` | Release year |

Default: `{author}/{title}`

## Exit Codes

| Code | Description |
|------|-------------|
| 0 | Success |
| 1 | Error (missing input, FFmpeg not found, processing failed) |

## Building

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests
cargo test
```

## License

MIT