use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use crate::metadata::BookMetadata;

/// Maximum number of bytes for chapter titles in embedded chapters.
///
/// The chpl (Nero chapter list) atom stores each title's length in a single
/// byte, so titles must be truncated on a UTF-8 char boundary before they are
/// handed to mp4ameta.
const CHAPTER_TITLE_MAX_BYTES: usize = 255;

/// Maximum number of chapters representable in a chpl (Nero chapter list) atom.
///
/// The atom stores the entry count in a single byte; larger counts wrap around.
pub(crate) const CHAPTER_LIST_MAX_CHAPTERS: usize = u8::MAX as usize;

/// Errors that can occur during tagging operations
#[derive(Error, Debug)]
pub enum TaggingError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("MP4 metadata error: {0}")]
    Mp4Meta(String),

    #[error("Cover download error: {0}")]
    CoverDownload(String),
    /// A local cover path fell outside the authorized asset area.
    #[error("local cover path escapes the asset root: {0}")]
    CoverPathOutsideAssetRoot(String),

    #[error("Invalid file format: {0}")]
    InvalidFormat(String),

    #[error("Cover conversion error: {0}")]
    CoverConversion(String),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("File move error: {0}")]
    FileMove(String),

    #[error("Duplicate chapter start times detected")]
    DuplicateChapterTimes,

    #[error(
        "Refusing to embed chapters for {path}: {count} chapter(s) start at or past the {playlength} playlength ({details})"
    )]
    ChaptersOutOfRange { path: PathBuf, count: usize, playlength: String, details: String },

    #[error("write policy for '{field}' has no valid value to write")]
    WriteWithoutValue { field: &'static str },

    #[error(
        "cannot preserve '{field}' for {path}: the preserved component cannot be established from the existing ©nam/©alb atoms; refusing to mutate. Use an explicit write with the known value."
    )]
    PreserveUnresolvable { field: &'static str, path: PathBuf },
}

/// Result type for tagging operations
pub type Result<T> = std::result::Result<T, TaggingError>;

/// Behavior when destination file already exists
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverwriteBehavior {
    /// Skip the file, don't overwrite
    Skip,
    /// Return an error
    #[default]
    Error,
    /// Overwrite the existing file
    Force,
}

/// Truncate a chapter title to the chpl atom's byte limit on a char boundary.
///
/// mp4ameta slices title bytes at `min(len, 255)` before writing the chpl atom,
/// which panics when that index falls inside a multi-byte UTF-8 character.
/// Passing an already-short-enough title keeps the dependency's slice at a char
/// boundary.
fn truncate_chapter_title(title: &str) -> String {
    if title.len() <= CHAPTER_TITLE_MAX_BYTES {
        return title.to_string();
    }

    // `str::floor_char_boundary` is not stable yet, so walk back to the nearest
    // boundary at or below the byte limit.
    let mut end = CHAPTER_TITLE_MAX_BYTES;
    while !title.is_char_boundary(end) {
        end -= 1;
    }

    title[..end].to_string()
}

/// Describe every chapter that starts at or beyond `file_duration`.
///
/// Returns `(count, details)` for the offending chapters — ready to drop into an
/// error message — or `None` when every chapter is in range. mp4ameta writes
/// start times verbatim, so a chapter at or beyond the file duration can only
/// become an unplayable entry.
pub(crate) fn describe_out_of_range_chapters(
    chapters: &[crate::metadata::Chapter],
    file_duration: Duration,
) -> Option<(usize, String)> {
    let out_of_range: Vec<&crate::metadata::Chapter> =
        chapters.iter().filter(|chapter| chapter.is_out_of_range(file_duration)).collect();
    if out_of_range.is_empty() {
        return None;
    }

    let details = out_of_range
        .iter()
        .map(|chapter| format!("'{}' at {}", chapter.title, format_duration(chapter.start_time)))
        .collect::<Vec<_>>()
        .join(", ");

    Some((out_of_range.len(), details))
}

/// The prefix of `chapters` that a Nero `chpl` atom can represent.
///
/// The atom stores its entry count in a single byte, so a longer list wraps the
/// count and readers see an empty or truncated chapter list. The QuickTime
/// chapter track has no such limit, so callers writing that representation must
/// use the full list rather than this prefix.
fn chpl_chapters(chapters: &[mp4ameta::Chapter]) -> &[mp4ameta::Chapter] {
    &chapters[..chapters.len().min(CHAPTER_LIST_MAX_CHAPTERS)]
}

/// Convert metadata chapters to mp4ameta chapters
///
/// Sorts chapters by start_time and converts each to mp4ameta::Chapter.
/// Titles are truncated to the chpl atom limit (255 bytes) on a char boundary.
/// The mp4ameta crate handles duration calculation internally based on chapter start times.
fn convert_chapters_for_embedding(chapters: &[crate::metadata::Chapter]) -> Vec<mp4ameta::Chapter> {
    let mut metadata_chapters = chapters.to_vec();

    // Sort by start_time
    metadata_chapters.sort_by_key(|c| c.start_time);

    let mut result = Vec::with_capacity(metadata_chapters.len());

    for chapter in metadata_chapters.iter() {
        let title = truncate_chapter_title(&chapter.title);
        result.push(mp4ameta::Chapter::new(chapter.start_time, &title));
    }

    result
}

/// Validate chapters before embedding
///
/// Sorts chapters by start_time and checks for duplicate start times.
/// Returns an error if duplicates are found.
fn validate_and_sort_chapters(chapters: &mut [crate::metadata::Chapter]) -> Result<()> {
    if chapters.is_empty() {
        return Ok(());
    }

    // Sort by start_time
    chapters.sort_by_key(|c| c.start_time);

    // Check for duplicates
    for i in 1..chapters.len() {
        if chapters[i].start_time == chapters[i - 1].start_time {
            return Err(TaggingError::DuplicateChapterTimes);
        }
    }

    Ok(())
}

/// Handles metadata tagging and file operations for audiobooks
#[derive(Debug, Clone)]
pub struct Tagger {
    /// Behavior when destination file exists
    overwrite_behavior: OverwriteBehavior,
}

impl Tagger {
    /// Create a new Tagger with default settings
    pub fn new() -> Self {
        Self { overwrite_behavior: OverwriteBehavior::default() }
    }

    /// Ensure a file exists, returning an error if not
    fn ensure_file_exists(path: &Path) -> Result<()> {
        if !path.exists() { Err(TaggingError::FileNotFound(path.to_path_buf())) } else { Ok(()) }
    }

