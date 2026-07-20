# m4b-merge

![License](https://img.shields.io/github/license/djdembeck/m4b-merge)
![CI](https://img.shields.io/github/actions/workflow/status/djdembeck/m4b-merge/ci.yml)

A blazing fast CLI tool for merging audiobook files into sorted, tagged M4B files.

m4b-merge takes split audio files (MP3, M4A, or M4B) and merges them into a single, consistently tagged M4B file. Originally written in Python and rewritten in Rust, it provides high-performance processing with zero-copy merging for files of the same format, ensuring the original bitrate and sample rate are preserved.

The tool automates metadata retrieval via ASIN lookup through the Audnexus API, embeds high-resolution cover art, and maintains chapter markers from source files.

- [Install](#install)
- [Usage](#usage)
- [Path Format Variables](#path-format-variables)
- [Exit Codes](#exit-codes)
- [Contributing](#contributing)
- [License](#license)

## Install

### Docker (Fastest Path)

Run without installation using the GHCR image.

```bash
docker run --rm \
  -v /path/to/input:/input \
  -v /path/to/output:/output \
  ghcr.io/djdembeck/m4b-merge:latest \
  -i /input/book_folder/ -o /output
```

### Pre-built Binaries

Download the latest release for your platform from the [releases page](https://github.com/djdembeck/m4b-merge/releases).

### From Source

Requires [FFmpeg](https://ffmpeg.org/) installed and available in your PATH.

```bash
cargo install --path .
```

## Usage

Basic CLI interaction. Use `m4b-merge --help` for all available flags.

### Basic Merging

Merge all audio files in a directory into a single M4B.

```bash
m4b-merge -i input/book_folder/
```

### Metadata & High-Resolution Covers

Provide an ASIN to automatically fetch metadata from Audnexus.

```bash
m4b-merge -i input/book_folder/ -a B012345678
```

### Custom Output & Organization

Specify a custom output directory and organization template.

```bash
m4b-merge -i input/book_folder/ \
  -o /my/library \
  -p "{author}/{series_name} {series_position} - {title}"
```

### Advanced Processing

Use `--dry-run` to preview operations, `--num-cpus` to control parallelism, and `--completed-directory` to move processed files.

```bash
m4b-merge -i input/book_folder/ \
  --dry-run \
  --num-cpus 8 \
  --completed-directory /path/to/done
```

### Verify Environment

Check FFmpeg installation and version.

```bash
m4b-merge --check-ffmpeg
```

## Path Format Variables

The output path is generated using the `-p` / `--path-format` template.

| Variable      | Description                      |
|---------------|----------------------------------|
| `{author}`   | Author name                     |
| `{narrator}`  | Narrator name                   |
| `{title}`    | Book title                      |
| `{subtitle}`  | Book subtitle                   |
| `{series_name}` | Series name                    |
| `{series_position}` | Series position number    |
| `{year}`     | Release year                    |

**Default:** `{author}/{title}`

## Exit Codes

| Code | Description                                                 |
|------|------------------------------------------------------------|
| `0`  | Success                                                     |
| `1`  | Error (missing input, FFmpeg not found, processing failed) |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on contributing to this project.

## License

GPL-3.0