//! Machine-readable schema and resolver identity plus supported token and
//! field-policy capabilities.
//!
//! The `--capabilities` command prints this document as JSON. Callers use it
//! to build pickers, examples, and validation against the engine's actual
//! vocabulary instead of maintaining their own registry, and to reject
//! protocol/resolver versions they do not understand.

use serde::Serialize;

use crate::job::SCHEMA_VERSION;
use crate::path_resolver::{RESOLVER_VERSION, TokenKind};

/// The identity and capability surface this build offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// The structured job schema version accepted by this build.
    pub schema_version: u32,
    /// The output-path resolver version: token set and substitution
    /// semantics. Callers must not assume an older build's token behavior;
    /// a queued job that pinned a different value must be re-previewed.
    pub resolver_version: &'static str,
    /// The default output template used when a job omits `path_format`.
    pub default_path_format: &'static str,
    /// Every supported template token with its substitution semantics.
    pub path_tokens: Vec<TokenCapability>,
    /// The per-field write policies a job can select.
    pub field_policies: FieldPolicies,
    /// The cover policies a job can select.
    pub cover_policies: CoverPolicies,
    /// The chapter policies a job can select.
    pub chapter_policies: ChapterPolicies,
}

/// One supported path token and what it substitutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenCapability {
    pub name: &'static str,
    pub value: &'static str,
    pub example: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldPolicies {
    pub values: Vec<&'static str>,
    /// Fields whose policy is `preserve` | `write` | `clear`.
    pub scalar_fields: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverPolicies {
    pub variants: Vec<&'static str>,
    pub constraint: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChapterPolicies {
    pub variants: Vec<&'static str>,
    pub constraint: &'static str,
}

impl Capabilities {
    /// Build the capability document for this build.
    pub fn build() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            resolver_version: RESOLVER_VERSION,
            default_path_format: crate::path_resolver::DEFAULT_TEMPLATE,
            path_tokens: token_capabilities(),
            field_policies: FieldPolicies {
                values: vec!["preserve", "write", "clear"],
                scalar_fields: vec![
                    "title",
                    "subtitle",
                    "authors",
                    "narrators",
                    "series",
                    "description",
                    "genres",
                    "year",
                ],
            },
            cover_policies: CoverPolicies {
                // Advertise only the bare spellings `CoverPolicy` parses;
                // its `{"write": "/absolute/path"}` object form is named in
                // the constraint text, not in the machine variant list.
                variants: vec!["write_url", "preserve", "clear"],
                constraint: "write also accepts the object form {\"write\": \"/absolute/path\"} for a local cover file; preserve requires a single natively-copyable input file",
            },
            chapter_policies: ChapterPolicies {
                variants: vec!["write", "preserve", "clear"],
                constraint: "preserve requires a single natively-copyable input file",
            },
        }
    }

    /// The stable JSON document, for `--capabilities`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("capabilities are fully serializable")
    }
}

fn token_capabilities() -> Vec<TokenCapability> {
    TokenKind::supported_tokens()
        .iter()
        .map(|kind| {
            let (value, example) = match kind {
                TokenKind::Author => {
                    ("first ordered author name; `Unknown` when none", "The Author")
                }
                TokenKind::Authors => (
                    "all ordered authors joined by \", \"; `Unknown` when none",
                    "The Author, Second Author",
                ),
                TokenKind::Narrator => {
                    ("first ordered narrator name; `Unknown` when none", "The Narrator")
                }
                TokenKind::Narrators => {
                    ("all ordered narrators joined by \", \"; `Unknown` when none", "The Narrator")
                }
                TokenKind::Title => ("base title, trimmed", "The Book"),
                TokenKind::Subtitle => {
                    ("subtitle when present; no segment when absent", "A Subtitle")
                }
                TokenKind::SeriesName => ("series name when present", "The Series"),
                TokenKind::SeriesPosition => ("series position when present", "1"),
                TokenKind::Year => ("release year when present", "2021"),
                TokenKind::Genre => ("first genre when present", "Mystery"),
                TokenKind::MetadataId => (
                    "provider-dependent id the book was looked up by (ASIN for audnexus, resolved \
                 catalog id for audiobookdb); no segment when absent",
                    "B08XYZ1234",
                ),
                TokenKind::BookId => (
                    "canonical AudiobookDB book id, distinct from metadata_id and ASIN; populated \
                 from an explicit provider lookup or a supplied/override book_id (used as-is, \
                 never catalog-authenticated); no segment when absent",
                    "575",
                ),
            };
            TokenCapability { name: kind.spelling(), value, example }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_document_is_stable_json() {
        let caps = Capabilities::build();
        let json = caps.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["resolver_version"], RESOLVER_VERSION);
        assert_eq!(parsed["default_path_format"], "{author}/{title}");
        // Every token the resolver accepts is advertised, and nothing else.
        let advertised: Vec<String> = parsed["path_tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            advertised,
            crate::path_resolver::PathResolver::supported_tokens()
                .iter()
                .map(|k| k.spelling().to_string())
                .collect::<Vec<_>>()
        );
        // Advertised variants are the parser's vocabulary exactly: a
        // spelling the job parser rejects here (like the cover `write`
        // object form) must not be advertised.
        assert_eq!(
            parsed["cover_policies"]["variants"],
            serde_json::json!(["write_url", "preserve", "clear"])
        );
        assert_eq!(
            parsed["chapter_policies"]["variants"],
            serde_json::json!(["write", "preserve", "clear"])
        );
    }
}
