//! Sunday-link deep-link import RECEIVER (Rec → Studio handoff).
//!
//! SundayRec finishes a recording and hands the audio straight into SundayStudio
//! by launching us with a `sundaystudio://import?path=…&returnTo=sundayrec` URL,
//! so the take lands in a fresh project without the user re-locating the file.
//! This module is the pure, testable core: it turns the raw URL into a validated
//! [`ImportRequest`]. The native plumbing — OS scheme registration, app
//! lifecycle, emitting the parsed request to the renderer — lives in `lib.rs`;
//! the renderer drives the actual import via the existing project/take flows.
//!
//! Contract (mirrors sunday-contracts `MediaHandoff`; converge once published —
//! today we own the post-name `sundaystudio://` scheme and SundayRec already
//! sends the shape below):
//!
//! ```text
//! sundaystudio://import
//!   ?path=<absolute path to the source audio/video, REQUIRED>
//!   &returnTo=<caller scheme, optional>       e.g. "sundayrec"
//! ```
//!
//! Everything is percent-decoded (`+` is also treated as a space, per the
//! `application/x-www-form-urlencoded` convention). Unknown query keys are
//! ignored so the contract can grow (language, context, …) without breaking
//! older builds — mirrors the same forward-compatibility SundayEdit relies on.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, AppResult};

/// The scheme SundayStudio registers for inbound deep links.
pub const SCHEME: &str = "sundaystudio";
/// The only action understood today.
pub const ACTION_IMPORT: &str = "import";

/// A validated request to import a recording into a project, parsed from a
/// `sundaystudio://import?…` deep link. The renderer turns this into a real
/// take/project via the normal import flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ImportRequest.ts")]
pub struct ImportRequest {
    /// Absolute path to the source audio/video file. Always present.
    pub path: String,
    /// Scheme of the app that launched us, so we can hand a result back later
    /// (e.g. `"sundayrec"`). `None` for a plain user-initiated link.
    pub return_to: Option<String>,
}

/// Parse a `sundaystudio://import?…` URL into an [`ImportRequest`].
///
/// Returns [`AppError::Validation`] for anything that isn't a well-formed
/// import link with a non-empty `path`.
pub fn parse_import_url(url: &str) -> AppResult<ImportRequest> {
    let trimmed = url.trim();

    // Strip the scheme (case-insensitive), tolerating `://` or a bare `:`.
    let rest = strip_scheme(trimmed, SCHEME)
        .ok_or_else(|| AppError::Validation(format!("not a {SCHEME}:// link: {url}")))?;

    // Split `action[?query]`. The action is everything up to the first `?`;
    // a trailing slash (`import/?…`) is tolerated.
    let (action_part, query) = match rest.split_once('?') {
        Some((a, q)) => (a, q),
        None => (rest, ""),
    };
    let action = action_part.trim_end_matches('/').trim_start_matches('/');
    if !action.eq_ignore_ascii_case(ACTION_IMPORT) {
        return Err(AppError::Validation(format!(
            "unsupported deep-link action: {action:?} (expected {ACTION_IMPORT:?})"
        )));
    }

    let mut path: Option<String> = None;
    let mut return_to: Option<String> = None;

    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let (raw_key, raw_val) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode_component(raw_key);
        let value = decode_component(raw_val);
        match key.as_str() {
            "path" => path = non_empty(value),
            "returnTo" | "return_to" => return_to = non_empty(value),
            _ => {} // forward-compatible: ignore unknown keys
        }
    }

    let path = path.ok_or_else(|| {
        AppError::Validation("deep-link import is missing a non-empty `path`".into())
    })?;

    // SECURITY: the link is untrusted (any app or pasted URL can launch us), and
    // `path` is copied straight off disk by the import flow. Only accept a clean
    // absolute path so a crafted link can't traverse out (`../../etc/passwd`) or
    // read a file relative to our working directory.
    validate_import_path(&path)?;

    Ok(ImportRequest { path, return_to })
}