    /// Fast text-level pre-check for a cover URL before downloading (SSRF guard).
    ///
    /// Only `https` URLs are accepted. Address literals that are loopback,
    /// private, link-local, CGN, or reserved (e.g. `127.0.0.1`, `10.x`,
    /// `192.168.x`, `169.254.169.254`, `::1`) are rejected, as is `localhost`.
    /// Public CDN hostnames and public IP literals pass; hostnames go on to be
    /// resolved and their *resolved addresses* are validated by
    /// [`download_image`](Self::download_image), since a name that looks public
    /// may still point at an internal network.
    fn validate_cover_url(url: &str) -> Result<()> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|e| TaggingError::CoverDownload(format!("Invalid cover URL '{url}': {e}")))?;

        if parsed.scheme() != "https" {
            return Err(TaggingError::CoverDownload(format!(
                "Cover URL '{url}' uses scheme '{}'; only https is allowed",
                parsed.scheme()
            )));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| TaggingError::CoverDownload(format!("Cover URL '{url}' has no host")))?;
        // `host_str` returns bracketed IPv6 literals (e.g. "[::1]").
        let host = host.trim_start_matches('[').trim_end_matches(']');

        let rejected = match std::net::IpAddr::from_str(host) {
            // IP literal: block loopback/private/link-local/reserved ranges.
            Ok(addr) => Self::is_restricted_ip(addr),
            // Hostname: block the loopback alias here; `localhost` resolves to
            // loopback addresses anyway, which `download_image`'s resolved
            // address check also catches.
            Err(_) => host.eq_ignore_ascii_case("localhost"),
        };
        if rejected {
            return Err(TaggingError::CoverDownload(format!(
                "Cover URL '{url}' points at a loopback/private/link-local address; refusing to fetch it"
            )));
        }
        Ok(())
    }

    /// True for IP literals that are loopback, private, link-local, multicast,
    /// reserved, or a tunnel embed of one of those IPv4 ranges (6to4, NAT64)
    /// and therefore must never be a cover-download destination.
    fn is_restricted_ip(addr: std::net::IpAddr) -> bool {
        match addr {
            std::net::IpAddr::V4(v4) => {
                let (a, b) = (v4.octets()[0], v4.octets()[1]);
                v4.is_unspecified()
                    || a == 0 // 0.0.0.0/8
                    || a == 127 // 127.0.0.0/8 loopback
                    || (a == 10) // 10.0.0.0/8
                    || (a == 172 && (16..=31).contains(&b)) // 172.16.0.0/12
                    || (a == 192 && b == 168) // 192.168.0.0/16
                    || (a == 169 && b == 254) // 169.254.0.0/16 link-local
                    || (a == 100 && (64..=127).contains(&b)) // 100.64.0.0/10 CGN
                    || a >= 224 // multicast + reserved (224.0.0.0/3)
            }
            std::net::IpAddr::V6(v6) => {
                let segs = v6.segments();
                if v6.is_unspecified()
                    || v6.is_loopback()
                    // fe80::/10 link-local
                    || (segs[0] & 0xffc0) == 0xfe80
                    // fc00::/7 unique-local
                    || (segs[0] & 0xfe00) == 0xfc00
                    // IPv4-mapped (::ffff:a.b.c.d): judge by the embedded v4.
                    || v6.to_ipv4_mapped().is_some_and(|v4| Self::is_restricted_ip(v4.into()))
                {
                    return true;
                }
                // `segments` returns the address's eight 16-bit groups in
                // network order.
                let first = segs[0];
                // ff00::/8 multicast.
                if (first & 0xff00) == 0xff00 {
                    return true;
                }
                // 2002::/16 (6to4): the embedded IPv4 address occupies 16-bit
                // groups 2-3; judge it as a plain v4 address.
                if first == 0x2002 {
                    return Self::is_restricted_ip(std::net::IpAddr::V4(Self::v4_from_6to4(&v6)));
                }
                // 64:ff9b::/96 (NAT64, RFC 6052): the embedded IPv4 address
                // occupies the last 4 bytes; judge it as a plain v4 address.
                // Only the true /96 is matched — the first 64 bits must be
                // exactly `64:ff9b:0000:0000` (groups 0-3) — so addresses in
                // neighboring prefixes that happen to end in the IPv4 form of a
                // restricted range are not misclassified.
                if segs[0] == 0x0064 && segs[1] == 0xff9b && segs[2] == 0 && segs[3] == 0 {
                    return Self::is_restricted_ip(std::net::IpAddr::V4(Self::v4_from_nat64(&v6)));
                }
                false
            }
        }
    }

    /// Extract the embedded IPv4 address from a 6to4 (`2002::/16`) address.
    /// The address's 16-bit groups 2-3 hold the IPv4 octets.
    fn v4_from_6to4(v6: &std::net::Ipv6Addr) -> std::net::Ipv4Addr {
        let segs = v6.segments();
        // Groups 2 and 3 (index 1) hold the IPv4 address.
        std::net::Ipv4Addr::new(
            (segs[1] >> 8) as u8,
            (segs[1] & 0xff) as u8,
            (segs[2] >> 8) as u8,
            (segs[2] & 0xff) as u8,
        )
    }

    /// Extract the embedded IPv4 address from a NAT64 (`64:ff9b::/96`) address.
    /// The address's last 4 bytes hold the IPv4 octets.
    fn v4_from_nat64(v6: &std::net::Ipv6Addr) -> std::net::Ipv4Addr {
        let octets: [u16; 8] = v6.segments();
        std::net::Ipv4Addr::new(
            (octets[6] >> 8) as u8,
            (octets[6] & 0xff) as u8,
            (octets[7] >> 8) as u8,
            (octets[7] & 0xff) as u8,
        )
    }

    /// Reject a set of resolved addresses if ANY of them is restricted
    /// ([`is_restricted_ip`]: loopback/private/link-local/multicast/reserved
    /// plus 6to4/NAT64 embeds of IPv4-restricted ranges).
    ///
    /// A hostname may list one public and one internal A record; the request
    /// could land on either, so every address is judged, not just the first.
    fn validate_addresses(addrs: impl IntoIterator<Item = std::net::SocketAddr>) -> Result<()> {
        for addr in addrs {
            if Self::is_restricted_ip(addr.ip()) {
                return Err(TaggingError::CoverDownload(format!(
                    "Cover URL resolves to a loopback/private/link-local address ({addr}); \
                     refusing to fetch it"
                )));
            }
        }
        Ok(())
    }

    /// Create a new Tagger with custom overwrite behavior
    pub fn with_overwrite_behavior(mut self, behavior: OverwriteBehavior) -> Self {
        self.overwrite_behavior = behavior;
        self
    }

    /// Build a cover-download client for one fetch: the no-redirect
    /// configuration plus a DNS pin of `host` to exactly `addrs`.
    ///
    /// Redirects are disabled: `download_image` validates the URL and its
    /// resolved addresses for SSRF before requesting it, and a redirect to an
    /// internal address would bypass that check. `download_image` treats any
    /// 3xx response as an error.
    ///
    /// The pin closes the TOCTOU window between `download_image`'s
    /// `validate_addresses` check and the connect: the client never re-resolves
    /// the host, so it can only reach the addresses that were just validated.
    /// The URL keeps the hostname, so TLS/SNI and certificate validation are
    /// unaffected.
    ///
    /// Proxying is intentionally disabled (`no_proxy()` also turns off the
    /// automatic system proxy, so `HTTPS_PROXY`/`ALL_PROXY` are ignored): a
    /// proxy receives the hostname in CONNECT and could route it to a
    /// restricted address the pin never saw, re-opening the SSRF bypass. The
    /// validated addresses are the only resolution path.
    fn build_fetch_client(host: &str, addrs: &[std::net::SocketAddr]) -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, addrs)
            .build()
            .expect("reqwest client with static settings cannot fail")
    }

    /// Separator between genres in `©gen`.
    ///
    /// `©gen` is a single string; the pre-Rust pipeline joined every genre
    /// with `/` and player/library conventions (and Audiobookshelf's parser)
    /// recognize that form, so it is kept for compatibility.
    const GENRE_SEPARATOR: &'static str = "/";

    /// Map `BookMetadata` onto an MP4 tag.
    ///
    /// Atoms:
    /// - title, plus subtitle when present -> ©nam (`"title - subtitle"`)
    /// - artist (authors), joined with ", " -> ©ART
    /// - album (base title only) -> ©alb
    /// - album artist (authors), joined with ", " -> ©aART
    /// - genre (all genres), joined with "/" -> ©gen
    /// - year -> ©day
    /// - comment (description) -> ©cmt
    /// - narrator -> ©wrt (composer), multiple narrators joined with ", "
    /// - series -> ©grp (grouping)
    ///
    /// Tags with no corresponding metadata are removed rather than left
    /// untouched, so retagging a file from a different source cannot leave
    /// stale values behind.
    fn apply_metadata(tag: &mut mp4ameta::Tag, metadata: &BookMetadata) {
        // Title (base title plus subtitle, when the subtitle is non-blank).
        // `BookMetadata::full_title` owns the documented `"title - subtitle"`
        // form so every consumer of the title agrees on the separator.
        tag.set_title(metadata.full_title());

        // Artist and album artist (authors)
        let authors = Self::join_values(&metadata.authors, ", ");
        if let Some(authors) = authors.as_deref() {
            tag.set_artist(authors);
            tag.set_album_artist(authors);
        } else {
            tag.remove_artists();
            tag.remove_album_artists();
        }

        // Album (base title only, so album grouping stays stable across
        // editions and unchanged by the subtitle)
        tag.set_album(&metadata.title);

        // Genre (all genres in a single `©gen` value)
        match Self::join_values(&metadata.genres, Self::GENRE_SEPARATOR) {
            Some(genres) => tag.set_genre(genres),
            None => tag.remove_genres(),
        }

        // Year
        if let Some(year) = metadata.year {
            tag.set_year(year.to_string());
        } else {
            tag.remove_year();
        }

        // Comment (description)
        if !metadata.description.is_empty() {
            tag.set_comment(&metadata.description);
        } else {
            tag.remove_comments();
        }

        // Narrator (composer is the convention used by Prologue, Audiobookshelf,
        // and Audible for audiobook narrators)
        if !metadata.narrators.is_empty() {
            tag.set_composer(metadata.narrators.join(", "));
        } else {
            tag.remove_composers();
        }

        // Series information. Grouping also carries any unrelated user content,
        // so rebuild the generated entry instead of appending: legacy
        // `Narrator: ...` segments are dropped and any existing `Series: ...`
        // segment is replaced. This keeps repeated writes and retags of files
        // produced by older versions from duplicating or retaining stale data.
        if !metadata.narrators.is_empty() || metadata.series_name.is_some() {
            let mut segments = Self::preserved_grouping_segments(tag.grouping());

            if let Some(series_name) = &metadata.series_name {
                let series_info = match &metadata.series_position {
                    Some(pos) => format!("{} #{}", series_name, pos),
                    None => series_name.clone(),
                };
                segments.push(format!("Series: {}", series_info));
            }

            if segments.is_empty() {
                tag.remove_groupings();
            } else {
                tag.set_grouping(segments.join(" | "));
            }
        }
    }

    /// Join non-blank values with `separator`, or `None` when every value is
    /// blank.
    fn join_values(values: &[String], separator: &str) -> Option<String> {
        let kept: Vec<&str> =
            values.iter().map(|value| value.trim()).filter(|value| !value.is_empty()).collect();
        if kept.is_empty() { None } else { Some(kept.join(separator)) }
    }

    /// Grouping segments that this tool did not generate.
    ///
    /// `©grp` historically held `Narrator: ...` and `Series: ...` entries; both
    /// are regenerated by [`Tagger::apply_metadata`], so they are filtered out
    /// while unrelated grouping content is kept.
    fn preserved_grouping_segments(grouping: Option<&str>) -> Vec<String> {
        grouping
            .unwrap_or_default()
            .split(" | ")
            .map(str::trim)
            .filter(|segment| {
                !segment.is_empty()
                    && !segment.starts_with("Narrator:")
                    && !segment.starts_with("Series:")
            })
            .map(str::to_string)
            .collect()
    }

    /// Write metadata tags to an MP4 file
    ///
    /// See [`Tagger::apply_metadata`] for the metadata-to-atom mapping.
    pub fn write_metadata<P: AsRef<Path>>(
        &self,
        file_path: P,
        metadata: &BookMetadata,
    ) -> Result<()> {
        let path = file_path.as_ref();

        Self::ensure_file_exists(path)?;

        debug!("Writing metadata to: {}", path.display());

        // Read existing tag or create new one
        let mut tag = mp4ameta::Tag::read_from_path(path).unwrap_or_default();

        Self::apply_metadata(&mut tag, metadata);

        // Write the tag back to file
        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write tags: {}", e)))?;

        info!("Successfully wrote metadata to: {}", path.display());
        Ok(())
    }

    /// Write metadata tags according to per-field policies.
    ///
    /// Unlike [`Tagger::write_metadata`] (the standalone CLI's all-fields
    /// behavior), this only mutates the atoms whose policy says so:
    /// - `Write` sets the atom from the effective value;
    /// - `Clear` removes the atom;
    /// - `Preserve` leaves the file's existing value untouched.
    ///
    /// Title and subtitle are independent logical components of the shared
    /// `©nam`/`©alb` representation: each is resolved on its own, and the
    /// unselected component of a preserved pair is read back from the file's
    /// existing atoms. A preserved component that cannot be established from
    /// the defined representation (`©alb` for the title, the `©nam`/`©alb`
    /// `"title - subtitle"` form for the subtitle) fails with
    /// [`TaggingError::PreserveUnresolvable`] before any atom is mutated.
    ///
    /// Atom ownership: title/subtitle own `©nam` and `©alb`; authors own
    /// `©ART`/`©aART`; narrators own `©wrt`; series owns the `Series:` segment
    /// of `©grp`; description owns `©cmt`; genres own `©gen`; year owns `©day`.
    /// Unrelated `©grp` segments and every atom not named here are never
    /// touched, so preserving one field cannot erase another.
    pub fn write_metadata_with_policies<P: AsRef<Path>>(
        &self,
        file_path: P,
        metadata: &BookMetadata,
        policies: &crate::job::WritePolicies,
    ) -> Result<()> {
        let path = file_path.as_ref();
        Self::ensure_file_exists(path)?;

        // Validate every explicit `write` before mutating anything: a write
        // without a valid available value is an error, not a silent preserve.
        Self::validate_policies(metadata, policies)?;

        // An all-preserve set changes no atom, so do not rewrite the file at
        // all: rewriting would reserialize the whole tag and is not
        // byte-identical to the input.
        if Self::all_preserve(policies) {
            debug!("All policies preserve; leaving {} untouched", path.display());
            return Ok(());
        }

        let mut tag = mp4ameta::Tag::read_from_path(path).unwrap_or_default();

        // When both components are preserved, the shared `©nam`/`©alb` pair
        // is left byte-for-byte untouched: rebuilding it would re-derive the
        // subtitle from the `©nam`-minus-`©alb` suffix and trim it, silently
        // dropping preserved tag data (e.g. `©nam = "Book -   "`). The
        // all-preserve early return above only covers the whole set; this
        // covers the case where another field is written and forces a tag
        // write, so the preserved title atoms must still survive verbatim.
        let both_preserve = matches!(
            (policies.title, policies.subtitle),
            (crate::job::TitlePolicy::Preserve, crate::job::SubtitlePolicy::Preserve)
        );
        if both_preserve {
            debug!("Title and subtitle preserved; leaving ©nam/©alb untouched");
        } else {
            // Resolve the title/subtitle components independently from the
            // existing atoms, then write `©nam`/`©alb` once. A written
            // component uses the effective metadata value (the same string
            // that names the output path); a preserved component is read back
            // from the file.
            let (title, subtitle) = Self::resolve_title_subtitle(
                path,
                &tag,
                &policies.title,
                &policies.subtitle,
                policies.effective_title(metadata),
                policies.effective_subtitle(metadata),
            )?;

            // A subtitle without a title is represented as the subtitle alone
            // in both atoms (no fabricated title separator); the unselected
            // component is never erased by the other's clear. The value is
            // used verbatim (no trim) so a preserved subtitle round-trips
            // exactly.
            let subtitle = subtitle.as_deref().filter(|s| !s.trim().is_empty());
            match (title.as_deref(), subtitle) {
                (Some(title), Some(subtitle)) => {
                    tag.set_title(format!("{title} - {subtitle}"));
                    tag.set_album(title);
                }
                (Some(title), None) => {
                    tag.set_title(title);
                    tag.set_album(title);
                }
                (None, Some(subtitle)) => {
                    tag.set_title(subtitle);
                    tag.set_album(subtitle);
                }
                (None, None) => {
                    tag.remove_title();
                    tag.remove_album();
                }
            }
        }

        match policies.authors {
            crate::job::FieldPolicy::Write => match Self::join_values(&metadata.authors, ", ") {
                Some(authors) => {
                    tag.set_artist(authors.clone());
                    tag.set_album_artist(authors);
                }
                None => unreachable!("validated: authors write requires values"),
            },
            crate::job::FieldPolicy::Clear => {
                tag.remove_artists();
                tag.remove_album_artists();
            }
            crate::job::FieldPolicy::Preserve => {}
        }

        match policies.narrators {
            crate::job::FieldPolicy::Write => match Self::join_values(&metadata.narrators, ", ") {
                Some(narrators) => tag.set_composer(narrators),
                None => unreachable!("validated: narrators write requires values"),
            },
            crate::job::FieldPolicy::Clear => {
                tag.remove_composers();
            }
            crate::job::FieldPolicy::Preserve => {}
        }

        self.write_series_policy(&mut tag, metadata, policies.series);

        match policies.description {
            crate::job::FieldPolicy::Write => {
                tag.set_comment(&metadata.description);
            }
            crate::job::FieldPolicy::Clear => {
                tag.remove_comments();
            }
            crate::job::FieldPolicy::Preserve => {}
        }

        match policies.genres {
            crate::job::FieldPolicy::Write => {
                match Self::join_values(&metadata.genres, Self::GENRE_SEPARATOR) {
                    Some(genres) => tag.set_genre(genres),
                    None => unreachable!("validated: genres write requires values"),
                }
            }
            crate::job::FieldPolicy::Clear => {
                tag.remove_genres();
            }
            crate::job::FieldPolicy::Preserve => {}
        }

        match policies.year {
            crate::job::FieldPolicy::Write => {
                tag.set_year(metadata.year.unwrap().to_string());
            }
            crate::job::FieldPolicy::Clear => {
                tag.remove_year();
            }
            crate::job::FieldPolicy::Preserve => {}
        }

        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write tags: {}", e)))?;

        info!("Successfully wrote policy-driven metadata to: {}", path.display());
        Ok(())
    }

    /// True when no policy mutates an atom: an all-preserve set can skip the
    /// file write entirely, keeping the output byte-identical to the input.
    fn all_preserve(policies: &crate::job::WritePolicies) -> bool {
        policies.title == crate::job::TitlePolicy::Preserve
            && policies.subtitle == crate::job::SubtitlePolicy::Preserve
            && policies.authors == crate::job::FieldPolicy::Preserve
            && policies.narrators == crate::job::FieldPolicy::Preserve
            && policies.series == crate::job::FieldPolicy::Preserve
            && policies.description == crate::job::FieldPolicy::Preserve
            && policies.genres == crate::job::FieldPolicy::Preserve
            && policies.year == crate::job::FieldPolicy::Preserve
    }

    /// Refuse a policy set whose `Write` has no valid available value.
    fn validate_policies(
        metadata: &BookMetadata,
        policies: &crate::job::WritePolicies,
    ) -> Result<()> {
        if policies.title == crate::job::TitlePolicy::Write
            && policies.effective_title(metadata).is_none()
        {
            return Err(TaggingError::WriteWithoutValue { field: "title" });
        }
        if policies.subtitle == crate::job::SubtitlePolicy::Write
            && policies.effective_subtitle(metadata).is_none()
        {
            return Err(TaggingError::WriteWithoutValue { field: "subtitle" });
        }
        let multi = |name: &'static str, values: &[String]| {
            if Self::join_values(values, ", ").is_none() {
                Err(TaggingError::WriteWithoutValue { field: name })
            } else {
                Ok(())
            }
        };
        if policies.authors == crate::job::FieldPolicy::Write {
            multi("authors", &metadata.authors)?;
        }
        if policies.narrators == crate::job::FieldPolicy::Write {
            multi("narrators", &metadata.narrators)?;
        }
        if policies.series == crate::job::FieldPolicy::Write
            && metadata.series_name.as_deref().map(str::trim).filter(|s| !s.is_empty()).is_none()
        {
            return Err(TaggingError::WriteWithoutValue { field: "series" });
        }
        if policies.description == crate::job::FieldPolicy::Write
            && metadata.description.trim().is_empty()
        {
            return Err(TaggingError::WriteWithoutValue { field: "description" });
        }
        if policies.genres == crate::job::FieldPolicy::Write {
            multi("genres", &metadata.genres)?;
        }
        if policies.year == crate::job::FieldPolicy::Write && metadata.year.is_none() {
            return Err(TaggingError::WriteWithoutValue { field: "year" });
        }
        Ok(())
    }

    /// Resolve the effective title and subtitle atoms from the policies and
    /// the file's existing `©nam`/`©alb`.
    ///
    /// `None` means the component is cleared. A preserved component reads
    /// back from the existing atoms; when the file has no album atom the
    /// base title cannot be established and the write fails before mutation
    /// rather than erasing data.
    fn resolve_title_subtitle(
        path: &Path,
        tag: &mp4ameta::Tag,
        title_policy: &crate::job::TitlePolicy,
        subtitle_policy: &crate::job::SubtitlePolicy,
        title_value: Option<String>,
        subtitle_value: Option<String>,
    ) -> Result<(Option<String>, Option<String>)> {
        let existing_title = tag.title().map(|t| t.to_string()).filter(|t| !t.is_empty());
        let existing_album = tag.album().map(|t| t.to_string()).filter(|t| !t.is_empty());

        let title: Option<String> = match title_policy {
            // The value comes from the effective metadata (validated
            // non-blank above), never from a string the policy carries.
            crate::job::TitlePolicy::Write => title_value,
            crate::job::TitlePolicy::Clear => None,
            crate::job::TitlePolicy::Preserve => {
                // `©alb` is documented as the base title only, so it is the
                // defined representation of the title component. A file with
                // a title but no album (foreign tool, manual edit) cannot
                // establish the base without guessing — refuse before mutation.
                match &existing_album {
                    Some(album) => Some(album.clone()),
                    None if existing_title.is_some() => Err(TaggingError::PreserveUnresolvable {
                        field: "title",
                        path: path.to_path_buf(),
                    })?,
                    None => None,
                }
            }
        };

        let subtitle: Option<String> = match subtitle_policy {
            // The value comes from the effective metadata (validated
            // non-blank in `validate_policies`), so there is no separate
            // write-without-value check here.
            crate::job::SubtitlePolicy::Write => subtitle_value,
            crate::job::SubtitlePolicy::Clear => None,
            crate::job::SubtitlePolicy::Preserve => {
                // The documented `©nam` form is `title - subtitle`: recover
                // the subtitle only when the existing title starts with the
                // album plus the documented separator. Never split on
                // arbitrary punctuation. A file with a title but no album
                // has no base to subtract — refuse before mutation.
                match (&existing_album, &existing_title) {
                    (Some(album), Some(t)) => {
                        // A title equal to the album is the established
                        // "no subtitle" representation — what `apply_metadata`
                        // writes and ffmpeg's concat carries over — not an
                        // error.
                        if t == album {
                            return Ok((title, None));
                        }
                        let sep = format!("{album} - ");
                        // The `©nam`/`©alb` pair is the documented
                        // representation: a title that does not extend the
                        // album by the documented separator is inconsistent,
                        // so the subtitle cannot be established — refuse
                        // before mutation rather than drop it as empty.
                        let Some(stripped) = t.strip_prefix(&sep) else {
                            return Err(TaggingError::PreserveUnresolvable {
                                field: "subtitle",
                                path: path.to_path_buf(),
                            });
                        };
                        // Preserve the subtitle verbatim (no trim): an all-preserve
                        // job must not rewrite the file, so the recovered suffix
                        // keeps its exact bytes. Only an all-whitespace suffix is
                        // treated as "no subtitle", not an error.
                        let sub = stripped.to_string();
                        (!sub.trim().is_empty()).then_some(sub)
                    }
                    // No title to subtract from, or an album without a title:
                    // the subtitle cannot be established — refuse.
                    (None, Some(_)) | (Some(_), None) => Err(TaggingError::PreserveUnresolvable {
                        field: "subtitle",
                        path: path.to_path_buf(),
                    })?,
                    (None, None) => None,
                }
            }
        };

        Ok((title, subtitle))
    }

    /// Write the `Series:` segment of `©grp` per policy, keeping unrelated
    /// segments. `Preserve` never touches `©grp`; `Write` adds the segment
    /// (validated to have a series name); `Clear` removes only the `Series:`
    /// segment. The tool-generated `Narrator:` legacy segment is filtered out
    /// on a write, matching [`Tagger::apply_metadata`]. A clear keeps any
    /// `Narrator:` and unrelated segments — it must not erase content that
    /// this field does not own.
    fn write_series_policy(
        &self,
        tag: &mut mp4ameta::Tag,
        metadata: &BookMetadata,
        policy: crate::job::FieldPolicy,
    ) {
        if policy == crate::job::FieldPolicy::Preserve {
            return;
        }
        if policy == crate::job::FieldPolicy::Clear {
            let grouping = tag.grouping().map(|g| g.to_string()).unwrap_or_default();
            let kept = grouping
                .split(" | ")
                .map(str::trim)
                .filter(|segment| !segment.is_empty() && !segment.starts_with("Series:"))
                .map(str::to_string)
                .collect::<Vec<_>>();
            if kept.is_empty() {
                tag.remove_groupings();
            } else {
                tag.set_grouping(kept.join(" | "));
            }
            return;
        }
        let mut segments = Self::preserved_grouping_segments(tag.grouping().map(|s| s as &str));
        segments.push(Self::series_info(metadata).expect("validated: series write"));
        if segments.is_empty() {
            tag.remove_groupings();
        } else {
            tag.set_grouping(segments.join(" | "));
        }
    }

    /// The `Series:` segment text for the metadata.
    fn series_info(metadata: &BookMetadata) -> Option<String> {
        let name = metadata.series_name.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
        Some(match &metadata.series_position {
            Some(pos) => format!("Series: {} #{pos}", name),
            None => format!("Series: {name}"),
        })
    }

    /// Download cover art from URL and embed it in the MP4 file
    pub async fn embed_cover<P: AsRef<Path>>(&self, file_path: P, cover_url: &str) -> Result<()> {
        let path = file_path.as_ref();

        Self::ensure_file_exists(path)?;

        debug!("Downloading cover from: {}", cover_url);

        // Download cover image
        let image_data = self.download_image(cover_url).await?;

        debug!("Downloaded cover image ({} bytes)", image_data.len());

        // Embed in file
        self.embed_cover_data(path, &image_data)
    }

    /// Maximum size of a local cover image in bytes (matches the URL limit).
    pub const LOCAL_COVER_MAX_BYTES: u64 = 5 * 1024 * 1024;

    /// Embed a cover image from a validated local file.
    ///
    /// The path must be absolute (job validation enforces this), the file
    /// must exist and be at most [`Self::LOCAL_COVER_MAX_BYTES`] bytes. The
    /// bytes then go through the same `covr` format handling as a URL
    /// download (JPEG/PNG as-is, WebP/GIF transcoded to JPEG).
    /// Embed a cover from a validated local file. The file must lie inside
    /// `asset_root` (the job's `output_root`, the only area the job owns);
    /// absolute paths that escape the root, or paths containing symlinks that
    /// resolve outside it, are rejected before any read. The read itself is
    /// bounded to [`Self::LOCAL_COVER_MAX_BYTES`]: a file that grows past the
    /// limit between the size check and the read is refused (no unbounded
    /// `fs::read` after a size-only check).
    ///
    /// Residual risk and trust boundary: the canonicalize-then-open sequence
    /// has a TOCTOU window. A concurrent writer that can modify files in the
    /// asset area (or replace the target itself) can swap it between
    /// [`Path::canonicalize`] and [`std::fs::File::open`] — replacing the
    /// file, or making it a symlink that points outside the asset root. Since
    /// `File::open` follows symlinks, the bytes read in that race are
    /// attacker-chosen and may come from outside the root; the size-bounded
    /// read limits the amount read, not the file's location. The implemented
    /// guarantee is static containment (the canonicalized path stays under the
    /// asset root) plus the size-bounded read, valid when no concurrent writer
    /// races the open. An atomic root-relative open (open the parent directory
    /// and resolve relative to it, then verify the opened inode) is a
    /// documented follow-up.
    pub fn embed_cover_local<P: AsRef<Path>, A: AsRef<Path>>(
        &self,
        file_path: P,
        cover_path: &Path,
        asset_root: A,
    ) -> Result<()> {
        if !cover_path.is_absolute() {
            return Err(TaggingError::CoverDownload(format!(
                "Local cover path must be absolute, got {}",
                cover_path.display()
            )));
        }
        // The asset root must itself be canonicalizable so the containment
        // comparison is well-defined; a missing root is a job configuration
        // error, not a cover content error.
        let asset_root = asset_root.as_ref().canonicalize().map_err(|e| {
            TaggingError::CoverDownload(format!(
                "Asset root {} is not usable: {}",
                asset_root.as_ref().display(),
                e
            ))
        })?;
        // Canonicalize the cover path: this resolves symlinks, so a path
        // spelled inside the asset area that links out of it is rejected.
        let canonical = cover_path.canonicalize().map_err(|e| {
            TaggingError::CoverDownload(format!(
                "Local cover file {} is not readable: {}",
                cover_path.display(),
                e
            ))
        })?;
        if !canonical.starts_with(&asset_root) {
            return Err(TaggingError::CoverPathOutsideAssetRoot(cover_path.display().to_string()));
        }
        // Bound the read to limit+1 bytes (a single open-then-bounded-read,
        // not a size-then-read TOCTOU window). If the buffer fills to the
        // limit+1 mark, the file is too large: refuse it.
        let file = std::fs::File::open(&canonical).map_err(|e| {
            TaggingError::CoverDownload(format!(
                "Failed to open local cover {}: {}",
                cover_path.display(),
                e
            ))
        })?;
        let mut data = Vec::with_capacity(Self::LOCAL_COVER_MAX_BYTES as usize + 1);
        let n = file.take(Self::LOCAL_COVER_MAX_BYTES + 1).read_to_end(&mut data).map_err(|e| {
            TaggingError::CoverDownload(format!(
                "Failed to read local cover {}: {}",
                cover_path.display(),
                e
            ))
        })?;
        if (n as u64) > Self::LOCAL_COVER_MAX_BYTES {
            return Err(TaggingError::CoverDownload(format!(
                "Local cover file {} exceeds the {} byte limit",
                cover_path.display(),
                Self::LOCAL_COVER_MAX_BYTES
            )));
        }
        self.embed_cover_data(file_path, &data)
    }

    /// Remove all embedded cover art. A deliberate `clear` cover policy;
    /// `preserve` never calls this.
    pub fn clear_cover<P: AsRef<Path>>(&self, file_path: P) -> Result<()> {
        let path = file_path.as_ref();
        Self::ensure_file_exists(path)?;

        let mut tag = mp4ameta::Tag::read_from_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to read tag: {}", e)))?;

        tag.remove_artworks();

        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write tags: {}", e)))?;

        info!("Cleared existing cover from: {}", path.display());
        Ok(())
    }

    /// Remove all chapter representations (Nero `chpl` list and QuickTime
    /// chapter track) from the file. A deliberate `clear` chapters policy;
    /// `preserve` never calls this.
    pub fn clear_chapters<P: AsRef<Path>>(&self, file_path: P) -> Result<()> {
        let path = file_path.as_ref();
        Self::ensure_file_exists(path)?;

        let mut tag = mp4ameta::Tag::read_from_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to read tag: {}", e)))?;

        tag.chapter_list_mut().clear();
        tag.chapter_track_mut().clear();

        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write chapters: {}", e)))?;

        info!("Cleared existing chapters from: {}", path.display());
        Ok(())
    }
    /// Embed pre-downloaded cover image data into an MP4 file
    pub fn embed_cover_data<P: AsRef<Path>>(&self, file_path: P, image_data: &[u8]) -> Result<()> {
        let path = file_path.as_ref();

        Self::ensure_file_exists(path)?;

        debug!("Embedding cover art into: {}", path.display());

        // Detect image format
        let format = detect_image_format(image_data)?;

        // `covr` atoms only carry JPEG/PNG/BMP; transcode anything else before
        // embedding instead of dropping the cover.
        let (img_fmt, data) = match format {
            ImageFormat::Jpeg => (mp4ameta::ImgFmt::Jpeg, image_data.to_vec()),
            ImageFormat::Png => (mp4ameta::ImgFmt::Png, image_data.to_vec()),
            ImageFormat::Webp | ImageFormat::Gif => {
                debug!("Converting {format:?} cover to JPEG before embedding");
                (mp4ameta::ImgFmt::Jpeg, convert_cover_to_jpeg(image_data)?)
            }
        };

        // Read existing tag or create new one
        let mut tag = mp4ameta::Tag::read_from_path(path).unwrap_or_default();

        let artwork = mp4ameta::Img::new(img_fmt, data);
        tag.set_artwork(artwork);

        // Write the tag back to file
        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to embed cover: {}", e)))?;

        info!("Successfully embedded cover art in: {}", path.display());
        Ok(())
    }

    /// Embed chapters into an M4B file
    ///
    /// This method writes chapters to the MP4 container in both forms players
    /// understand:
    /// - the Nero `chpl` atom (chapter list, read by ffmpeg and some Android apps)
    /// - the QuickTime chapter text track with a `chap` track reference (read by
    ///   Apple players, which ignore `chpl`)
    ///
    /// Any existing chapters in either form are replaced.
    ///
    /// # Arguments
    ///
    /// * `file_path` - Path to the M4B file
    /// * `chapters` - Slice of chapters to embed
    ///
    /// # Errors
    ///
    /// Returns `TaggingError` if:
    /// - File doesn't exist
    /// - Chapters are invalid (duplicates, or starting at/beyond the file duration)
    /// - Writing to file fails
    pub fn embed_chapters<P: AsRef<Path>>(
        &self,
        file_path: P,
        chapters: &[crate::metadata::Chapter],
    ) -> Result<()> {
        let path = file_path.as_ref();

        // Validate file exists
        Self::ensure_file_exists(path)?;

        // Skip if empty (with debug log)
        if chapters.is_empty() {
            debug!("No chapters to embed, skipping");
            return Ok(());
        }

        // Validate and sort chapters by start time
        let mut chapters_vec = chapters.to_vec();
        validate_and_sort_chapters(&mut chapters_vec)?;

        if chapters_vec.len() > CHAPTER_LIST_MAX_CHAPTERS {
            warn!(
                "Nero chapter list is limited to {} entries; omitting {} chapter(s) from: {}",
                CHAPTER_LIST_MAX_CHAPTERS,
                chapters_vec.len() - CHAPTER_LIST_MAX_CHAPTERS,
                path.display()
            );
        }

        info!("Embedding {} chapters into: {}", chapters_vec.len(), path.display());

        // Read existing tag
        let mut tag = mp4ameta::Tag::read_from_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to read tag: {}", e)))?;

        // Reject chapters at or beyond the file duration before writing. mp4ameta
        // stores start times verbatim, so such a chapter becomes an unplayable
        // entry. This guard lives at the write boundary so every caller — this
        // API, `process_completed_book`, and the processor — is covered.
        if let Some((count, details)) =
            describe_out_of_range_chapters(&chapters_vec, tag.info.duration)
        {
            return Err(TaggingError::ChaptersOutOfRange {
                path: path.to_path_buf(),
                count,
                playlength: format_duration(tag.info.duration),
                details,
            });
        }

        // Clear existing chapters in both representations
        tag.chapter_list_mut().clear();
        tag.chapter_track_mut().clear();

        // Convert to mp4ameta chapters and write both representations: the
        // Nero `chpl` list, truncated to the atom's 255-entry cap, and the
        // QuickTime chapter text track, which holds the full list.
        let mp4_chapters = convert_chapters_for_embedding(&chapters_vec);
        tag.chapter_list_mut().extend_from_slice(chpl_chapters(&mp4_chapters));
        tag.chapter_track_mut().extend(mp4_chapters);

        // Write back to file
        tag.write_to_path(path)
            .map_err(|e| TaggingError::Mp4Meta(format!("Failed to write chapters: {}", e)))?;

        info!("Successfully embedded {} chapters", chapters_vec.len());
        Ok(())
    }

    /// Download image from URL.
    ///
    /// SSRF guard: `validate_cover_url` fast-rejects non-https schemes and
    /// restricted address literals textually, then the host is resolved and
    /// every resolved address is checked against [`is_restricted_ip`] via
    /// [`validate_addresses`] before the request goes out (a public-looking
    /// name may point at an internal network). The connection is pinned to
    /// exactly those validated addresses — the per-fetch client overrides DNS
    /// for the host, so it no longer re-resolves and a re-resolve to a
    /// restricted address between our lookup and the connect is not possible.
    /// Redirects are disabled (`Policy::none()`), so a 3xx cannot bounce the
    /// download to an unchecked internal host; `download_image` treats any 3xx
    /// as an error.
    async fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        const MAX_SIZE: usize = 5 * 1024 * 1024; // 5 MB

        // Textual pre-check (scheme, literal IP, localhost) before any I/O.
        Self::validate_cover_url(url)?;

        // Resolve the host and reject if ANY resolved address is restricted.
        let parsed = reqwest::Url::parse(url)
            .map_err(|e| TaggingError::CoverDownload(format!("Invalid cover URL '{url}': {e}")))?;
        let port = parsed.port_or_known_default().unwrap_or(443);
        let host = parsed
            .host_str()
            .ok_or_else(|| TaggingError::CoverDownload(format!("Cover URL '{url}' has no host")))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let addrs = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|e| {
            TaggingError::CoverDownload(format!("Cover URL '{url}' does not resolve: {e}"))
        })?;
        let addrs: Vec<std::net::SocketAddr> = addrs.collect();
        Self::validate_addresses(addrs.iter().copied())?;

        // Pin the request to the validated addresses so the client cannot
        // re-resolve the host between the check above and the connect.
        let client = Self::build_fetch_client(&host, &addrs);
        let response =
            client.get(url).timeout(std::time::Duration::from_secs(30)).send().await.map_err(
                |e| TaggingError::CoverDownload(format!("Request failed/timeout: {}", e)),
            )?;

        // The client disables redirects (they could bypass the SSRF check above);
        // a 3xx means the server refused to serve the image directly.
        if response.status().is_redirection() {
            return Err(TaggingError::CoverDownload(format!(
                "Cover URL redirected ({}); redirects are not followed",
                response.status()
            )));
        }
        if !response.status().is_success() {
            return Err(TaggingError::CoverDownload(format!("HTTP error: {}", response.status())));
        }

        // Check content-length header for size limit
        if let Some(content_length) = response.content_length() {
            if content_length > MAX_SIZE as u64 {
                return Err(TaggingError::CoverDownload(format!(
                    "Image size {} bytes exceeds 5MB limit",
                    content_length
                )));
            }
        }

        // Download and verify size
        let bytes = response
            .bytes()
            .await
            .map_err(|e| TaggingError::CoverDownload(format!("Failed to read bytes: {}", e)))?;

        if bytes.len() > MAX_SIZE {
            return Err(TaggingError::CoverDownload(format!(
                "Image size {} bytes exceeds 5MB limit",
                bytes.len()
            )));
        }

        Ok(bytes.to_vec())
    }

    /// Write chapters to a chapters.txt file in mp4v2 format
    ///
    /// Format:
    /// ```text
    /// ## artist: Author Name
    /// ## album: Book Title
    /// ## total-duration: 10:23:45.123
    /// 00:00:00.000 Chapter 1 Title
    /// 00:15:32.500 Chapter 2 Title
    /// 01:23:45.000 Chapter 3 Title
    /// ```
    pub fn write_chapters_txt<P: AsRef<Path>>(
        &self,
        output_path: P,
        metadata: &BookMetadata,
    ) -> Result<()> {
        let artist =
            if metadata.authors.is_empty() { None } else { Some(metadata.authors.join(", ")) };

        self.write_chapters_txt_with(
            output_path,
            &metadata.title,
            artist.as_deref(),
            &metadata.chapters,
        )
    }

    /// Write chapters to a chapters.txt file using explicit header values.
    ///
    /// Used when chapters were resolved without provider metadata (generated
    /// from the input tracks), where there is no [`BookMetadata`] to export.
    pub fn write_chapters_txt_with<P: AsRef<Path>>(
        &self,
        output_path: P,
        album: &str,
        artist: Option<&str>,
        chapters: &[crate::metadata::Chapter],
    ) -> Result<()> {
        let path = output_path.as_ref();

        debug!("Writing chapters.txt to: {}", path.display());

        // Build chapters content
        let mut content = String::new();

        // Add headers
        if let Some(artist) = artist.filter(|artist| !artist.is_empty()) {
            content.push_str(&format!("## artist: {}\n", artist));
        }
        content.push_str(&format!("## album: {}\n", album));

        // Add total duration if available
        let total_duration = chapters.iter().map(|chapter| chapter.end_time()).max();
        if let Some(duration) = total_duration {
            content.push_str(&format!("## total-duration: {}\n", format_duration(duration)));
        }

        // Add empty line before chapters
        content.push('\n');

        // Add chapters
        for chapter in chapters {
            let timestamp = format_duration(chapter.start_time);
            content.push_str(&format!("{} {}\n", timestamp, chapter.title));
        }

        // Write to file
        std::fs::write(path, content)?;

        info!("Successfully wrote chapters.txt to: {}", path.display());
        Ok(())
    }

    /// Move source files to completed directory
    ///
    /// Returns a list of successfully moved file paths
    pub fn move_completed_files<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        files: &[P],
        dest_dir: Q,
    ) -> Result<Vec<PathBuf>> {
        let dest = dest_dir.as_ref();

        // Ensure destination directory exists
        std::fs::create_dir_all(dest)?;

        let mut moved_files = Vec::new();

        for file in files {
            let source = file.as_ref();

            if !source.exists() {
                warn!("Source file does not exist, skipping: {}", source.display());
                continue;
            }

            let file_name = source
                .file_name()
                .ok_or_else(|| TaggingError::FileMove("Invalid source path".to_string()))?;

            let dest_path = dest.join(file_name);

            // Check if destination exists
            if dest_path.exists() {
                match self.overwrite_behavior {
                    OverwriteBehavior::Skip => {
                        warn!("Destination exists, skipping: {}", dest_path.display());
                        continue;
                    }
                    OverwriteBehavior::Error => {
                        return Err(TaggingError::FileMove(format!(
                            "Destination already exists: {}",
                            dest_path.display()
                        )));
                    }
                    OverwriteBehavior::Force => {
                        warn!("Overwriting existing file: {}", dest_path.display());
                        std::fs::remove_file(&dest_path).map_err(|e| {
                            TaggingError::FileMove(format!(
                                "Failed to remove existing destination {}: {}",
                                dest_path.display(),
                                e
                            ))
                        })?;
                    }
                }
            }

            // Move the file
            if let Err(e) = std::fs::rename(source, &dest_path) {
                if is_cross_device_error(&e) {
                    std::fs::copy(source, &dest_path).map_err(|copy_err| {
                        TaggingError::FileMove(format!(
                            "Failed to copy {} to {} (cross-device fallback): {}",
                            source.display(),
                            dest_path.display(),
                            copy_err
                        ))
                    })?;

                    std::fs::remove_file(source).map_err(|remove_err| {
                        TaggingError::FileMove(format!(
                            "Failed to remove source {} after copy: {}",
                            source.display(),
                            remove_err
                        ))
                    })?;
                } else {
                    return Err(TaggingError::FileMove(format!(
                        "Failed to move {} to {}: {}",
                        source.display(),
                        dest_path.display(),
                        e
                    )));
                }
            }

            info!("Moved file: {} -> {}", source.display(), dest_path.display());
            moved_files.push(dest_path);
        }

        info!("Successfully moved {} files to {}", moved_files.len(), dest.display());
        Ok(moved_files)
    }

    /// Process a completed audiobook: tag, add cover, write chapters, and move source files
    ///
    /// This is a convenience method that performs all post-merge operations
    pub async fn process_completed_book<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        output_file: P,
        metadata: &BookMetadata,
        source_files: &[Q],
        completed_dir: Option<&Path>,
    ) -> Result<()> {
        let output = output_file.as_ref();

        // 1. Write metadata tags
        self.write_metadata(output, metadata)?;

        // 2. Download and embed cover art if URL is available
        if let Some(cover_url) = &metadata.cover_url {
            if let Err(e) = self.embed_cover(output, cover_url).await {
                warn!("Failed to embed cover art: {}", e);
                // Don't fail the whole operation if cover embedding fails
            }
        }

        // 3. Embed chapters if available
        if !metadata.chapters.is_empty() {
            if let Err(e) = self.embed_chapters(output, &metadata.chapters) {
                warn!("Failed to embed chapters: {}", e);
            }
        }

        // 4. Write chapters.txt alongside output file
        let chapters_txt_path = output.with_extension("").with_extension("chapters.txt");
        if let Err(e) = self.write_chapters_txt(&chapters_txt_path, metadata) {
            warn!("Failed to write chapters.txt: {}", e);
            // Don't fail the whole operation if chapters writing fails
        }

        // 5. Move source files to completed directory if specified
        if let Some(dest_dir) = completed_dir {
            let source_paths: Vec<_> = source_files.iter().map(|p| p.as_ref()).collect();
            if let Err(e) = self.move_completed_files(&source_paths, dest_dir) {
                error!("Failed to move source files: {}", e);
                // Don't fail the whole operation if file moving fails
            }
        }

        info!("Completed post-processing for: {}", output.display());
        Ok(())
    }
}

