# Rust Docker Build Results

## Build Summary
- **Date**: 2026-02-09
- **Image Tag**: `m4b-merge-rust:test`
- **Build Status**: ✅ SUCCESS

## Build Metrics
| Metric | Value |
|--------|-------|
| Total Build Time | ~45 seconds |
| Image Disk Usage | 757 MB |
| Image Content Size | 203 MB |
| Exit Code | 0 |

## Build Stages
1. **Base Images**: Pulled `rustlang/rust:nightly-slim` and `debian:trixie-slim`
2. **Builder Stage**: Installed build dependencies, fetched crates, compiled release binary
3. **Runtime Stage**: Installed FFmpeg, created user, copied binary

## Warnings
- 1 dead code warning in `src/audio/ffmpeg.rs:200` - field `extra` in `FFprobeStream` struct is never read
  - Severity: Low (warning only)
  - Impact: No functional issues

## Verification Results
- ✅ Image tagged correctly
- ✅ Image runs successfully
- ✅ `--help` displays properly
- ✅ Binary functional

## Help Output
```
A CLI tool which outputs consistently sorted, tagged, single m4b files

Usage: m4b-merge [OPTIONS]

Options:
  -i, --inputs <PATH>               Input files or directories to process (required)
  -o, --output <PATH>               Output directory for merged files
      --api_url <URL>               Audnexus API URL to use for metadata lookup [default: https://api.audnex.us]
      --completed_directory <PATH>  Directory path to move original input files to after processing
      --num_cpus <N>                Number of CPUs to use for parallel processing [default: 1]
      --log_level <LEVEL>           Set logging level (error, warn, info, debug, trace) [default: info]
  -p, --path_format <TEMPLATE>      Structure of output path/naming template [default: {author}/{title}]
  -a, --asin <ASIN>                 ASIN for metadata lookup (optional)
      --dry-run                     Show what would be done without actually doing it
      --check-ffmpeg                Check FFmpeg installation and display version information
  -h, --help                        Print help
  -V, --version                     Print version
```

## Notes
- Multi-stage build keeps runtime image small (203MB content)
- FFmpeg included for audio processing capabilities
- Non-root user configured for security