/// Reject any import path that isn't a clean absolute path: a relative path, or
/// one containing a `..` (parent-dir) component. This is the choke point that
/// turns the untrusted deep-link `path` into something safe to copy off disk.
///
/// Absolute is recognised cross-platform: a leading `/` (POSIX) or a Windows
/// drive/UNC prefix (`C:\…`, `\\server\…`). Legit Rec → Studio handoffs always
/// send an absolute file path, so this preserves every real link.
fn validate_import_path(path: &str) -> AppResult<()> {
    if !is_absolute_path(path) {
        return Err(AppError::Validation(format!(
            "deep-link import path must be absolute: {path:?}"
        )));
    }
    // Reject a `..` *component* on either separator. Splitting on both `/` and
    // `\` catches traversal regardless of the platform that produced the link.
    if path.split(['/', '\\']).any(|seg| seg == "..") {
        return Err(AppError::Validation(format!(
            "deep-link import path must not contain '..': {path:?}"
        )));
    }
    Ok(())
}

/// True for a POSIX-absolute path (`/…`) or a Windows absolute path: a drive
/// (`C:\…` / `C:/…`) or a UNC share (`\\server\…`).
fn is_absolute_path(path: &str) -> bool {
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    // `X:\…` or `X:/…` — a single ASCII drive letter, a colon, then a separator.
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// If `s` begins with `scheme:` (case-insensitive), return the remainder with
/// any leading `//` removed. Otherwise `None`.
fn strip_scheme<'a>(s: &'a str, scheme: &str) -> Option<&'a str> {
    let prefix_len = scheme.len() + 1; // "+ 1" for the ':'
    if s.len() < prefix_len {
        return None;
    }
    let (head, tail) = s.split_at(prefix_len);
    let (name, colon) = head.split_at(scheme.len());
    if colon != ":" || !name.eq_ignore_ascii_case(scheme) {
        return None;
    }
    Some(tail.strip_prefix("//").unwrap_or(tail))
}

/// `Some(trimmed)` if non-empty after trimming, else `None`.
fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Percent-encode a string as a URL query-component value: RFC 3986 unreserved
/// characters (`A-Z a-z 0-9 - _ . ~`) pass through, everything else (including
/// `/`, spaces and non-ASCII) becomes `%XX`. The exact inverse of
/// [`decode_component`] for any input (spaces round-trip via `%20`, never `+`).
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(hex_digit(b >> 4));
                out.push(hex_digit(b & 0x0f));
            }
        }
    }
    out
}