impl Default for Tagger {
    fn default() -> Self {
        Self::new()
    }
}

/// Image format enumeration.
///
/// `Jpeg` and `Png` are stored as-is; `Webp` and `Gif` are not valid `covr`
/// payloads and are transcoded to JPEG before embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageFormat {
    Jpeg,
    Png,
    Webp,
    Gif,
}

/// Detect image format from magic bytes
fn detect_image_format(data: &[u8]) -> Result<ImageFormat> {
    if data.len() < 8 {
        return Err(TaggingError::InvalidFormat("Image data too short".to_string()));
    }

    // JPEG magic bytes: FF D8 FF
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Ok(ImageFormat::Jpeg);
    }

    // PNG magic bytes: 89 50 4E 47 0D 0A 1A 0A
    if data[0] == 0x89
        && data[1] == 0x50
        && data[2] == 0x4E
        && data[3] == 0x47
        && data[4] == 0x0D
        && data[5] == 0x0A
        && data[6] == 0x1A
        && data[7] == 0x0A
    {
        return Ok(ImageFormat::Png);
    }

    // GIF magic bytes: "GIF87a" or "GIF89a"
    if data.len() >= 6 && &data[0..3] == b"GIF" && (&data[3..6] == b"87a" || &data[3..6] == b"89a")
    {
        return Ok(ImageFormat::Gif);
    }

    // WebP magic bytes: "RIFF" .... "WEBP" (chunk size between the two tags)
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Ok(ImageFormat::Webp);
    }

    Err(TaggingError::InvalidFormat("Unknown image format".to_string()))
}

