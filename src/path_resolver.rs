//! One parsed-template output-path resolver, shared by the non-mutating
//! preview and execution.
//!
//! Semantics that are contractual with callers:
//! - The template is parsed exactly once into literal segments and tokens.
//!   Only template separators create directories; a substituted value is
//!   component data, so slashes, dot segments, and token spellings inside a
//!   value can never introduce directories, re-trigger substitution, or
//!   escape the output root.
//! - Substituted values are sanitized as component data. A value that
//!   sanitizes to nothing contributes no segment.
//! - Unknown tokens are rejected with an actionable diagnostic naming the
//!   token; there is no silent-ignore path.
//! - The resolved relative path is the exact same string in preview and in
//!   execution, from the same effective metadata.

use std::fmt;

use crate::metadata::BookMetadata;

/// Resolver identity: the token set and substitution semantics this build
/// implements. Bump when a token is added, removed, renamed, or a
/// substitution rule changes; the value appears in the `--capabilities`
/// output and is meant to be compared by callers across runs.
pub const RESOLVER_VERSION: &str = "1";

/// Default template when a job omits `path_format`.
pub const DEFAULT_TEMPLATE: &str = "{author}/{title}";

/// A parsed template: literal segments interleaved with tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub parts: Vec<TemplatePart>,
}

/// One part of a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePart {
    /// Literal text; its separators split path components.
    Literal(String),
    /// A supported token; its value is substituted as component data.
    Token(TokenKind),
}

/// Supported template tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Author,
    Authors,
    Narrator,
    Narrators,
    Title,
    Subtitle,
    SeriesName,
    SeriesPosition,
    Year,
    Genre,
    MetadataId,
    BookId,
}

impl TokenKind {
    /// Token spelling as it appears in a template.
    pub fn spelling(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Authors => "authors",
            Self::Narrator => "narrator",
            Self::Narrators => "narrators",
            Self::Title => "title",
            Self::Subtitle => "subtitle",
            Self::SeriesName => "series_name",
            Self::SeriesPosition => "series_position",
            Self::Year => "year",
            Self::Genre => "genre",
            Self::MetadataId => "metadata_id",
            Self::BookId => "book_id",
        }
    }
}

/// Errors produced while parsing a template or resolving it against metadata.
#[derive(Debug)]
pub enum PathResolutionError {
    /// A template occurrence the resolver does not support.
    UnknownToken {
        token: String,
        /// The position (in characters) where the unknown token begins.
        position: usize,
    },
    /// No path component survives sanitization, so no output file name can
    /// be derived.
    EmptyPath { template: String },
}

impl fmt::Display for PathResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownToken { token, position } => write!(
                f,
                "path template uses unsupported token '{{{token}}}' at position {position}; \
                 supported tokens: {SUPPORTED_TOKENS}",
                SUPPORTED_TOKENS = TokenKind::all_spelling_list()
            ),
            Self::EmptyPath { template } => write!(
                f,
                "path template {template:?} resolves to no file name after sanitizing the \
                 substituted values"
            ),
        }
    }
}

impl std::error::Error for PathResolutionError {}

fn parse_token_kind(name: &str) -> Option<TokenKind> {
    (TokenKind::all().iter().find(|k| k.spelling() == name)).copied()
}

impl Template {
    /// Parse a template exactly once, rejecting unknown tokens.
    pub fn parse(template: &str) -> Result<Self, PathResolutionError> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut i = 0usize;
        let mut char_pos = 0usize;
        while i < template.len() {
            let ch = template[i..].chars().next().expect("checked index");
            if ch == '{' {
                let rest = &template[i + 1..];
                let Some(end) = rest.find('}') else {
                    // No closing brace: the rest is literal text (the legacy
                    // contract is literal pass-through, and a value containing
                    // a brace is data, not syntax).
                    literal.push_str(&template[i..]);
                    break;
                };
                let name = &rest[..end];
                if let Some(kind) = parse_token_kind(name) {
                    if !literal.is_empty() {
                        parts.push(TemplatePart::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(TemplatePart::Token(kind));
                } else {
                    return Err(PathResolutionError::UnknownToken {
                        token: name.to_string(),
                        position: char_pos,
                    });
                }
                char_pos += 1 + name.len() + 1;
                i += 1 + end + 1;
            } else {
                literal.push(ch);
                char_pos += 1;
                i += ch.len_utf8();
            }
        }
        if !literal.is_empty() {
            parts.push(TemplatePart::Literal(literal));
        }
        Ok(Self { parts })
    }
}

/// The resolver: parse once, substitute values as component data, join.
#[derive(Debug, Clone, Copy, Default)]
pub struct PathResolver;

impl PathResolver {
    /// The full list of supported token spellings, for diagnostics.
    pub fn supported_tokens() -> &'static [TokenKind] {
        TokenKind::all()
    }