fn hex_digit(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

/// Percent-decode one query component. `%XX` → byte, `+` → space, everything
/// else verbatim. Invalid `%` escapes are left as-is rather than rejected, so a
/// stray `%` in a path never sinks the whole import. Bytes are reassembled and
/// read as UTF-8 (lossily) at the end.
fn decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_link() {
        // Exactly what SundayRec sends today.
        let url = "sundaystudio://import?path=%2FUsers%2Fola%2Fsermon.wav&returnTo=sundayrec";
        let req = parse_import_url(url).unwrap();
        assert_eq!(req.path, "/Users/ola/sermon.wav");
        assert_eq!(req.return_to.as_deref(), Some("sundayrec"));
    }

    #[test]
    fn path_is_required() {
        let err = parse_import_url("sundaystudio://import?returnTo=sundayrec").unwrap_err();
        assert_eq!(err.code(), "validation");
        // An empty/whitespace path counts as missing.
        assert!(parse_import_url("sundaystudio://import?path=%20%20").is_err());
        // No query at all is also missing a path.
        assert!(parse_import_url("sundaystudio://import").is_err());
    }

    #[test]
    fn rejects_traversal_and_relative_paths() {
        // SECURITY: the `path` comes from an *untrusted* deep link — any other app
        // (or a pasted link) can launch us with `sundaystudio://import?path=…`, and
        // that path is copied off disk verbatim by `take_import`. A `..` sequence
        // or a relative path must be rejected so a crafted link can't read e.g.
        // `../../../../etc/passwd` or a file relative to the app's cwd.
        for bad in [
            "sundaystudio://import?path=../../../../etc/passwd",
            "sundaystudio://import?path=%2E%2E%2F%2E%2E%2Fetc%2Fpasswd", // encoded ../../
            "sundaystudio://import?path=/Users/ola/../../../etc/passwd",
            "sundaystudio://import?path=relative/take.wav",
            "sundaystudio://import?path=take.wav",
        ] {
            assert!(
                parse_import_url(bad).is_err(),
                "traversal/relative path must be rejected: {bad}"
            );
        }
    }

    #[test]
    fn accepts_clean_absolute_paths() {
        // The legitimate Rec → Studio handoff: a plain absolute path with no `..`.
        let req =
            parse_import_url("sundaystudio://import?path=%2FUsers%2Fola%2Fsermon.wav").unwrap();
        assert_eq!(req.path, "/Users/ola/sermon.wav");
        // A Windows absolute path is also fine.
        let req = parse_import_url("sundaystudio://import?path=C:\\Users\\Ola\\take.wav").unwrap();
        assert_eq!(req.path, "C:\\Users\\Ola\\take.wav");
    }

    #[test]
    fn rejects_wrong_scheme_and_action() {
        assert!(parse_import_url("https://import?path=/a.wav").is_err());
        assert!(parse_import_url("sundayrec://import?path=/a.wav").is_err());
        assert!(parse_import_url("sundaystudio://export?path=/a.wav").is_err());
        // A near-miss scheme (prefix of ours) must not match.
        assert!(parse_import_url("sunday://import?path=/a.wav").is_err());
    }

    #[test]
    fn scheme_and_action_are_case_insensitive() {
        let req = parse_import_url("SundayStudio://Import?path=/a.wav").unwrap();
        assert_eq!(req.path, "/a.wav");
    }

    #[test]
    fn tolerates_trailing_slash_after_action() {
        let req = parse_import_url("sundaystudio://import/?path=/a.wav").unwrap();
        assert_eq!(req.path, "/a.wav");
    }

    #[test]
    fn optional_return_to_defaults_to_none() {
        let req = parse_import_url("sundaystudio://import?path=/a.wav").unwrap();
        assert_eq!(req.return_to, None);
    }

    #[test]
    fn accepts_return_to_alias() {
        let req =
            parse_import_url("sundaystudio://import?path=/a.wav&return_to=sundaystage").unwrap();
        assert_eq!(req.return_to.as_deref(), Some("sundaystage"));
    }

    #[test]
    fn ignores_unknown_keys() {
        // Forward-compatible: a future caller can add fields we don't know yet.
        let req =
            parse_import_url("sundaystudio://import?path=/a.wav&language=no&futureFlag=1").unwrap();
        assert_eq!(req.path, "/a.wav");
        assert_eq!(req.return_to, None);
    }

    #[test]
    fn lone_percent_is_left_intact() {
        // A stray '%' (not a valid escape) must not lose the rest of the path.
        let req = parse_import_url("sundaystudio://import?path=/a%b/c.wav").unwrap();
        assert_eq!(req.path, "/a%b/c.wav");
    }

    #[test]
    fn encode_decode_round_trips() {
        for s in [
            "/Users/ola/My Sermon (2026).wav",
            "C:\\Users\\Ola\\take.flac",
            "jingle + søndag/æøå",
            "",
        ] {
            assert_eq!(
                decode_component(&encode_component(s)),
                s,
                "round-trip {s:?}"
            );
        }
        // Spaces encode as %20 (not +), so they survive the +→space decode rule.
        assert_eq!(encode_component("a b"), "a%20b");
    }

    #[test]
    fn plus_decodes_to_space() {
        // x-www-form-urlencoded convention: a literal '+' in the value is a space.
        let req = parse_import_url("sundaystudio://import?path=/a+b/c.wav").unwrap();
        assert_eq!(req.path, "/a b/c.wav");
    }
}