/// Transcode an arbitrary raster image (WebP, GIF, ...) to JPEG via FFmpeg.
///
/// MP4 `covr` atoms only carry JPEG, PNG, or BMP data, so container formats
/// that cover providers also serve must be converted before embedding. The
/// pre-rewrite pipeline got this for free by writing the download to disk and
/// handing it to m4b-tool, which shells out to FFmpeg; FFmpeg is already a
/// required runtime dependency here, so conversion stays local.
///
/// Animated GIF and WebP inputs contribute their first frame only.
fn convert_cover_to_jpeg(image_data: &[u8]) -> Result<Vec<u8>> {
    let ffmpeg = crate::audio::ffmpeg::FFmpeg::discover().map_err(|e| {
        TaggingError::CoverConversion(format!(
            "FFmpeg is required to convert this cover image: {e}"
        ))
    })?;

    use std::io::Write;

    // `-frames:v 1` keeps animated inputs to a single still frame. No scaling
    // filter is applied, so the source dimensions are preserved. Both image
    // streams use pipes so FFmpeg diagnostics cannot expose temporary paths.
    let mut child = std::process::Command::new(ffmpeg.ffmpeg_path())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
            "-frames:v",
            "1",
            "-c:v",
            "mjpeg",
            "-q:v",
            "2",
            "-f",
            "image2",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| TaggingError::CoverConversion(format!("Failed to run FFmpeg: {e}")))?;

    // Feed stdin concurrently with draining stdout/stderr: an animated image
    // can produce its first JPEG before FFmpeg consumes all remaining input.
    // Dropping stdin signals EOF; always reap even if the input write fails.
    let stdin = child.stdin.take();
    let (input_result, output) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            stdin
                .ok_or_else(|| std::io::Error::other("FFmpeg stdin is not available"))
                .and_then(|mut stdin| stdin.write_all(image_data))
        });
        let output = child.wait_with_output();
        let input_result = writer
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("FFmpeg cover input writer panicked")));
        (input_result, output)
    });
    let output =
        output.map_err(|e| TaggingError::CoverConversion(format!("Failed to run FFmpeg: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TaggingError::CoverConversion(format!(
            "FFmpeg failed to convert cover image: {}",
            stderr.trim()
        )));
    }
    if let Err(e) = input_result {
        // With `-frames:v 1`, FFmpeg may close its input after decoding the
        // first frame of a larger animation. A successful decoder exit is
        // authoritative for that expected broken pipe.
        if e.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(TaggingError::CoverConversion(format!(
                "Failed to pipe cover image to FFmpeg: {e}"
            )));
        }
    }

    let jpeg = output.stdout;

    if jpeg.is_empty() {
        return Err(TaggingError::CoverConversion(
            "FFmpeg produced an empty cover image".to_string(),
        ));
    }

    Ok(jpeg)
}