    /// Resolve `template` against `metadata` into a relative path of
    /// `/`-separated components. The last component is the file name stem;
    /// the caller appends the output extension.
    ///
    /// Only template separators create directories. A token's value joins
    /// the surrounding literal text in the same segment, so the value's own
    /// content (slashes, token spellings, dots) is component data, never
    /// syntax.
    pub fn resolve(template: &str, metadata: &BookMetadata) -> Result<String, PathResolutionError> {
        let parsed = Template::parse(template)?;
        let mut components: Vec<String> = Vec::new();
        let mut current = String::new();
        let flush = |components: &mut Vec<String>, current: &mut String| {
            if !current.is_empty() {
                components.push(sanitize_component(current));
                current.clear();
            }
        };
        for part in &parsed.parts {
            match part {
                // Only template separators create directories: each `/` in
                // literal text closes the current segment.
                TemplatePart::Literal(text) => {
                    for (index, segment) in text.split('/').enumerate() {
                        if index > 0 {
                            flush(&mut components, &mut current);
                        }
                        current.push_str(segment);
                    }
                }
                // A token's value substitutes into the segment it appears in;
                // the value is component data, never syntax.
                TemplatePart::Token(kind) => {
                    current.push_str(&token_value(*kind, metadata));
                }
            }
        }
        flush(&mut components, &mut current);
        // Empty segments (empty token values, double separators, leading and
        // trailing separators) are stripped, matching the documented
        // legacy behavior.
        let components: Vec<String> = components.into_iter().filter(|c| !c.is_empty()).collect();
        if components.is_empty() {
            return Err(PathResolutionError::EmptyPath { template: template.to_string() });
        }
        Ok(components.join("/"))
    }

    /// Resolve the template to the full relative output path, including the
    /// `.m4b` extension on the last component. This is the exact path both
    /// preview and execution report.
    pub fn resolve_output(
        template: &str,
        metadata: &BookMetadata,
    ) -> Result<String, PathResolutionError> {
        Ok(format!("{}.m4b", Self::resolve(template, metadata)?))
    }
}

/// Sanitize one path component so it is safe as a file system entry name.
///
/// Component-data sanitization: path separators and platform-illegal
/// characters are removed, leading `.` and drive-like `:` prefixes are
/// stripped so a value cannot traverse, address a drive, or become a hidden
/// or reserved entry.
fn sanitize_component(raw: &str) -> String {
    // Path separators are never component data: a substituted value cannot
    // create directories. (The legacy path keeps this split-after-substitution
    // behavior deliberately; the new contract does not.)
    let without_separators: String = raw.chars().filter(|c| *c != '/' && *c != '\\').collect();
    let sanitized = sanitize_filename::sanitize(&without_separators);
    // Dot runs are never component data: a `..` run cannot survive (no
    // traversal, no empty-segment trick), while single dots in names are
    // kept. Leading dots are trimmed so a value cannot become a hidden
    // entry.
    let no_dots = remove_dot_runs(&sanitized);
    no_dots.trim_start_matches(['.', ':']).trim_end().to_string()
}

/// Remove `..` runs from a string, keeping single dots.
fn remove_dot_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dot_run = false;
    for ch in s.chars() {
        if ch == '.' {
            if dot_run {
                out.pop();
                dot_run = false;
            } else {
                out.push(ch);
                dot_run = true;
            }
        } else {
            out.push(ch);
            dot_run = false;
        }
    }
    out
}

