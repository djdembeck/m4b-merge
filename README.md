# m4b-merge

![License](https://img.shields.io/github/license/djdembeck/m4b-merge)
![CI](https://img.shields.io/github/actions/workflow/status/djdembeck/m4b-merge/ci.yml)

A blazing fast CLI tool for merging audiobook files into sorted, tagged M4B files.

m4b-merge takes split audio files (MP3, M4A, or M4B) and merges them into a single, consistently tagged M4B file. Originally written in Python and rewritten in Rust, it provides high-performance processing with zero-copy merging for files of the same format, ensuring the original bitrate and sample rate are preserved.

The tool automates metadata lookup by ASIN or AudiobookDB book ID through [AudiobookDB](https://audiobookdb.org) — a community-maintained audiobook metadata database with proper book/release separation and moderated contributions — embeds high-resolution cover art, and maintains chapter markers. The looked-up provider's chapters are preferred, the first input file's own chapters are used when the provider has none, and only when both are empty is one chapter per input track generated instead.

- [Install](#install)
- [Usage](#usage)
- [Path Format Variables](#path-format-variables)
- [Exit Codes](#exit-codes)
- [Contributing](#contributing)
- [License](#license)

## Install

### Docker (Fastest Path)

Choose a published `vX.Y.Z` tag or digest from the [release you intend to run](https://github.com/djdembeck/m4b-merge/releases), and set `IMAGE` to its full GHCR reference. The workflow does not maintain a rolling Docker `latest` alias, so pin an explicit tag. GitHub's *Latest* release label is independent: it tracks the highest published stable version and is not advanced by lower-version delayed completions.

```bash
: "${IMAGE:?Set IMAGE to a published GHCR version tag or digest}"
docker run --rm \
  -v /path/to/input:/input \
  -v /path/to/output:/output \
  "$IMAGE" \
  -i /input/book_folder/ -o /output
```

### Pre-built Binaries

Download the release for your platform from the [releases page](https://github.com/djdembeck/m4b-merge/releases). Select the asset by its exact full name — never by the first asset containing `linux`:

| Asset | Platform | Linkage | Runtime prerequisites |
|---|---|---|---|
| `m4b-merge-linux` | Linux x86-64 | static musl | FFmpeg, CA roots, resolver files |
| `m4b-merge-linux-arm64` | Linux ARM64 | static musl | FFmpeg, CA roots, resolver files |
| `m4b-merge-macos` | macOS | — | FFmpeg (CA roots and name resolution come from the platform) |
| `m4b-merge-windows.exe` | Windows | — | FFmpeg (CA roots and name resolution come from the platform) |

From this release forward no GNU-linked Linux asset is produced. For
v2.0.0 only, `m4b-merge-linux` is the original GNU binary; the assets
added to that release are the static-musl x86-64 asset
`m4b-merge-linux-musl` and the ARM64 asset
`m4b-merge-linux-arm64`.

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

Provide a metadata ID (ASIN or AudiobookDB book ID) to automatically fetch metadata from [AudiobookDB](https://audiobookdb.org). (`--metadata-source audnexus` remains available for the legacy Audnexus API.)

```bash
m4b-merge -i input/book_folder/ -a B012345678
```

The looked-up edition is release-first: the selected release's title, subtitle,
and release date win when present, and the book-level title/subtitle are the
fallback (year falls back to the book copyright, then original publication; the
subtitle falls back to the book subtitle, then the book's disambiguation). This
applies to the legacy `--metadata-id` CLI path as well as the `--job`
`input_provider` path, so for reissued editions the embedded title and year are
the release's. An explicit `release_id` that the book does not carry is a
diagnosed failure — never a silent fall back to the book's first release.

Non-US Audible books need `--region` with `--metadata-source audnexus` (default `us`; accepted: `au ca de es fr in it jp us uk`).

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

### Authoritative Job File

Process a book from a versioned JSON job file. The job is the single source
of truth for the metadata base, the manual overrides, and the output; it is
mutually exclusive with `-i`, `-o`, `-p`, `-a`, and `--completed-directory`.

```bash
m4b-merge --job job.json
```

The job selects its metadata base exactly one of two ways:

- `input_provider` — an explicit catalog id (`book_id`, plus an exact
  `release_id` for `audiobookdb`). An unresolvable id, a release the book
  does not carry, or a missing client fails the job before any file is
  touched.
- `input_supplied` — the values are authoritative as-is; no catalog lookup
  is attempted and the job runs fully offline.

An optional `overrides` object replaces individual fields; present
non-blank values win over the base.

```json
{
  "schema_version": 1,
  "inputs": ["/data/book/track1.m4a"],
  "input_supplied": {
    "title": "The Great Gatsby",
    "authors": ["F. Scott Fitzgerald"]
  },
  "output_root": "/library",
  "path_format": "{author}/{title}"
}
```

Field semantics:

- `inputs` (required, non-empty) — the audio files to merge.
- `output_root` (required) — the output directory root; created if missing.
- `path_format` (optional) — output template inside the root; default
  `{author}/{title}` (see Path Format Variables).
- `source` (optional) — `{"source": "audiobookdb" | "audnexus",
  "api_url": "…", "region": "…"}`; defaults to the CLI defaults when omitted
  in provider mode, and is irrelevant in supplied mode.
- `completed_directory` (optional) — after successful processing, the source
  files are moved here. In job mode the CLI `--completed-directory` flag is
  rejected (the job file owns it), so omitting this field means the sources
  are not moved.
- `overrides` (optional) — same shape as `input_supplied`.

Every supported scalar metadata field is written from the effective
metadata; the provider cover URL (when present) is embedded, and the resolved
chapters are embedded with a `chapters.txt` sidecar.

### Verify Environment

Check FFmpeg installation and version.

```bash
m4b-merge --check-ffmpeg
```

## Embedded Tags

| Atom    | Source                                                        |
|---------|---------------------------------------------------------------|
| `©nam`  | Title; subtitle appended as `title - subtitle` when present   |
| `©ART`  | All authors, joined by `, `                                   |
| `©aART` | Same as `©ART` (album artist)                                 |
| `©alb`  | Base title only (never the subtitle)                           |
| `©gen`  | All genres, joined by `/` (single `©gen` value)                |
| `©day`  | Release year                                                  |
| `©cmt`  | Description                                                   |
| `©wrt`  | All narrators, joined by `, `                                 |
| `©grp`  | `Series: <name> #<position>`                                  |

A tag with no corresponding metadata is removed, so retagging from a different
source cannot leave stale values behind. The one exception is `©grp`: the group
is rebuilt only when the metadata carries a narrator or a series name; when it
carries neither, the existing group is left as is (on a rebuild, legacy
`Narrator:` and `Series:` segments are dropped, a new series entry is added
when series metadata is present, and unrelated segments are kept, so a group
holding only legacy content is cleared entirely).

## Path Format Variables

The output path is generated using the `-p` / `--path-format` template.

| Variable      | Description                      |
|---------------|----------------------------------|
| `{author}`   | Author name                     |
| `{narrator}`  | Narrator name                   |
| `{authors}`   | All authors, joined by `, ` (empty: `Unknown`) |
| `{narrators}` | All narrators, joined by `, ` (empty: `Unknown`) |
| `{title}`    | Book title                      |
| `{subtitle}`  | Book subtitle                   |
| `{series_name}` | Series name                    |
| `{series_position}` | Series position number    |
| `{year}`     | Release year                    |
| `{genre}`      | First genre                     |
| `{metadata_id}` | Looked-up metadata ID (ASIN or AudiobookDB book ID) |

A token whose value is missing or empty does not contribute a path segment (empty segments are stripped). The only exceptions are `{author}`, `{authors}`, `{narrator}`, and `{narrators}`, which substitute `Unknown` when no person is known.

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