/// Format a Duration as HH:MM:SS.mmm
pub(crate) fn format_duration(duration: Duration) -> String {
    let total_millis = duration.as_millis();
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis % 3_600_000) / 60_000;
    let seconds = (total_millis % 60_000) / 1_000;
    let millis = total_millis % 1_000;

    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

/// Check if an IO error indicates a cross-device link (EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows)
fn is_cross_device_error(err: &std::io::Error) -> bool {
    // Unix: errno 18 is EXDEV (Invalid cross-device link)
    // Windows: errno 17 is ERROR_NOT_SAME_DEVICE
    err.raw_os_error() == Some(18) || (cfg!(windows) && err.raw_os_error() == Some(17))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Chapter;
    use tempfile::TempDir;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(0)), "00:00:00.000");
        assert_eq!(format_duration(Duration::from_secs(90)), "00:01:30.000");
        assert_eq!(format_duration(Duration::from_millis(5432100)), "01:30:32.100");
        assert_eq!(format_duration(Duration::from_millis(3661001)), "01:01:01.001");
    }

    #[test]
    fn test_validate_cover_url_table() {
        // (url, should pass). Textual pre-check only: https + host that is not
        // a restricted address literal or localhost passes; resolved-address
        // validation happens in `download_image` and is tested separately.
        let cases = [
            ("https://images.example.com/covers/book.jpg", true),
            ("https://93.184.216.34/cover.jpg", true), // public IP literal
            ("https://localhost/cover.jpg", false),
            ("http://images.example.com/covers/book.jpg", false), // not https
            ("file:///etc/passwd", false),
            ("gopher://127.0.0.1/cover", false),
            ("https://127.0.0.1/cover.jpg", false),
            ("https://10.0.0.5/cover.jpg", false),
            ("https://192.168.1.10/cover.jpg", false),
            ("https://169.254.169.254/latest/meta-data/", false), // cloud metadata
            ("https://::1/cover.jpg", false),
            ("https://[::1]/cover.jpg", false),
            ("https://0.0.0.0/cover.jpg", false),
            ("https://0.9.9.9/cover.jpg", false), // 0.0.0.0/8
            ("not a url", false),
        ];
        for (url, expected) in cases {
            let passed = Tagger::validate_cover_url(url).is_ok();
            assert_eq!(passed, expected, "url {url:?}");
        }
    }

    #[test]
    fn test_validate_addresses_rejects_restricted_members() {
        use std::net::SocketAddr;
        use std::net::SocketAddrV4;
        use std::net::SocketAddrV6;
        use std::net::{Ipv4Addr, Ipv6Addr};

        // All-public set passes.
        let pub_only: [SocketAddr; 2] = [
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 443)),
            SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::new(0x2606, 0x4700, 0, 0, 0, 0, 0, 0x1),
                443,
                0,
                0,
            )),
        ];
        assert!(Tagger::validate_addresses(pub_only).is_ok());

        // A single restricted v4 address in a mixed set is rejected.
        let mixed: [SocketAddr; 2] =
            [pub_only[0], SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 10), 443))];
        assert!(Tagger::validate_addresses(mixed).is_err());

        // Restricted v6 (link-local) is rejected too.
        let v6ll: [SocketAddr; 1] = [SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
            443,
            0,
            0,
        ))];
        assert!(Tagger::validate_addresses(v6ll).is_err());

        // Empty set passes (nothing to judge).
        assert!(Tagger::validate_addresses(std::iter::empty::<SocketAddr>()).is_ok());
    }

    #[test]
    fn test_is_restricted_ip_table() {
        use std::net::IpAddr;
        use std::str::FromStr;

        let reject = [
            // Existing v4 behavior.
            "0.0.0.0",
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "224.0.0.1",
            "255.255.255.255",
            // Existing v6 behavior.
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            // IPv4-mapped restricted.
            "::ffff:127.0.0.1",
            // IPv6 multicast (ff00::/8).
            "ff02::1",
            "ff05::2",
            // 6to4 (2002::/16) embeds of restricted v4 ranges.
            "2002:7f00:1::5",
            "2002:0a00:0001::1",
            // NAT64 (64:ff9b::/96) embed of loopback.
            "64:ff9b::7f00:1",
            // NAT64 /96 embed of a CGN address (100.64.0.1).
            "64:ff9b::6440:1",
        ];
        for s in reject {
            assert!(
                Tagger::is_restricted_ip(IpAddr::from_str(s).unwrap()),
                "{s} should be restricted"
            );
        }

        let accept = [
            // Public v4.
            "1.1.1.1",
            "172.15.255.255",
            "172.32.0.1",
            "223.255.255.255",
            // Public v6.
            "2001:4860:4860::8888",
            "2400:3200::1",
            // 6to4 embed of a public v4 address (128.100.255.255).
            "2002:8064:ffff::",
            // NAT64 embed of a public v4 address (128.100.255.255).
            "64:ff9b::8064:ffff",
            // Outside the 64:ff9b::/96 prefix (a middle group is nonzero):
            // not a NAT64 embed, so the trailing 7f00:1 must not be read as
            // an embedded 127.0.0.1.
            "64:ff9b:0:1::7f00:1",
        ];
        for s in accept {
            assert!(
                !Tagger::is_restricted_ip(IpAddr::from_str(s).unwrap()),
                "{s} should be accepted"
            );
        }
    }

    #[test]
    fn test_detect_image_format_jpeg() {
        // JPEG magic bytes
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        assert_eq!(detect_image_format(&jpeg_data).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn test_detect_image_format_png() {
        // PNG magic bytes
        let png_data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_image_format(&png_data).unwrap(), ImageFormat::Png);
    }

    #[test]
    fn test_detect_image_format_unknown() {
        let unknown_data = vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        assert!(detect_image_format(&unknown_data).is_err());
    }

    #[test]
    fn test_detect_image_format_too_short() {
        let short_data = vec![0xFF, 0xD8];
        assert!(detect_image_format(&short_data).is_err());
    }

    #[test]
    fn test_detect_image_format_gif87a() {
        let gif_data = b"GIF87a\x01\x00\x01\x00".to_vec();
        assert_eq!(detect_image_format(&gif_data).unwrap(), ImageFormat::Gif);
    }

    #[test]
    fn test_detect_image_format_gif89a() {
        let gif_data = b"GIF89a\x01\x00\x01\x00".to_vec();
        assert_eq!(detect_image_format(&gif_data).unwrap(), ImageFormat::Gif);
    }

    #[test]
    fn test_detect_image_format_webp() {
        // "RIFF" <size> "WEBP"
        let mut webp_data = b"RIFF".to_vec();
        webp_data.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        webp_data.extend_from_slice(b"WEBPVP8 ");
        assert_eq!(detect_image_format(&webp_data).unwrap(), ImageFormat::Webp);
    }

    #[test]
    fn test_detect_image_format_riff_not_webp() {
        // A RIFF container that is not WebP (e.g. WAV) must stay unknown.
        let mut riff_data = b"RIFF".to_vec();
        riff_data.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        riff_data.extend_from_slice(b"WAVEfmt ");
        assert!(detect_image_format(&riff_data).is_err());
    }

    /// Generate a PNG test image with FFmpeg, then transcode it to `format`.
    fn generate_image(dir: &TempDir, format: &str) -> Vec<u8> {
        use std::process::Command;

        let png_path = dir.path().join("source.png");
        let out_path = dir.path().join(format!("cover.{format}"));

        let status = Command::new("ffmpeg")
            .args(["-f", "lavfi", "-i", "testsrc=size=64x64:duration=1", "-frames:v", "1", "-y"])
            .arg(&png_path)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => panic!("FFmpeg required for tests"),
        }

        let status = Command::new("ffmpeg")
            .arg("-i")
            .arg(&png_path)
            .args(["-frames:v", "1", "-y"])
            .arg(&out_path)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => panic!("FFmpeg required for tests"),
        }

        std::fs::read(&out_path).unwrap()
    }

    #[test]
    fn test_convert_cover_to_jpeg_produces_jpeg() {
        let temp_dir = TempDir::new().unwrap();
        let webp = generate_image(&temp_dir, "webp");

        let jpeg = convert_cover_to_jpeg(&webp).unwrap();
        assert_eq!(&jpeg[0..3], &[0xFF, 0xD8, 0xFF], "expected JPEG magic bytes");
        assert_eq!(detect_image_format(&jpeg).unwrap(), ImageFormat::Jpeg);
    }

    #[test]
    fn test_convert_cover_to_jpeg_rejects_non_image() {
        let error = convert_cover_to_jpeg(b"this is not an image")
            .expect_err("invalid image data must fail conversion");
        assert!(matches!(&error, TaggingError::CoverConversion(_)));
        let message = error.to_string();
        assert!(message.contains("FFmpeg failed to convert cover image"), "{message}");
        assert!(!message.contains("cover_input"), "{message}");
        assert!(!message.contains("cover_output"), "{message}");
        assert!(!message.contains(std::env::temp_dir().to_string_lossy().as_ref()), "{message}");
    }

    #[test]
    fn test_embed_cover_webp_converted_to_jpeg() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);
        let webp = generate_image(&temp_dir, "webp");

        let tagger = Tagger::new();
        tagger.embed_cover_data(&m4b_path, &webp).expect("embed webp cover");

        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let artwork = tag.artwork().expect("cover should be embedded");
        assert_eq!(artwork.fmt, mp4ameta::ImgFmt::Jpeg);
        assert_eq!(&artwork.data[0..3], &[0xFF, 0xD8, 0xFF], "stored data must be JPEG");
    }

    #[test]
    fn test_embed_cover_gif_converted_to_jpeg() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);
        let gif = generate_image(&temp_dir, "gif");

        let tagger = Tagger::new();
        tagger.embed_cover_data(&m4b_path, &gif).expect("embed gif cover");

        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let artwork = tag.artwork().expect("cover should be embedded");
        assert_eq!(artwork.fmt, mp4ameta::ImgFmt::Jpeg);
        assert_eq!(&artwork.data[0..3], &[0xFF, 0xD8, 0xFF], "stored data must be JPEG");
    }

    #[test]
    fn test_overwrite_behavior_default() {
        assert_eq!(OverwriteBehavior::default(), OverwriteBehavior::Error);
    }

    #[test]
    fn test_tagger_creation() {
        let tagger = Tagger::new();
        assert!(matches!(tagger.overwrite_behavior, OverwriteBehavior::Error));
    }

    #[test]
    fn test_tagger_with_overwrite_behavior() {
        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Force);
        assert!(matches!(tagger.overwrite_behavior, OverwriteBehavior::Force));
    }

    #[test]
    fn test_write_chapters_txt() {
        let temp_dir = TempDir::new().unwrap();
        let chapters_path = temp_dir.path().join("chapters.txt");

        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Description")
            .add_author("Test Author")
            .add_chapter(Chapter::new(
                "Chapter 1",
                Duration::from_secs(0),
                Duration::from_secs(600),
            ))
            .add_chapter(Chapter::new(
                "Chapter 2",
                Duration::from_secs(600),
                Duration::from_secs(600),
            ));

        let tagger = Tagger::new();
        tagger.write_chapters_txt(&chapters_path, &metadata).unwrap();

        let content = std::fs::read_to_string(&chapters_path).unwrap();
        assert!(content.contains("## artist: Test Author"));
        assert!(content.contains("## album: Test Book"));
        assert!(content.contains("00:00:00.000 Chapter 1"));
        assert!(content.contains("00:10:00.000 Chapter 2"));
    }

    #[test]
    fn test_write_chapters_txt_without_artist() {
        let temp_dir = TempDir::new().unwrap();
        let chapters_path = temp_dir.path().join("audiobook.chapters.txt");

        // Generated chapters have no provider metadata: no artist header, and the
        // album falls back to the output name.
        let chapters = vec![
            Chapter::new("Chapter 01", Duration::from_secs(0), Duration::from_secs(5)),
            Chapter::new("Chapter 02", Duration::from_secs(5), Duration::from_secs(5)),
        ];

        let tagger = Tagger::new();
        tagger.write_chapters_txt_with(&chapters_path, "audiobook", None, &chapters).unwrap();

        let content = std::fs::read_to_string(&chapters_path).unwrap();
        assert!(!content.contains("## artist:"));
        assert!(content.contains("## album: audiobook"));
        assert!(content.contains("## total-duration: 00:00:10.000"));
        assert!(content.contains("00:00:00.000 Chapter 01"));
        assert!(content.contains("00:00:05.000 Chapter 02"));
    }

    #[test]
    fn test_move_completed_files() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let file2 = source_dir.join("file2.txt");
        std::fs::write(&file1, "content1").unwrap();
        std::fs::write(&file2, "content2").unwrap();

        let tagger = Tagger::new();
        let moved =
            tagger.move_completed_files(&[file1.clone(), file2.clone()], &dest_dir).unwrap();

        assert_eq!(moved.len(), 2);
        assert!(dest_dir.join("file1.txt").exists());
        assert!(dest_dir.join("file2.txt").exists());
        assert!(!file1.exists());
        assert!(!file2.exists());
    }

    #[test]
    fn test_move_completed_files_skip_existing() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let existing_file = dest_dir.join("file1.txt");
        std::fs::write(&file1, "new content").unwrap();
        std::fs::write(&existing_file, "existing content").unwrap();

        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Skip);
        let moved = tagger.move_completed_files(&[file1.clone()], &dest_dir).unwrap();

        assert_eq!(moved.len(), 0);
        assert!(file1.exists()); // Should not be moved
        assert_eq!(std::fs::read_to_string(&existing_file).unwrap(), "existing content");
    }

    #[test]
    fn test_move_completed_files_error_on_existing() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let existing_file = dest_dir.join("file1.txt");
        std::fs::write(&file1, "new content").unwrap();
        std::fs::write(&existing_file, "existing content").unwrap();

        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Error);
        let result = tagger.move_completed_files(&[file1.clone()], &dest_dir);

        assert!(result.is_err());
    }

    #[test]
    fn test_move_completed_files_force_overwrite() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        let dest_dir = temp_dir.path().join("completed");

        std::fs::create_dir(&source_dir).unwrap();
        std::fs::create_dir(&dest_dir).unwrap();

        // Create test files
        let file1 = source_dir.join("file1.txt");
        let existing_file = dest_dir.join("file1.txt");
        std::fs::write(&file1, "new content").unwrap();
        std::fs::write(&existing_file, "existing content").unwrap();

        let tagger = Tagger::new().with_overwrite_behavior(OverwriteBehavior::Force);
        let moved = tagger.move_completed_files(&[file1.clone()], &dest_dir).unwrap();

        assert_eq!(moved.len(), 1);
        assert!(!file1.exists());
        assert_eq!(std::fs::read_to_string(&existing_file).unwrap(), "new content");
    }

    #[test]
    fn test_tagger_file_not_found() {
        let tagger = Tagger::new();
        let metadata = BookMetadata::new("B08XYZ1234", "Test", "Desc");

        let result = tagger.write_metadata("/nonexistent/path/file.m4b", &metadata);
        assert!(matches!(result, Err(TaggingError::FileNotFound(_))));
    }

    #[test]
    fn test_apply_metadata_writes_narrators_to_composer() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc")
            .add_author("Author One")
            .add_author("Author Two")
            .add_narrator("Narrator One")
            .add_narrator("Narrator Two");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.composer(), Some("Narrator One, Narrator Two"));
        assert_eq!(tag.artist(), Some("Author One, Author Two"));
        assert_eq!(tag.grouping(), None, "narrator must not be duplicated in grouping");
    }

    #[test]
    fn test_apply_metadata_includes_subtitle_in_title() {
        let metadata =
            BookMetadata::new("B08XYZ1234", "Test Book", "Desc").with_subtitle("The Sequel");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.title(), Some("Test Book - The Sequel"));
        assert_eq!(tag.album(), Some("Test Book"), "album keeps the base title only");
    }

    #[test]
    fn test_apply_metadata_without_subtitle_uses_plain_title() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.title(), Some("Test Book"));
    }

    #[test]
    fn test_apply_metadata_ignores_blank_subtitle() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc").with_subtitle("   ");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.title(), Some("Test Book"));
    }

    #[test]
    fn test_apply_metadata_joins_all_genres() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc")
            .add_genre("Fiction")
            .add_genre("Science Fiction")
            .add_genre("Adventure");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.genre(), Some("Fiction/Science Fiction/Adventure"));
    }

    #[test]
    fn test_apply_metadata_single_genre_unchanged() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc").add_genre("Fiction");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.genre(), Some("Fiction"));
    }

    #[test]
    fn test_apply_metadata_without_genres_leaves_genre_unset() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc");

        let mut tag = mp4ameta::Tag::default();
        tag.set_genre("Stale Genre");

        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.genre(), None, "retagging must not keep a stale genre");
    }

    #[test]
    fn test_apply_metadata_writes_album_artist_from_authors() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc")
            .add_author("Author One")
            .add_author("Author Two");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.album_artist(), Some("Author One, Author Two"));
        assert_eq!(tag.album(), Some("Test Book"));
    }

    #[test]
    fn test_apply_metadata_without_authors_leaves_album_artist_unset() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc");

        let mut tag = mp4ameta::Tag::default();
        tag.set_album_artist("Stale Artist");

        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.album_artist(), None, "retagging must not keep a stale album artist");
    }

    #[test]
    fn test_apply_metadata_keeps_series_in_grouping() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc")
            .add_narrator("Narrator One")
            .with_series("My Series", "2");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.composer(), Some("Narrator One"));
        assert_eq!(tag.grouping(), Some("Series: My Series #2"));
    }

    #[test]
    fn test_apply_metadata_without_narrators_leaves_composer_unset() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc").add_author("Author");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.composer(), None);
    }

    #[test]
    fn test_apply_metadata_removes_stale_year_comment_composer_when_absent() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "").add_author("Author");

        let mut tag = mp4ameta::Tag::default();
        tag.set_year("2019");
        tag.set_comment("Old description");
        tag.set_composer("Old Narrator");

        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.year(), None, "retagging must not keep a stale year");
        assert_eq!(tag.comment(), None, "retagging must not keep a stale comment");
        assert_eq!(tag.composer(), None, "retagging must not keep a stale composer");
    }

    #[test]
    fn test_apply_metadata_keeps_present_year_comment_composer() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "New description")
            .add_author("Author")
            .add_narrator("Narrator One")
            .with_year(2020);

        let mut tag = mp4ameta::Tag::default();
        tag.set_year("2019");
        tag.set_comment("Old description");
        tag.set_composer("Old Narrator");

        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.year(), Some("2020"));
        assert_eq!(tag.comment(), Some("New description"));
        assert_eq!(tag.composer(), Some("Narrator One"));
    }

    #[test]
    fn test_apply_metadata_series_grouping_is_idempotent() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc")
            .add_narrator("Narrator One")
            .with_series("My Series", "2");

        let mut tag = mp4ameta::Tag::default();
        Tagger::apply_metadata(&mut tag, &metadata);
        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.grouping(), Some("Series: My Series #2"));
    }

    #[test]
    fn test_apply_metadata_drops_legacy_grouping_entries() {
        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Desc")
            .add_narrator("New Narrator")
            .with_series("New Series", "2");

        let mut tag = mp4ameta::Tag::default();
        tag.set_grouping("Narrator: Old Narrator | Series: Old Series #1");

        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.composer(), Some("New Narrator"));
        assert_eq!(tag.grouping(), Some("Series: New Series #2"));
    }

    #[test]
    fn test_apply_metadata_preserves_unrelated_grouping() {
        let metadata =
            BookMetadata::new("B08XYZ1234", "Test Book", "Desc").add_narrator("New Narrator");

        let mut tag = mp4ameta::Tag::default();
        tag.set_grouping("User Grouping | Narrator: Old Narrator");

        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.grouping(), Some("User Grouping"));
    }

    #[test]
    fn test_apply_metadata_clears_grouping_holding_only_legacy_narrator() {
        let metadata =
            BookMetadata::new("B08XYZ1234", "Test Book", "Desc").add_narrator("New Narrator");

        let mut tag = mp4ameta::Tag::default();
        tag.set_grouping("Narrator: Old Narrator");

        Tagger::apply_metadata(&mut tag, &metadata);

        assert_eq!(tag.composer(), Some("New Narrator"));
        assert_eq!(tag.grouping(), None);
    }

    #[test]
    fn test_chapters_txt_format() {
        let temp_dir = TempDir::new().unwrap();
        let chapters_path = temp_dir.path().join("chapters.txt");

        let metadata = BookMetadata::new("B08XYZ1234", "Test Book", "Description")
            .add_author("Author One")
            .add_author("Author Two")
            .with_year(2023)
            .add_chapter(Chapter::new(
                "Intro",
                Duration::from_millis(0),
                Duration::from_millis(5000),
            ))
            .add_chapter(Chapter::new(
                "Chapter 1",
                Duration::from_millis(5000),
                Duration::from_millis(10000),
            ))
            .add_chapter(Chapter::new(
                "Chapter 2",
                Duration::from_millis(15000),
                Duration::from_millis(10000),
            ));

        let tagger = Tagger::new();
        tagger.write_chapters_txt(&chapters_path, &metadata).unwrap();

        let content = std::fs::read_to_string(&chapters_path).unwrap();

        // Check header format
        assert!(content.contains("## artist: Author One, Author Two"));
        assert!(content.contains("## album: Test Book"));
        assert!(content.contains("## total-duration: 00:00:25.000"));

        // Check chapter format
        assert!(content.contains("00:00:00.000 Intro"));
        assert!(content.contains("00:00:05.000 Chapter 1"));
        assert!(content.contains("00:00:15.000 Chapter 2"));
    }

    // Chapter embedding helper function tests

    #[test]
    fn test_convert_chapters_for_embedding() {
        let chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
        ];

        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        assert_eq!(mp4_chapters.len(), 2);
        assert_eq!(mp4_chapters[0].title, "Ch1");
        assert_eq!(mp4_chapters[0].start, Duration::ZERO);
        assert_eq!(mp4_chapters[1].title, "Ch2");
        assert_eq!(mp4_chapters[1].start, Duration::from_secs(60));
    }

    #[test]
    fn test_convert_chapters_for_embedding_sorts_chapters() {
        // Test that chapters are sorted by start_time
        let chapters = vec![
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch3", Duration::from_secs(120), Duration::from_secs(60)),
        ];

        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        assert_eq!(mp4_chapters.len(), 3);
        assert_eq!(mp4_chapters[0].title, "Ch1");
        assert_eq!(mp4_chapters[1].title, "Ch2");
        assert_eq!(mp4_chapters[2].title, "Ch3");
    }

    #[test]
    fn test_convert_chapters_for_embedding_title_truncation() {
        // Test that titles longer than 255 characters are truncated
        let long_title = "A".repeat(300);
        let chapters =
            vec![Chapter::new(long_title.clone(), Duration::ZERO, Duration::from_secs(60))];

        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        assert_eq!(mp4_chapters.len(), 1);
        assert_eq!(mp4_chapters[0].title.len(), 255);
    }

    #[test]
    fn test_convert_chapters_for_embedding_utf8_safe_truncation() {
        // Titles must be cut on a char boundary within the 255-byte chpl limit;
        // mp4ameta slices at a raw byte index and panics mid-character.
        let emoji_title = "😀".repeat(300);
        let chapters =
            vec![Chapter::new(emoji_title.clone(), Duration::ZERO, Duration::from_secs(60))];

        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        assert_eq!(mp4_chapters.len(), 1);
        // Each emoji is 4 bytes, so 63 whole emoji (252 bytes) fit under 255.
        assert_eq!(mp4_chapters[0].title, "😀".repeat(63));
        assert!(mp4_chapters[0].title.len() <= CHAPTER_TITLE_MAX_BYTES);
    }

    #[test]
    fn test_convert_chapters_for_embedding_truncates_three_byte_chars() {
        // 'é' is 2 bytes: 127 chars fit in 254 bytes, while 128 need 256.
        let title = "é".repeat(200);
        let chapters = vec![Chapter::new(title, Duration::ZERO, Duration::from_secs(60))];

        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        assert_eq!(mp4_chapters[0].title, "é".repeat(127));
        assert!(mp4_chapters[0].title.len() <= CHAPTER_TITLE_MAX_BYTES);
    }

    #[test]
    fn test_truncate_chapter_title_keeps_short_titles() {
        assert_eq!(truncate_chapter_title(""), "");
        assert_eq!(truncate_chapter_title("Chapter 1"), "Chapter 1");

        // Exactly at the limit is unchanged, including multibyte text.
        let at_limit = "é".repeat(127) + "a";
        assert_eq!(at_limit.len(), CHAPTER_TITLE_MAX_BYTES);
        assert_eq!(truncate_chapter_title(&at_limit), at_limit);
    }

    #[test]
    fn test_convert_chapters_for_embedding_keeps_all_entries() {
        // Conversion is representation-agnostic; only the chpl write site caps.
        let chapters: Vec<Chapter> = (0..CHAPTER_LIST_MAX_CHAPTERS + 1)
            .map(|i| {
                Chapter::new(
                    format!("Ch{}", i),
                    Duration::from_secs(i as u64 * 60),
                    Duration::from_secs(60),
                )
            })
            .collect();

        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        assert_eq!(mp4_chapters.len(), CHAPTER_LIST_MAX_CHAPTERS + 1);
        assert_eq!(mp4_chapters.last().unwrap().title, "Ch255");
    }

    #[test]
    fn test_chpl_chapters_caps_at_representable_count() {
        // A 256-entry chapter list would wrap the chpl entry count byte.
        let chapters: Vec<Chapter> = (0..CHAPTER_LIST_MAX_CHAPTERS + 1)
            .map(|i| {
                Chapter::new(
                    format!("Ch{}", i),
                    Duration::from_secs(i as u64 * 60),
                    Duration::from_secs(60),
                )
            })
            .collect();

        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        let chpl = chpl_chapters(&mp4_chapters);
        assert_eq!(chpl.len(), CHAPTER_LIST_MAX_CHAPTERS);
        assert_eq!(chpl.last().unwrap().title, "Ch254");

        // Lists that already fit are returned unchanged.
        assert_eq!(chpl_chapters(&mp4_chapters[..3]).len(), 3);
        assert!(chpl_chapters(&[]).is_empty());
    }

    #[test]
    fn test_convert_chapters_for_embedding_empty() {
        let chapters: Vec<Chapter> = vec![];
        let mp4_chapters = convert_chapters_for_embedding(&chapters);
        assert!(mp4_chapters.is_empty());
    }

    #[test]
    fn test_validate_and_sort_chapters_empty() {
        let mut chapters: Vec<Chapter> = vec![];
        let result = validate_and_sort_chapters(&mut chapters);
        assert!(result.is_ok());
        assert!(chapters.is_empty());
    }

    #[test]
    fn test_validate_and_sort_chapters_single() {
        let mut chapters = vec![Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60))];
        let result = validate_and_sort_chapters(&mut chapters);
        assert!(result.is_ok());
        assert_eq!(chapters.len(), 1);
    }

    #[test]
    fn test_validate_and_sort_chapters_unsorted_gets_sorted() {
        let mut chapters = vec![
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch3", Duration::from_secs(120), Duration::from_secs(60)),
        ];

        validate_and_sort_chapters(&mut chapters).unwrap();

        assert_eq!(chapters[0].title, "Ch1");
        assert_eq!(chapters[0].start_time, Duration::ZERO);
        assert_eq!(chapters[1].title, "Ch2");
        assert_eq!(chapters[1].start_time, Duration::from_secs(60));
        assert_eq!(chapters[2].title, "Ch3");
        assert_eq!(chapters[2].start_time, Duration::from_secs(120));
    }

    #[test]
    fn test_validate_and_sort_chapters_duplicates() {
        let chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::ZERO, Duration::from_secs(60)),
        ];
        let mut chapters_copy = chapters;
        let result = validate_and_sort_chapters(&mut chapters_copy);
        assert!(matches!(result, Err(TaggingError::DuplicateChapterTimes)));
    }

    #[test]
    fn test_validate_and_sort_chapters_already_sorted() {
        let mut chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::from_secs(60), Duration::from_secs(60)),
        ];

        let result = validate_and_sort_chapters(&mut chapters);
        assert!(result.is_ok());
        assert_eq!(chapters[0].title, "Ch1");
        assert_eq!(chapters[1].title, "Ch2");
    }

    // Helper function to create a minimal M4B file for testing
    fn create_minimal_m4b(dir: &TempDir) -> PathBuf {
        use std::process::Command;

        let path = dir.path().join("test.m4b");
        let status = Command::new("ffmpeg")
            .args(&[
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=mono",
                "-t",
                "10",
                "-c:a",
                "aac",
                "-b:a",
                "64k",
                "-y",
                path.to_str().unwrap(),
            ])
            .status();

        match status {
            Ok(s) if s.success() => path,
            _ => panic!("FFmpeg required for tests"),
        }
    }

    #[test]
    fn test_embed_chapters_empty() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        let tagger = Tagger::new();
        let chapters: Vec<Chapter> = vec![];

        let result = tagger.embed_chapters(&m4b_path, &chapters);
        assert!(result.is_ok());
    }

    #[test]
    fn test_embed_chapters_success() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        let chapters = vec![
            Chapter::new("Chapter 1", Duration::ZERO, Duration::from_secs(5)),
            Chapter::new("Chapter 2", Duration::from_secs(5), Duration::from_secs(5)),
        ];

        let tagger = Tagger::new();
        let result = tagger.embed_chapters(&m4b_path, &chapters);
        assert!(result.is_ok());

        // Verify chapters were embedded by reading the file
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let chapter_list = tag.chapter_list();
        assert_eq!(chapter_list.len(), 2);
        assert_eq!(chapter_list[0].title, "Chapter 1");
        assert_eq!(chapter_list[1].title, "Chapter 2");
    }

    #[test]
    fn test_embed_chapters_replaces_existing() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        // First embed some chapters
        let initial_chapters =
            vec![Chapter::new("Initial Chapter", Duration::ZERO, Duration::from_secs(10))];
        let tagger = Tagger::new();
        tagger.embed_chapters(&m4b_path, &initial_chapters).unwrap();

        // Verify initial chapters
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        assert_eq!(tag.chapter_list().len(), 1);

        // Now embed new chapters (should replace)
        let new_chapters = vec![
            Chapter::new("New Chapter 1", Duration::ZERO, Duration::from_secs(5)),
            Chapter::new("New Chapter 2", Duration::from_secs(5), Duration::from_secs(5)),
        ];
        tagger.embed_chapters(&m4b_path, &new_chapters).unwrap();

        // Verify chapters were replaced
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let chapter_list = tag.chapter_list();
        assert_eq!(chapter_list.len(), 2);
        assert_eq!(chapter_list[0].title, "New Chapter 1");
        assert_eq!(chapter_list[1].title, "New Chapter 2");
    }

    #[test]
    fn test_embed_chapters_sorts_unsorted_input() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        // Provide chapters out of order
        let chapters = vec![
            Chapter::new("Chapter 2", Duration::from_secs(5), Duration::from_secs(5)),
            Chapter::new("Chapter 1", Duration::ZERO, Duration::from_secs(5)),
        ];

        let tagger = Tagger::new();
        let result = tagger.embed_chapters(&m4b_path, &chapters);
        assert!(result.is_ok());

        // Verify chapters were sorted
        let tag = mp4ameta::Tag::read_from_path(&m4b_path).unwrap();
        let chapter_list = tag.chapter_list();
        assert_eq!(chapter_list.len(), 2);
        assert_eq!(chapter_list[0].title, "Chapter 1");
        assert_eq!(chapter_list[1].title, "Chapter 2");
    }

    #[test]
    fn test_embed_chapters_file_not_found() {
        let tagger = Tagger::new();
        let chapters = vec![Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60))];

        let result = tagger.embed_chapters("/nonexistent/path/file.m4b", &chapters);
        assert!(matches!(result, Err(TaggingError::FileNotFound(_))));
    }

    #[test]
    fn test_embed_chapters_duplicate_times_error() {
        let temp_dir = TempDir::new().unwrap();
        let m4b_path = create_minimal_m4b(&temp_dir);

        let chapters = vec![
            Chapter::new("Ch1", Duration::ZERO, Duration::from_secs(60)),
            Chapter::new("Ch2", Duration::ZERO, Duration::from_secs(60)),
        ];

        let tagger = Tagger::new();
        let result = tagger.embed_chapters(&m4b_path, &chapters);
        assert!(matches!(result, Err(TaggingError::DuplicateChapterTimes)));
    }
    /// Build a real M4B pre-tagged with a known `©nam`/`©alb` pair plus the
    /// atoms other policies must not touch.
    fn tagged_m4b(dir: &TempDir, nam: &str, album: &str) -> PathBuf {
        let path = create_minimal_m4b(dir);
        let mut tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        tag.set_title(nam);
        tag.set_album(album);
        tag.set_artist("Original Artist");
        tag.set_composer("Original Narrator");
        tag.set_comment("Original Description");
        tag.set_genre("Original/Genre");
        tag.set_year("2001");
        tag.set_grouping("My Custom Segment | Series: Old #1");
        tag.write_to_path(&path).unwrap();
        path
    }

    fn meta() -> BookMetadata {
        BookMetadata::new("ID1234567890", "New Title", "new description")
            .add_author("New Author")
            .add_narrator("New Narrator")
            .add_genre("New Genre")
            .with_subtitle("New Sub")
            .with_series("New Series", "2")
            .with_year(2024)
    }

    /// A policy set that writes every field; the written values come from the
    /// effective metadata (the same strings that name the output), so the
    /// policies carry no values of their own.
    fn all_write_policies() -> crate::job::WritePolicies {
        crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Write,
            subtitle: crate::job::SubtitlePolicy::Write,
            authors: crate::job::FieldPolicy::Write,
            narrators: crate::job::FieldPolicy::Write,
            series: crate::job::FieldPolicy::Write,
            description: crate::job::FieldPolicy::Write,
            genres: crate::job::FieldPolicy::Write,
            year: crate::job::FieldPolicy::Write,
        }
    }

    #[test]
    fn policy_write_all_fields_updates_only_named_atoms() {
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let tagger = Tagger::new();
        tagger.write_metadata_with_policies(&path, &meta(), &all_write_policies()).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "New Title - New Sub");
        assert_eq!(tag.album().unwrap(), "New Title");
        assert_eq!(tag.artist().unwrap(), "New Author");
        assert_eq!(tag.album_artist().unwrap(), "New Author");
        assert_eq!(tag.composer().unwrap(), "New Narrator");
        assert_eq!(tag.genre().unwrap(), "New Genre");
        assert_eq!(tag.year().unwrap(), "2024");
        // The unrelated grouping segment must survive a full write.
        assert_eq!(tag.grouping().unwrap(), "My Custom Segment | Series: New Series #2");
    }

    #[test]
    fn policy_preserve_all_leaves_file_untouched() {
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let before = std::fs::read(&path).unwrap();
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Preserve,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "Old Title - Old Sub");
        assert_eq!(tag.album().unwrap(), "Old Title");
        assert_eq!(tag.artist().unwrap(), "Original Artist");
        assert_eq!(tag.composer().unwrap(), "Original Narrator");
        assert_eq!(tag.comment().unwrap(), "Original Description");
        assert_eq!(tag.genre().unwrap(), "Original/Genre");
        assert_eq!(tag.year().unwrap(), "2001");
        assert_eq!(tag.grouping().unwrap(), "My Custom Segment | Series: Old #1");
        // An all-preserve set must leave the file byte-identical: no atom is
        // rewritten and the file is not reserialized at all.
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn policy_both_preserve_leaves_title_atoms_verbatim_when_other_field_writes() {
        // When title AND subtitle are both preserved, the shared `©nam`/`©alb`
        // pair must be left byte-for-byte even though another field write
        // forces the tag to be reserialized. Rebuilding it would re-derive
        // the subtitle from the `©nam`-minus-`©alb` suffix and trim it,
        // silently dropping preserved bytes.
        let temp_dir = TempDir::new().unwrap();
        // A whitespace-only subtitle suffix: the canonical no-subtitle case
        // (`©nam == ©alb`) is the easy one; this is the data-loss case.
        let path = tagged_m4b(&temp_dir, "Book -   ", "Book");
        let tagger = Tagger::new();
        // Preserve the title pair, write only the year.
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Preserve,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Write,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        // The written year is applied...
        assert_eq!(tag.year().unwrap(), "2024");
        // ...and the preserved title atoms survive byte-for-byte.
        assert_eq!(tag.title().unwrap(), "Book -   ");
        assert_eq!(tag.album().unwrap(), "Book");
    }

    #[test]
    fn policy_title_only_preserves_subtitle_component() {
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Write,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        // The title is written from the effective metadata (the same string
        // that names the output), and the unselected subtitle component must
        // survive a title-only write.
        assert_eq!(tag.title().unwrap(), "New Title - Old Sub");
        assert_eq!(tag.album().unwrap(), "New Title");
        // Unselected atoms stay.
        assert_eq!(tag.artist().unwrap(), "Original Artist");
    }

    #[test]
    fn policy_subtitle_only_preserves_title_component() {
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Preserve,
            subtitle: crate::job::SubtitlePolicy::Write,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        // The subtitle is written from the effective metadata; the unselected
        // title component must survive a subtitle-only write.
        assert_eq!(tag.title().unwrap(), "Old Title - New Sub");
        assert_eq!(tag.album().unwrap(), "Old Title");
    }

    #[test]
    fn policy_clear_title_preserves_subtitle_component() {
        // A clear-title must not erase the subtitle; the subtitle is the
        // only surviving component, represented alone (no fabricated title).
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Clear,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "Old Sub");
        assert_eq!(tag.album().unwrap(), "Old Sub");
    }

    #[test]
    fn policy_clear_title_with_write_subtitle() {
        // clear-title + write-subtitle: the written subtitle is the only
        // surviving component.
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Clear,
            subtitle: crate::job::SubtitlePolicy::Write,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "New Sub");
        assert_eq!(tag.album().unwrap(), "New Sub");
    }

    #[test]
    fn policy_preserve_subtitle_on_equal_title_album_is_no_sub() {
        // A file whose `©nam` equals `©alb` is the established no-subtitle
        // representation (what `apply_metadata` writes and ffmpeg's concat
        // carries over): a subtitle preserve must read it as "no subtitle",
        // not fail, and a plain all-preserve run against it must succeed.
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title", "Old Title");
        let tagger = Tagger::new();
        // Write the title, preserve the subtitle: the subtitle preserve must
        // read the equal title/album pair as "no subtitle", not fail.
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Write,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            ..all_write_policies()
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        // Title written from metadata; the preserved subtitle was none, so
        // `©nam` is the bare title and `©alb` the base title only.
        assert_eq!(tag.title().unwrap(), "New Title");
        assert_eq!(tag.album().unwrap(), "New Title");
    }

    #[test]
    fn policy_write_subtitle_with_blank_effective_value_fails() {
        // A subtitle write with no (or blank) effective subtitle value is a
        // diagnosed failure before any atom is mutated, not a silent clear.
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let meta_no_sub = BookMetadata::new("ID1234567890", "New Title", "d").with_subtitle("  ");
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            subtitle: crate::job::SubtitlePolicy::Write,
            ..all_write_policies()
        };
        let err = tagger.write_metadata_with_policies(&path, &meta_no_sub, &policies).unwrap_err();
        assert!(matches!(
            err,
            TaggingError::WriteWithoutValue { field } if field == "subtitle"
        ));
        // No atom changed: the file is untouched.
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "Old Title - Old Sub");
    }

    #[test]
    fn policy_clear_specific_atoms_removes_only_them() {
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Preserve,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Clear,
            narrators: crate::job::FieldPolicy::Clear,
            series: crate::job::FieldPolicy::Clear,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Clear,
            year: crate::job::FieldPolicy::Clear,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert!(tag.artist().is_none());
        assert!(tag.composer().is_none());
        assert!(tag.genre().is_none());
        assert!(tag.year().is_none());
        // The `Series:` segment is gone; the unrelated segment survives.
        assert_eq!(tag.grouping().unwrap(), "My Custom Segment");
        // Title/subtitle/description preserved.
        assert_eq!(tag.title().unwrap(), "Old Title - Old Sub");
        assert_eq!(tag.comment().unwrap(), "Original Description");
    }

    #[test]
    fn policy_write_without_value_fails_before_mutation() {
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let tagger = Tagger::new();
        // A metadata with no authors, writing authors: must fail, file intact.
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Write,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Write,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        let empty_authors = BookMetadata::new("ID1234567890", "T", "");
        let err =
            tagger.write_metadata_with_policies(&path, &empty_authors, &policies).unwrap_err();
        assert!(matches!(err, TaggingError::WriteWithoutValue { field: "authors" }));

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.artist().unwrap(), "Original Artist");
        assert_eq!(tag.title().unwrap(), "Old Title - Old Sub");
    }

    #[test]
    fn policy_preserve_title_without_album_fails_before_mutation() {
        let temp_dir = TempDir::new().unwrap();
        let path = create_minimal_m4b(&temp_dir);
        // A foreign file with a title but no album atom.
        let mut tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        tag.set_title("Only A Title");
        tag.set_artist("Some Artist");
        tag.write_to_path(&path).unwrap();

        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Preserve,
            subtitle: crate::job::SubtitlePolicy::Write,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        let err = tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap_err();
        assert!(matches!(err, TaggingError::PreserveUnresolvable { field: "title", .. }));
        // File untouched: the title atom is still the original.
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "Only A Title");
        assert_eq!(tag.artist().unwrap(), "Some Artist");
        assert!(tag.album().is_none());
    }

    #[test]
    fn policy_preserve_subtitle_with_inconsistent_nam_fails_before_mutation() {
        let temp_dir = TempDir::new().unwrap();
        let path = create_minimal_m4b(&temp_dir);
        // `©nam` does not extend `©alb` by the documented ` - ` separator.
        let mut tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        tag.set_title("Completely Different Name");
        tag.set_album("Base Title");
        tag.write_to_path(&path).unwrap();

        let tagger = Tagger::new();
        // Writing the title (a valid write) rebuilds the `©nam`/`©alb` pair;
        // the preserved subtitle must be re-established from the existing
        // atoms, but the inconsistent pair cannot, so it fails before any atom
        // is mutated. (When both components are preserved the pair is left
        // untouched, so this check applies only to the rebuild path.)
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Write,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        let err = tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap_err();
        assert!(matches!(err, TaggingError::PreserveUnresolvable { field: "subtitle", .. }));
    }

    #[test]
    fn policy_preserve_subtitle_keeps_verbatim_bytes() {
        // A preserved subtitle must round-trip exactly: trailing whitespace in
        // the `©nam` suffix is tag data, not noise to trim.
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Book - Sub  ", "Book");
        let tagger = Tagger::new();
        // An authors write rebuilds the `©nam`/`©alb` pair from the preserved
        // components; the preserved subtitle must round-trip exactly.
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Preserve,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Write,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "Book - Sub  ");
        assert_eq!(tag.album().unwrap(), "Book");
    }

    #[test]
    fn policy_clear_series_keeps_narrator_and_unrelated() {
        // Clearing `series` removes only the `Series:` segment; the
        // `Narrator:` and unrelated segments are not this field's to erase.
        let temp_dir = TempDir::new().unwrap();
        // Title extends album by the documented ` - ` separator so the
        // preserved subtitle resolves.
        let path = tagged_m4b(&temp_dir, "Base - Sub", "Base");
        let mut tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        tag.set_grouping("Narrator: X | Series: Y | Custom");
        tag.write_to_path(&path).unwrap();

        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Preserve,
            subtitle: crate::job::SubtitlePolicy::Preserve,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Clear,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta(), &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.grouping().unwrap(), "Narrator: X | Custom");
    }

    #[test]
    fn write_title_and_subtitle_from_effective_metadata() {
        // A title and subtitle `write` uses the effective metadata's values
        // (the same strings that name the output), so a job that supplies a
        // title with no subtitle writes the bare title to `©nam`/`©alb`.
        let temp_dir = TempDir::new().unwrap();
        let path = tagged_m4b(&temp_dir, "Old Title - Old Sub", "Old Title");
        let meta_no_sub = BookMetadata::new("ID1234567890", "New Title", "d");
        let tagger = Tagger::new();
        let policies = crate::job::WritePolicies {
            title: crate::job::TitlePolicy::Write,
            subtitle: crate::job::SubtitlePolicy::Clear,
            authors: crate::job::FieldPolicy::Preserve,
            narrators: crate::job::FieldPolicy::Preserve,
            series: crate::job::FieldPolicy::Preserve,
            description: crate::job::FieldPolicy::Preserve,
            genres: crate::job::FieldPolicy::Preserve,
            year: crate::job::FieldPolicy::Preserve,
        };
        tagger.write_metadata_with_policies(&path, &meta_no_sub, &policies).unwrap();

        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.title().unwrap(), "New Title");
        assert_eq!(tag.album().unwrap(), "New Title");
    }

    #[test]
    fn clear_chapters_removes_both_representations() {
        let temp_dir = TempDir::new().unwrap();
        let path = create_minimal_m4b(&temp_dir);
        let tagger = Tagger::new();
        tagger
            .embed_chapters(&path, &[Chapter::new("C1", Duration::ZERO, Duration::from_secs(5))])
            .unwrap();
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert_eq!(tag.chapter_list().len(), 1);

        tagger.clear_chapters(&path).unwrap();
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert!(tag.chapter_list().is_empty());
        assert!(tag.chapter_track().is_empty());
    }

    #[test]
    fn local_cover_embeds_and_rejects_bad_paths() {
        let temp_dir = TempDir::new().unwrap();
        let asset_root = temp_dir.path().join("assets");
        std::fs::create_dir_all(&asset_root).unwrap();
        let path = create_minimal_m4b(&temp_dir);
        let cover = asset_root.join("cover.png");
        // Minimal 1x1 PNG.
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&cover, PNG).unwrap();

        let tagger = Tagger::new();
        // A file inside the asset area is accepted.
        tagger.embed_cover_local(&path, &cover, &asset_root).unwrap();
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert!(tag.artwork().is_some());

        // Relative path is rejected.
        let err = tagger.embed_cover_local(&path, Path::new("cover.png"), &asset_root).unwrap_err();
        assert!(matches!(err, TaggingError::CoverDownload(_)));
        // Missing file is rejected.
        let missing = asset_root.join("missing.png");
        let err = tagger.embed_cover_local(&path, &missing, &asset_root).unwrap_err();
        assert!(matches!(err, TaggingError::CoverDownload(_)));
    }

    #[test]
    fn local_cover_rejects_escaping_paths() {
        let temp_dir = TempDir::new().unwrap();
        let asset_root = temp_dir.path().join("assets");
        std::fs::create_dir_all(&asset_root).unwrap();
        let path = create_minimal_m4b(&temp_dir);
        // A PNG that lives OUTSIDE the asset area.
        let outside = temp_dir.path().join("outside.png");
        let png = [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        std::fs::write(&outside, &png).unwrap();
        let tagger = Tagger::new();
        // A direct path outside the asset area is rejected, not read.
        let err = tagger.embed_cover_local(&path, &outside, &asset_root).unwrap_err();
        assert!(
            matches!(err, TaggingError::CoverPathOutsideAssetRoot(_)),
            "expected path-escape rejection, got {err:?}"
        );
        // A symlink inside the asset area that points out of it is rejected.
        let link = asset_root.join("link.png");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(unix)]
        let err = tagger.embed_cover_local(&path, &link, &asset_root).unwrap_err();
        #[cfg(unix)]
        assert!(
            matches!(err, TaggingError::CoverPathOutsideAssetRoot(_)),
            "expected symlink-escape rejection, got {err:?}"
        );
    }

    #[test]
    fn local_cover_rejects_oversized_file() {
        let temp_dir = TempDir::new().unwrap();
        let asset_root = temp_dir.path().join("assets");
        std::fs::create_dir_all(&asset_root).unwrap();
        let path = create_minimal_m4b(&temp_dir);
        let big = asset_root.join("big.png");
        // limit + 1 bytes; the bounded read must refuse it.
        let data = vec![0u8; Tagger::LOCAL_COVER_MAX_BYTES as usize + 1];
        std::fs::write(&big, data).unwrap();
        let tagger = Tagger::new();
        let err = tagger.embed_cover_local(&path, &big, &asset_root).unwrap_err();
        assert!(
            matches!(err, TaggingError::CoverDownload(_)),
            "expected oversize rejection, got {err:?}"
        );
    }

    #[test]
    fn clear_cover_removes_embedded_artwork() {
        let temp_dir = TempDir::new().unwrap();
        let asset_root = temp_dir.path().to_path_buf();
        let path = create_minimal_m4b(&temp_dir);
        let cover = temp_dir.path().join("cover.png");
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&cover, PNG).unwrap();

        let tagger = Tagger::new();
        tagger.embed_cover_local(&path, &cover, &asset_root).unwrap();
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert!(tag.artwork().is_some());

        tagger.clear_cover(&path).unwrap();
        let tag = mp4ameta::Tag::read_from_path(&path).unwrap();
        assert!(tag.artwork().is_none());
    }
}