/// The value a token substitutes, before component sanitization.
///
/// Singular people tokens take the first ordered (non-blank) name; plural
/// tokens join the ordered names with the documented `, ` separator. Missing
/// values contribute nothing, except the people tokens, which keep the
/// documented `Unknown` fallback.
fn token_value(kind: TokenKind, metadata: &BookMetadata) -> String {
    let first = |values: &[String]| {
        values.iter().map(|v| v.trim()).find(|v| !v.is_empty()).map(str::to_string)
    };
    let joined = |values: &[String]| -> Vec<String> {
        values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()).map(|v| v.to_string()).collect()
    };
    match kind {
        TokenKind::Author => first(&metadata.authors).unwrap_or_else(|| "Unknown".to_string()),
        TokenKind::Authors => {
            let v = joined(&metadata.authors);
            if v.is_empty() { "Unknown".to_string() } else { v.join(", ") }
        }
        TokenKind::Narrator => first(&metadata.narrators).unwrap_or_else(|| "Unknown".to_string()),
        TokenKind::Narrators => {
            let v = joined(&metadata.narrators);
            if v.is_empty() { "Unknown".to_string() } else { v.join(", ") }
        }
        TokenKind::Title => metadata.title.trim().to_string(),
        TokenKind::Subtitle => metadata
            .subtitle
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
            .to_string(),
        TokenKind::SeriesName => metadata
            .series_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
            .to_string(),
        TokenKind::SeriesPosition => metadata
            .series_position
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
            .to_string(),
        TokenKind::Year => metadata.year.map(|y| y.to_string()).unwrap_or_default(),
        TokenKind::Genre => first(&metadata.genres).unwrap_or_default(),
        TokenKind::MetadataId => metadata.metadata_id.trim().to_string(),
        // The canonical book id (provider-supplied when present). A supplied
        // job carries none, so the token contributes no segment — it is
        // never a relabeling of the provider-dependent `metadata_id`.
        TokenKind::BookId => metadata.book_id.trim().to_string(),
    }
}

