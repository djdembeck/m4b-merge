# Changelog

All notable changes to this project will be documented in this file. See [standard-version](https://github.com/conventional-changelog/standard-version) for commit guidelines.

## 1.0.0 (2026-07-20)

### Complete Rewrite: Python → Rust

m4b-merge has been rewritten from Python to Rust, delivering a single statically-linked binary that eliminates the Python runtime, pip dependencies, and external tool requirements (m4b-tool, mutagen, mp4chaps). The rewrite was developed over 6 days (Feb 9–14, 2026) across 137 development sessions and 76 commits, then hardened through 5 months of post-release iteration.

#### Why Rust

- **Zero runtime dependencies** — one binary, no virtualenv, no pip, no system packages beyond FFmpeg
- **Type safety** — compile-time guarantees replace Python's runtime assertion culture
- **Performance** — 11.7× faster single-file copy (1.2s vs 14s for 546 MB files) via native `std::fs::copy`
- **Deterministic builds** — reproducible binaries via Cargo.lock and SHA-pinned CI actions
- **Memory safety** — no GIL, no reference counting overhead, bounded allocations

#### Architecture

| Module | Crate | Purpose |
|---|---|---|
| CLI | clap (derive) | Declarative argument parsing with built-in help |
| HTTP | reqwest 0.13 + rustls | TLS via rustls (no OpenSSL dependency) |
| Async | tokio (rt-multi-thread) | Multi-threaded runtime for API requests and file I/O |
| MP4 metadata | mp4ameta 0.13 | Native M4B tag and chapter embedding |
| Discovery | walkdir + natord | Recursive file discovery with natural sorting |
| Logging | tracing + tracing-subscriber | Structured logging with env-filter |
| Errors | anyhow + thiserror | Typed error chains with context |

#### Feature Parity with Python

All Python features carried forward, with additions:

| Feature | Python | Rust | Notes |
|---|---|---|---|
| Multi-disc file grouping | ✅ | ✅ | CD1/Disc 1/CD 1 detection |
| Natural sort order | ✅ | ✅ | natord crate |
| Audible API metadata | ✅ | ✅ | Audnexus backend with retry logic |
| Path format templates | ✅ | ✅ | {author}, {title}, {narrator}, {series}, {asin} |
| M4A/M4B copy mode | ✅ | ✅ | 11.7× faster single-file via std::fs::copy |
| MP3→AAC transcode | ✅ | ✅ | Bitrate detection and matching |
| Cover art embedding | ✅ | ✅ | With 5 MB size guard |
| chapters.txt output | ✅ | ✅ | UTF-8 safe truncation at 255 chars |
| Chapter embedding (native) | ❌ | ✅ | **New** — mp4ameta 0.13 writes chapters into M4B |
| Chapter extraction from source | ❌ | ✅ | **New** — fallback when API has no chapters |
| Dry-run mode | ✅ | ✅ | With graceful missing-file handling |
| EXDEV file moves | ✅ | ✅ | Copy+remove fallback for cross-device |
| ASIN folder extraction | ✅ | ✅ | `[B012345678]` regex pattern |
| Multi-threaded merge | ✅ | ✅ | `num_cpus` propagated to FFmpeg |

#### Performance

| Operation | Python | Rust | Improvement |
|---|---|---|---|
| Single-file copy (546 MB) | ~14s (FFmpeg) | ~1.2s (fs::copy) | **11.7×** |
| Multi-file concat | FFmpeg concat demuxer | FFmpeg concat demuxer | Parity — MJPEG cover fix added |
| Startup overhead | Python interpreter + imports | Instant — single binary | Eliminated cold-start cost |
| Memory footprint | Variable (GC) | Bounded (static) | Predictable |
| Binary size | N/A (Python) | ~8 MB (static) | Self-contained |

Key optimizations:
- Native `std::fs::copy` for single-file Copy mode bypasses FFmpeg entirely
- Precompiled regex for disc number detection (no per-file compile)
- Cached ffprobe metadata avoids redundant subprocesses
- Minimal tokio features (`rt-multi-thread`, `macros`, `fs`, `io-util`) reduce binary bloat

#### Breaking Changes

- **CLI argument format**: snake_case to kebab-case
  - `--api_url` → `--api-url`
  - `--completed_directory` → `--completed-directory`
  - `--num_cpus` → `--num-cpus`
  - `--log_level` → `--log-level`
  - `--path_format` → `--path-format`
- **License correction**: Cargo.toml was set to MIT by accident during rewrite; corrected to GPL-3.0 (original project license)
- **Installation**: `pip install` replaced by `cargo install` or pre-built binaries
- **Docker**: Python 3.13 runtime replaced by static binary + FFmpeg base image

#### Post-Rewrite Hardening (Feb–Jul 2026)

- **Chapter embedding** — mp4ameta upgraded to v0.13 with `chapter_list_mut()` for native M4B chapter writing; full round-trip extraction → embedding
- **Python cleanup** — removed all Python artifacts (src/m4b_merge/*, tests/test_*.py, docker/*, root config): ~2,000 lines deleted
- **Test parity** — filled coverage gaps: multi-author/narrator parsing, bitrate extraction from real audio, long-duration chapter timestamps (25h+ offsets), single M4B integration
- **CI pipeline** — split into gated stages (lint → test → docker), SHA-pinned actions, Alpine migration, rustls-tls, cargo-audit, ARM64 QEMU builds
- **Docker** — multi-platform builds for Linux amd64/arm64, cached Rust deps via stub-source, multi-stage build with FFmpeg base
- **chpl atom parsing** — fixed to ISO 14496-12 spec (1-byte fields, guard atom_len < 8)
- **API resilience** — improved retry logic, connection timeouts (10s), pool idle timeout (30s)
- **reqwest 0.13** — upgraded from 0.11.27, default features disabled (json + rustls only)
- **Edition 2024** — migrated from Rust 2021 with MSRV 1.85

#### Test Coverage

- 89 unit tests passing
- 8 integration tests passing (3 with runtime FFmpeg guards)
- Clippy clean, rustfmt checked

#### Migration Guide

**From pip to cargo:**
```bash
# Before
pip install m4b-merge
m4b-merge --input /path/to/books

# After
cargo install --path .
m4b-merge --input /path/to/books
```

**From Docker:**
```bash
# Before (Python runtime)
docker run djdembeck/m4b-merge:latest --input /data

# After (static binary)
docker run djdembeck/m4b-merge:latest --input /data
```
## [Unreleased]

### Added
- Chapter embedding into M4B files using native mp4ameta library
- Chapters are now embedded directly into the M4B container, not just written to chapters.txt
- Full chapter metadata round-trip: extraction from source → embedding in output

### [0.5.3](https://github.com/djdembeck/m4b-merge/compare/v0.5.2...v0.5.3) (2024-08-07)

### Bug Fixes

  * If junk_dir is not set, do not perform post-process move ([4286a1c](https://github.com/djdembeck/m4b-merge/commit/4286a1ce7c50d56d5d9e22136cbdc292cd3d52e3))
  * Add --tmp-dir with os.pid to each m4b-tool invocation ([36689e8](https://github.com/djdembeck/m4b-merge/commit/36689e8f52ed7af7e3c70660501529d852dc482e))


### [0.5.2](https://github.com/djdembeck/m4b-merge/compare/v0.5.1...v0.5.2) (2023-04-27)


### Bug Fixes

* file_title not found, replaced with title ([3ec4d66](https://github.com/djdembeck/m4b-merge/commit/3ec4d661fd032836b374e277d2b947a170d16716))

### [0.5.1](https://github.com/djdembeck/m4b-merge/compare/v0.5.0...v0.5.1) (2023-02-24)


### Bug Fixes

* **merge:** :bug: incorrect dict key ([54d4a8b](https://github.com/djdembeck/m4b-merge/commit/54d4a8b259a0486ace02f69264aeacd7e224f26f))

## [0.5.0](https://github.com/djdembeck/m4b-merge/compare/v0.4.11...v0.5.0) (2023-02-24)


### Features

* **merge:** :sparkles: add support for `asin` as output path term ([87a3623](https://github.com/djdembeck/m4b-merge/commit/87a3623fd9799d5c7f30da34015b84b17eadb12d))

### [0.4.11](https://github.com/djdembeck/m4b-merge/compare/v0.4.10...v0.4.11) (2023-01-23)


### Bug Fixes

* write temporary covers to `input_path` ([#104](https://github.com/djdembeck/m4b-merge/issues/104)) ([7cfca92](https://github.com/djdembeck/m4b-merge/commit/7cfca92b61ad8f47a656418fb8385acc6625b0d9)), closes [#103](https://github.com/djdembeck/m4b-merge/issues/103)

### [0.4.10](https://github.com/djdembeck/m4b-merge/compare/v0.4.8...v0.4.10) (2022-09-21)


### Bug Fixes

* **merge:** :bug: properly fix moving completed input files ([f0f4ae9](https://github.com/djdembeck/m4b-merge/commit/f0f4ae9468796f13d6738cb4ba9592df9e858d74))

### [0.4.8](https://github.com/djdembeck/m4b-merge/compare/v0.4.7...v0.4.8) (2022-09-12)


### Features

* **merge:** :sparkles: use LOG_LEVEL from environment variable if available ([6779104](https://github.com/djdembeck/m4b-merge/commit/677910471c1ea88f272df29d1b5f0faf34e6b073))


### Bug Fixes

* **merge:** :ambulance: fix crash on single file in a folder ([a895b4d](https://github.com/djdembeck/m4b-merge/commit/a895b4de44f549068c4b010a3b4fb1a82d1750ad))
* **merge:** :bug: handle case where input has no `bit_rate` and/or `sample_rate` ([9e17fbd](https://github.com/djdembeck/m4b-merge/commit/9e17fbd7b58145461ca1cee422ab881e76415483))

### [0.4.7](https://github.com/djdembeck/m4b-merge/compare/v0.4.6...v0.4.7) (2022-02-28)


### Bug Fixes

* **docker:** :ambulance: also chown /config ([8e99393](https://github.com/djdembeck/m4b-merge/commit/8e993935e92cd2e49a10cd2abbec4cf394bbee83))
* **docker:** :bug: better startup permissions management ([3c4cef5](https://github.com/djdembeck/m4b-merge/commit/3c4cef567f185e2c690c043b2316c1e4439ed441))
* **merge:** :bug: cleanup find_extension process ([a37bfbe](https://github.com/djdembeck/m4b-merge/commit/a37bfbe96870774d35e3255813932f7ce2e7c518))
* **merge:** :bug: separate these into own functions so multi disc and single file both can pick up unknown extensions ([a8da6b5](https://github.com/djdembeck/m4b-merge/commit/a8da6b5ab3fe726057d4c9b18a7d486f5947990a))

### [0.4.6](https://github.com/djdembeck/m4b-merge/compare/v0.4.5...v0.4.6) (2022-02-07)

### [0.4.2](https://github.com/djdembeck/m4b-merge/compare/v0.4.1...v0.4.2) (2021-11-04)

### [0.4.5](https://github.com/djdembeck/m4b-merge/compare/v0.4.4...v0.4.5) (2021-12-06)


### Bug Fixes

* **merge:** :bug: handle api having no author or narrators ([3adac9b](https://github.com/djdembeck/m4b-merge/commit/3adac9bd66480e1b373f9a17946dbd6c355f1e9e))

### [0.4.4](https://github.com/djdembeck/m4b-merge/compare/v0.4.3...v0.4.4) (2021-11-26)


### Features

* **merge:** :sparkles: Allow specifying output naming convention ([8980308](https://github.com/djdembeck/m4b-merge/commit/89803080db9816b8a71b8ff2d1f5135c2199c4dc))


### Bug Fixes

* **merge:** :bug: don't create empty directory of file name ([cbd2297](https://github.com/djdembeck/m4b-merge/commit/cbd22973d137875a317d68dd444897f44ecb0830))
* **merge:** :bug: fix replace_tag replacing partial terms instead of full term ([7abea6f](https://github.com/djdembeck/m4b-merge/commit/7abea6fd5c08252e4413f42b83ca1ecff5a28479))

### [0.4.3](https://github.com/djdembeck/m4b-merge/compare/v0.4.1...v0.4.3) (2021-11-18)


### Features

* **merge:** :construction: better config 1: move user configurable options to arguments ([c2cd229](https://github.com/djdembeck/m4b-merge/commit/c2cd2292fc8d3b3d50511deaf404e3df487cfb86))


### Bug Fixes

* **audible:** :bug: fix double import config issue with api_url ([0e657fb](https://github.com/djdembeck/m4b-merge/commit/0e657fb0ae2a0a7d58dd53d72110d66e75dfef3b))
* **audible:** :bug: fix validate url ([36a357b](https://github.com/djdembeck/m4b-merge/commit/36a357bbfd030165c09a45e33baae17ee8c20d94))
* **audible:** :bug: pass url directly instead of importing config ([27f796f](https://github.com/djdembeck/m4b-merge/commit/27f796fb01f4d20bf9a12eafe7eb7fc5ff8430d6))
* **merge:** :ambulance: fix  inconsistent variable name ([51b9b94](https://github.com/djdembeck/m4b-merge/commit/51b9b94d1b96d073587a2cf760565cff479ab049))
* **merge:** :bug: fix asin validation before merge ([0d00c09](https://github.com/djdembeck/m4b-merge/commit/0d00c09d07322a34bd18d560e15bac333090bc67))
* **merge:** :bug: fix error when no cover exists ([b42b081](https://github.com/djdembeck/m4b-merge/commit/b42b081bdf28f4c526fedd8bd71870d8252481ea))
* **merge:** :bug: fix path comparison for junk dir ([a98c828](https://github.com/djdembeck/m4b-merge/commit/a98c8287069fbf90a075826848e2433225046992))

### [0.4.2](https://github.com/djdembeck/m4b-merge/compare/v0.4.1...v0.4.2) (2021-11-03)


### Bug Fixes

* **merge:** :bug: fix error when no cover exists ([b42b081](https://github.com/djdembeck/m4b-merge/commit/b42b081bdf28f4c526fedd8bd71870d8252481ea))

### [0.4.1](https://github.com/djdembeck/m4b-merge/compare/v0.3.5...v0.4.1) (2021-10-06)


### Bug Fixes

* **audible:** :bug: verify isAccurate exists before using it ([6f21eae](https://github.com/djdembeck/m4b-merge/commit/6f21eae6c343e14aafb1a4521444b1ad687c8184))
* **merge:** :bug: don't expect series position to exist ([cf41203](https://github.com/djdembeck/m4b-merge/commit/cf412030db3b9d2c67632f6ea1737c478bb3ad20))
* **merge:** :bug: set series_position to none if it doesn't exist ([3aaed08](https://github.com/djdembeck/m4b-merge/commit/3aaed08889f9585ad6b96a4a2f3434f7f0144f00))
