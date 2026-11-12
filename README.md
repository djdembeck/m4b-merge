# m4b-merge

![License](https://img.shields.io/github/license/djdembeck/m4b-merge)
![CI](https://img.shields.io/github/actions/workflow/status/djdembeck/m4b-merge/ci.yml)

A blazing fast CLI tool for merging audiobook files into sorted, tagged M4B files.

m4b-merge takes split audio files (MP3, M4A, or M4B) and merges them into a single, consistently tagged M4B file. Originally written in Python and rewritten in Rust, it provides high-performance processing with zero-copy merging for files of the same format, ensuring the original bitrate and sample rate are preserved.

The tool automates metadata lookup by ASIN or AudiobookDB book ID through [AudiobookDB](https://audiobookdb.org) — a community-maintained audiobook metadata database with proper book/release separation and moderated contributions — embeds high-resolution cover art, and maintains chapter markers. The looked-up provider's chapters are preferred, the first input file's own chapters are used when the provider has none, and only when both are empty is one chapter per input track generated instead.

- [Install](#install)
- [Usage](#usage)
- [Path Format Variables](#path-format-variables)
- [Capabilities](#capabilities)
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
  `release_id` for `audiobookdb`). The catalog is the authority; an
  unresolvable id, a release the book does not carry, or a missing client
  fails the job before any file is touched.
- `input_supplied` — the values are authoritative as-is; no catalog lookup
  is attempted and the job runs fully offline.

On top of the base, an optional `overrides` object replaces individual
fields; present non-blank values win over the base.

```json
{
  "schema_version": 1,
  "inputs": ["/data/book/track1.m4a"],
  "input_supplied": {
    "title": "The Great Gatsby",
    "authors": ["F. Scott Fitzgerald"]
  },
  "output_root": "/library",
  "path_format": "{author}/{title}",
  "title": "write",
  "subtitle": "preserve",
  "authors": "write",
  "year": "clear",
  "cover": "write_url",
  "chapters": "write"
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
- `overrides` (optional) — same shape as `input_supplied`; present
  non-blank values win over the base.
- `title` / `subtitle` — `preserve` (default), `clear`, or `write`. A `write`
  writes the effective metadata's value (the title, and the subtitle when one
  is present) — the same strings that name the output path, so the tag cannot
  diverge from the file name — and fails the job if that value is blank.
  Title and subtitle share the `©nam`/`©alb` atom pair: a write of one
  component preserves the other, and `clear` of one removes only that
  component. `preserve` recovers the component from the existing tags; a file
  whose tags cannot establish it fails before mutation rather than erasing
  data.
- `authors`, `narrators`, `series`, `description`, `genres`, `year` —
  `preserve` (default), `write`, or `clear`. A `write` requires the effective
  value to be non-blank, and fails the job if it is not. `preserve` fields
  keep the file's existing values even when the effective metadata lacks them.
- `cover` — `write_url` (default; embeds the provider cover URL when the
  resolved metadata carries one — the legacy baseline), `{"write":
  "/absolute/path"}` (local file; the path must lie inside the job's
  `output_root`, which is the only asset area the job owns, and must be at
  most 5 MiB — a path that escapes the root, including via a symlink, is
  rejected before any read), `{"write_url": "https://…"}` (override, under
  the existing SSRF guards), `preserve` (single input only; keeps the source
  M4B's embedded cover without a download), or `clear` (removes the cover).
  `preserve` on a multi-file group is rejected, because a merge carries no
  cover and a strict preservation promise is impossible.
- `chapters` — `write` (default; embeds chapters and writes the
  `chapters.txt` sidecar: an explicit
  `{"write": {"chapters": [{"title", "start_millis", "duration_millis"}]}}`
  list when given, otherwise the resolved chapters; an empty explicit list
  clears, while a list that resolved to nothing leaves the source chapters
  untouched), `preserve` (single input only; skips all chapter mutation so
  the source chapters survive), or `clear` (removes existing chapters, no
  sidecar). A `preserve` chapters policy on a multi-file group is rejected; a
  strict preserve promise can only be made for a single input.

The output path is `output_root` + the (sanitized) template, so the same job
against the same book always lands on the same path — no worker-time catalog
rematching is needed by callers.

#### Preview (`--job-preview`)

A non-mutating preview of a job:

```bash
m4b-merge --job job.json --job-preview
```

It resolves the effective metadata exactly as execution does (base +
overrides; a supplied job performs **no catalog request**) and the same
parsed template with the same resolver, printing the exact sanitized
relative output path (plus the effective metadata) as JSON. It creates no
output or completed directories, writes no media, and reserves no names, and
makes no execution success promise: collisions, media errors, and filesystem
failures are execution-time. The previewed path is the path execution
produces, provided the metadata is the same — so a **provider** preview that
refetches at execution is **advisory** unless the caller captures the
supplied resolved metadata (supplied mode has no fetch, so its preview is
exact).

#### Machine results (`--machine`)

Machine-readable execution results for a job:

```bash
m4b-merge --job job.json --machine
```

Stable JSON on **stdout only** — all logs and progress go to stderr, so the
document parses cleanly on success and on failure:

```json
{ "status": "succeeded | partial_failure | failed",
  "groups": [{ "group": "job", "outcome": "succeeded", "input_files": ["track1.m4b"],
    "artifacts": [ { "relative_path": "Author/Title.m4b", "kind": "audio", "exists": true },
                    { "relative_path": "Author/Title.chapters.txt", "kind": "sidecar", "exists": true } ] } ] }
```

Every produced output and sidecar is enumerated; artifact paths are
**relative to the job's `output_root`** and validated to lie inside it (no
absolute-path fallback). `input_files` are the job's input paths, not
artifacts. `status` never claims success for a failed group; a partial or
total failure exits 1 after the document is printed, and setup/run-level
failures (missing FFmpeg, discovery failure) emit a `failed` document with a
top-level `error` and empty `groups`. A **job run is one book**: one group
named `"job"`, so `partial_failure` cannot occur from `--machine --job`; it
is reserved for multi-group library consumers of `Processor::run`.

#### No-clobber

Job execution claims the exact resolved output name with
`O_CREAT|O_EXCL` before any write: no silent suffixing, no
exists-then-rename race, and no overwrite rename. A pre-existing file or
directory at the path fails the run with a diagnosed collision, untouched; a
failed group leaves no falsely successful partial output, removes its claimed
placeholder on merge failure, and never destroys its source files. Created
parent directories are re-validated against the output root at claim time,
and the produced file is re-validated after the merge: it must be a regular,
non-empty file inside the canonical output root, so a swapped symlinked
parent cannot redirect the accepted output outside the root.
 This protects
the job's staging area only; publishing into a shared destination
still needs the caller's race-safe publication.

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

When a job file drives processing, only the fields whose policy is `write` or
`clear` are touched; `preserve` fields keep the file's existing values even
when the effective metadata lacks them.

## Path Format Variables

The output path is generated from a template. Two template conventions exist:

- **Legacy CLI** (`-p` / `--path-format`): the historical string-replacement
  behavior. The template is expanded, then split on `/`, and each resulting
  piece is sanitized.
- **Job file** (`path_format` on `--job`): a parsed-template resolver
  (identity: `resolver_version` from `--capabilities`). Only template
  separators create directories; a token's value is substituted as component
  data and sanitized in place, so a value can never introduce a directory,
  re-substitute another token, traverse (`..`), address a drive, or escape
  `output_root`.

| Variable      | Description                      |
|---------------|----------------------------------|
| `{author}`   | First ordered author name (empty: `Unknown`) |
| `{narrator}`  | First ordered narrator name (empty: `Unknown`) |
| `{authors}`   | All ordered authors, joined by `, ` (empty: `Unknown`) |
| `{narrators}` | All ordered narrators, joined by `, ` (empty: `Unknown`) |
| `{title}`    | Book title                      |
| `{subtitle}`  | Book subtitle                   |
| `{series_name}` | Series name                    |
| `{series_position}` | Series position number    |
| `{year}`     | Release year                    |
| `{genre}`      | First genre                     |
| `{metadata_id}` | Provider-dependent looked-up ID (ASIN for audnexus, resolved catalog ID for audiobookdb); empty in supplied mode |
| `{book_id}`   | Canonical AudiobookDB catalog book ID from the resolved metadata; populated only for AudiobookDB provider lookups, empty for audnexus and supplied mode. Distinct from `{metadata_id}`; job-resolver-only |

In the table above, `{book_id}` is a **job-resolver-only** token: the legacy
CLI `-p`/`--path-format` template does not substitute it (an unmatched
placeholder is left as literal text by the legacy path). All other tokens are
shared by both conventions.

A token whose value is missing or empty does not contribute a path segment
(empty segments are stripped). The only exceptions are `{author}`,
`{authors}`, `{narrator}`, and `{narrators}`, which substitute `Unknown` when
no person is known. Unknown tokens in a job `path_format` are rejected with
an actionable diagnostic (the token, its position, and the supported list)
before any file is touched. A job `path_format` that resolves to no
components at all (for example `{subtitle}` on a book with no subtitle) fails
the group with an `EmptyPath` error before any file is touched; the legacy CLI
path has no such case because it falls back to `output.m4b`.

**Default:** `{author}/{title}`

## Capabilities

`m4b-merge --capabilities` prints the build's machine-readable identity and
vocabulary as JSON: the accepted `schema_version`, the output-path
`resolver_version`, the default template, every supported path token with its
substitution semantics, and the field/cover/chapter policy vocabulary.
Callers build pickers, examples, and validation from this document instead of
maintaining their own registry, and reject protocol/resolver versions they do
not understand. The command is self-contained (no FFmpeg, job, or network).
The `--job-preview` and `--machine` surfaces carry the same
`schema_version`/`resolver_version` identity, so a caller can verify that a
path came from the resolver version it validated.

## Exit Codes

| Code | Description                                                                                                     |
|------|-----------------------------------------------------------------------------------------------------------------|
| `0`  | Success (in `--machine` mode: every group succeeded)                                                            |
| `1`  | Error (missing input, FFmpeg not found, processing failed; in `--machine` mode: partial or total failure)       |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on contributing to this project.

## License

GPL-3.0