impl TokenKind {
    /// Every supported token.
    pub fn supported_tokens() -> &'static [TokenKind] {
        Self::all()
    }

    fn all() -> &'static [TokenKind] {
        &[
            Self::Author,
            Self::Authors,
            Self::Narrator,
            Self::Narrators,
            Self::Title,
            Self::Subtitle,
            Self::SeriesName,
            Self::SeriesPosition,
            Self::Year,
            Self::Genre,
            Self::MetadataId,
            Self::BookId,
        ]
    }

    fn all_spelling_list() -> String {
        Self::all()
            .iter()
            .map(|k| format!("{{{}}} ", k.spelling()))
            .collect::<String>()
            .trim()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::BookMetadata;

    fn meta() -> BookMetadata {
        BookMetadata::new("B08XYZ1234", "The Book", "")
            .add_author("The Author")
            .add_author("Second Author")
            .add_narrator("The Narrator")
            .add_genre("Mystery")
            .with_subtitle("A Subtitle")
            .with_series("The Series", "1")
            .with_year(2021)
    }

    fn meta_with_book_id(mut m: BookMetadata) -> BookMetadata {
        m.book_id = "575".to_string();
        m
    }

    #[test]
    fn resolves_full_template_in_order() {
        let m = meta_with_book_id(meta());
        // Template separators alone create directories; a token's value joins
        // the same segment as its neighbors (no re-splitting after
        // substitution).
        let result = PathResolver::resolve(
            "{author}/{series_name}/{title} - {subtitle} ({year})/{narrators}/{authors}/{book_id}/{metadata_id}",
            &m,
        )
        .unwrap();
        assert_eq!(
            result,
            "The Author/The Series/The Book - A Subtitle (2021)/The Narrator/The Author, Second Author/575/B08XYZ1234"
        );
    }

    #[test]
    fn unknown_token_is_rejected_with_position() {
        let m = meta();
        let err = PathResolver::resolve("{author}/{bogus}", &m).unwrap_err();
        let message = err.to_string();
        match err {
            PathResolutionError::UnknownToken { token, position } => {
                assert_eq!(token, "bogus");
                assert_eq!(position, 9);
                assert!(
                    message.contains("supported tokens"),
                    "diagnostic must list supported tokens: {message}"
                );
            }
            other => panic!("expected UnknownToken, got {other:?}"),
        }
    }

    #[test]
    fn value_slashes_cannot_introduce_directories() {
        let mut m = meta();
        m.title = "Evil/Title/With/Separators".to_string();
        let result = PathResolver::resolve("{author}/{title}", &m).unwrap();
        // Two components only: the value's slashes are component data.
        assert_eq!(result, "The Author/EvilTitleWithSeparators");
    }

    #[test]
    fn value_token_spellings_are_not_re_substituted() {
        let mut m = meta();
        m.title = "{author}/x{book_id}".to_string();
        let result = PathResolver::resolve("{author}/{title}", &m).unwrap();
        assert_eq!(result, "The Author/{author}x{book_id}");
    }

    #[test]
    fn traversal_like_values_cannot_escape() {
        let mut m = meta();
        m.title = "../../etc/evil".to_string();
        let result = PathResolver::resolve("{author}/{title}", &m).unwrap();
        // Separators and dot sequences are removed from the value; no `..`
        // segment survives, so the path stays under the author dir.
        assert_eq!(result, "The Author/etcevil");
        // A bare `..` value sanitizes to empty and drops out entirely.
        let mut n = meta();
        n.title = "..".to_string();
        let result = PathResolver::resolve("{author}/{title}", &n).unwrap();
        assert_eq!(result, "The Author");
    }

    #[test]
    fn leading_dots_cannot_make_hidden_entries() {
        // A value that begins with dots must not survive as a hidden
        // (dot-prefixed) entry, in either a directory component or the
        // file-name component.
        let mut d = meta();
        d.authors = vec![".hidden".to_string()];
        let result = PathResolver::resolve("{author}/{title}", &d).unwrap();
        assert_eq!(result, "hidden/The Book");
        let mut f = meta();
        f.title = ".hidden".to_string();
        let result = PathResolver::resolve("{author}/{title}", &f).unwrap();
        assert_eq!(result, "The Author/hidden");
        // A dotfile-style name must not survive as a hidden entry: the
        // leading dot is stripped, so the job writes a visible file.
        let mut c = meta();
        c.title = ".config".to_string();
        let result = PathResolver::resolve("{title}", &c).unwrap();
        assert_eq!(result, "config");
        assert!(!result.starts_with('.'), "dotfile survived: {result}");
    }

    #[test]
    fn missing_values_contribute_no_segments() {
        let mut m = meta();
        m.subtitle = None;
        m.series_name = None;
        m.series_position = None;
        m.year = None;
        m.genres.clear();
        m.book_id.clear();
        let result = PathResolver::resolve(
            "{author}/{subtitle}/{series_name}/{series_position}/{year}/{genre}/{book_id}/{title}",
            &m,
        )
        .unwrap();
        assert_eq!(result, "The Author/The Book");
    }

    #[test]
    fn empty_template_is_an_error() {
        let m = meta();
        assert!(matches!(
            PathResolver::resolve("", &m),
            Err(PathResolutionError::EmptyPath { .. })
        ));
        assert!(matches!(
            PathResolver::resolve("{subtitle}", &BookMetadata::new("x", "t", "")),
            Err(PathResolutionError::EmptyPath { .. })
        ));
    }

    #[test]
    fn unbalanced_brace_is_literal() {
        let m = meta();
        let result = PathResolver::resolve("{author}/{title}", &m).unwrap();
        assert_eq!(result, "The Author/The Book");
        let result = PathResolver::resolve("The {book", &m).unwrap();
        assert_eq!(result, "The {book");
    }

    #[test]
    fn unicode_titles_survive_as_components() {
        let mut m = meta();
        m.title = "Wünderbare Bücher – 2nd".to_string();
        let result = PathResolver::resolve("{author}/{title}", &m).unwrap();
        assert_eq!(result, "The Author/Wünderbare Bücher – 2nd");
    }
}
