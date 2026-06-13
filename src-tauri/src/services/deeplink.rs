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
//! ## Single source of truth
//!
//! The grammar is owned by the canonical `sunday-contracts` crate
//! (`MediaHandoff` / [`parse_handoff_url`], git-tag `v0.4.1`) — the exact same
//! parser SundayEdit and SundayRec speak. We do NOT re-implement it: we call it
//! with our own scheme (`sundaystudio`) and then map the canonical
//! [`MediaHandoff`] onto our [`ImportRequest`]. That keeps all eight fields
//! (path · media_kind · language · context · glossary · service_id · church_id ·
//! return_to) in lockstep with the rest of the suite — a `round_trip_parity`
//! test fails loudly if either side ever drifts.
//!
//! `ImportRequest` stays a SundayStudio-local type rather than re-exporting
//! `MediaHandoff` directly, for two reasons the canonical type deliberately
//! does not cover:
//!   1. **TS bindings** — it derives `ts_rs::TS` to generate the frontend
//!      `ImportRequest.ts` the renderer's typed `invoke` wrapper consumes.
//!   2. **Security** — the deep-link `path` is untrusted (any app or pasted URL
//!      can launch us) and gets copied off disk by the import flow, so we reject
//!      relative paths and `..` traversal here. The wire contract intentionally
//!      stays transport-only and does no filesystem policy.
//!
//! ```text
//! sundaystudio://import
//!   ?path=<absolute path to the source audio/video, REQUIRED>
//!   &media_kind=<"video"|"audio", optional>   (alias: kind)
//!   &language=<ISO code, optional>            (alias: lang)       e.g. "no"
//!   &context=<free-text priming, optional>    e.g. "Sermon, speaker: Ola"
//!   &glossary=<comma-separated terms>         de-duplicated, order preserved
//!   &service_id=<originating service, optional>
//!   &church_id=<originating tenant, optional>
//!   &returnTo=<caller scheme, optional>       (alias: return_to)  e.g. "sundayrec"
//! ```
//!
//! Everything is percent-decoded (`+` is also treated as a space, per the
//! `application/x-www-form-urlencoded` convention). Unknown query keys are
//! ignored so the contract can grow without breaking older builds. Do not add or
//! rename fields here — change the canonical `sunday-contracts` contract first.

use serde::{Deserialize, Serialize};
use sunday_contracts::{parse_handoff_url, MediaHandoff, MediaKind};
use ts_rs::TS;

use crate::error::{AppError, AppResult};

/// The scheme SundayStudio registers for inbound deep links.
pub const SCHEME: &str = "sundaystudio";
/// The only action understood today.
pub const ACTION_IMPORT: &str = "import";

/// A validated request to import a recording into a project, parsed from a
/// `sundaystudio://import?…` deep link. The renderer turns this into a real
/// take/project via the normal import flow. Carries the full canonical
/// `MediaHandoff` field set (sunday-contracts v0.4.x).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ImportRequest.ts")]
pub struct ImportRequest {
    /// Absolute path to the source audio/video file. Always present.
    pub path: String,
    /// `"video"` or `"audio"`, when the caller declared it; anything else
    /// degrades to `None` rather than failing the import.
    pub media_kind: Option<String>,
    /// ISO language code of the recording, if the caller specified one.
    pub language: Option<String>,
    /// Free-text priming for context-aware processing.
    pub context: Option<String>,
    /// Glossary terms (speaker names, jargon) — de-duplicated, order preserved.
    pub glossary: Vec<String>,
    /// Originating service id, so the project can link back to it.
    pub service_id: Option<String>,
    /// Originating tenant id.
    pub church_id: Option<String>,
    /// Scheme of the app that launched us, so we can hand a result back later
    /// (e.g. `"sundayrec"`). `None` for a plain user-initiated link.
    pub return_to: Option<String>,
}

/// Render a canonical [`MediaKind`] as the lowercase string the renderer and TS
/// bindings expect (`"video"` / `"audio"`).
fn media_kind_str(kind: MediaKind) -> String {
    match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
    }
    .to_string()
}

impl From<MediaHandoff> for ImportRequest {
    /// Project the canonical handoff onto our local type. The canonical `action`
    /// field is dropped (always `"import"`; our scheme + parser already pin it),
    /// and `media_kind` is flattened to its wire string for the TS bindings.
    fn from(h: MediaHandoff) -> Self {
        ImportRequest {
            path: h.path,
            media_kind: h.media_kind.map(media_kind_str),
            language: h.language,
            context: h.context,
            glossary: h.glossary,
            service_id: h.service_id,
            church_id: h.church_id,
            return_to: h.return_to,
        }
    }
}

/// Parse a `sundaystudio://import?…` URL into an [`ImportRequest`].
///
/// Delegates the grammar to the canonical `sunday-contracts` parser, then
/// applies SundayStudio's own untrusted-path security policy on top. Returns
/// [`AppError::Validation`] for anything that isn't a well-formed import link
/// with a clean, absolute `path`.
pub fn parse_import_url(url: &str) -> AppResult<ImportRequest> {
    // The canonical parser owns the scheme/action/query grammar and the codec.
    // It already rejects the wrong scheme, the wrong action, and a missing or
    // empty `path` — we surface those as our own `Validation` errors.
    let handoff =
        parse_handoff_url(url, SCHEME).map_err(|e| AppError::Validation(e.to_string()))?;

    // SECURITY: the link is untrusted (any app or pasted URL can launch us), and
    // `path` is copied straight off disk by the import flow. Only accept a clean
    // absolute path so a crafted link can't traverse out (`../../etc/passwd`) or
    // read a file relative to our working directory. The wire contract stays
    // transport-only, so this policy lives here, not in `sunday-contracts`.
    validate_import_path(&handoff.path)?;

    Ok(ImportRequest::from(handoff))
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

#[cfg(test)]
mod tests {
    use super::*;
    use sunday_contracts::build_handoff_url;

    #[test]
    fn parses_a_full_link() {
        // Exactly what SundayRec sends today.
        let url = "sundaystudio://import?path=%2FUsers%2Fola%2Fsermon.wav&returnTo=sundayrec";
        let req = parse_import_url(url).unwrap();
        assert_eq!(req.path, "/Users/ola/sermon.wav");
        assert_eq!(req.return_to.as_deref(), Some("sundayrec"));
        // Fields the caller didn't send stay empty/None.
        assert_eq!(req.media_kind, None);
        assert_eq!(req.language, None);
        assert_eq!(req.context, None);
        assert!(req.glossary.is_empty());
        assert_eq!(req.service_id, None);
        assert_eq!(req.church_id, None);
    }

    #[test]
    fn parses_the_full_canonical_media_handoff_field_set() {
        // The complete canonical MediaHandoff grammar (sunday-contracts v0.4.x).
        let url = "sundaystudio://import?path=%2FUsers%2Fola%2Fsermon.mp4\
                   &media_kind=video&language=no\
                   &context=Preken%2C%20taler%3A%20Ola%20Nordmann\
                   &glossary=Ola%20Nordmann,kerygma,%20kerygma%20,Agape\
                   &service_id=svc-123&church_id=ch-456&returnTo=sundayrec";
        let req = parse_import_url(url).unwrap();
        assert_eq!(req.path, "/Users/ola/sermon.mp4");
        assert_eq!(req.media_kind.as_deref(), Some("video"));
        assert_eq!(req.language.as_deref(), Some("no"));
        assert_eq!(req.context.as_deref(), Some("Preken, taler: Ola Nordmann"));
        // De-duplicated case-insensitively, order preserved.
        assert_eq!(req.glossary, vec!["Ola Nordmann", "kerygma", "Agape"]);
        assert_eq!(req.service_id.as_deref(), Some("svc-123"));
        assert_eq!(req.church_id.as_deref(), Some("ch-456"));
        assert_eq!(req.return_to.as_deref(), Some("sundayrec"));
    }

    /// The drift guard: every canonical field must survive the canonical
    /// build → our parse → our `ImportRequest` round trip. If `sunday-contracts`
    /// grows or renames a field and our conversion doesn't carry it, this fails.
    #[test]
    fn round_trip_parity_carries_all_eight_fields() {
        let canonical = MediaHandoff {
            action: ACTION_IMPORT.to_string(),
            path: "/Users/ola/My Talk (2026).mov".to_string(),
            media_kind: Some(MediaKind::Video),
            language: Some("no".to_string()),
            context: Some("Sermon, speaker: Ola".to_string()),
            glossary: vec!["Ola".to_string(), "kerygma".to_string()],
            service_id: Some("svc-1".to_string()),
            church_id: Some("ch-1".to_string()),
            return_to: Some("sundayrec".to_string()),
        };
        // Build the wire URL with the canonical encoder, parse it back with our
        // receiver, and assert each field landed.
        let url = build_handoff_url(SCHEME, &canonical);
        let req = parse_import_url(&url).unwrap();

        assert_eq!(req.path, canonical.path);
        assert_eq!(req.media_kind.as_deref(), Some("video"));
        assert_eq!(req.language, canonical.language);
        assert_eq!(req.context, canonical.context);
        assert_eq!(req.glossary, canonical.glossary);
        assert_eq!(req.service_id, canonical.service_id);
        assert_eq!(req.church_id, canonical.church_id);
        assert_eq!(req.return_to, canonical.return_to);

        // And the direct `From` projection must agree with the parsed result, so
        // the conversion is the single mapping (no second code path can drift).
        assert_eq!(ImportRequest::from(canonical), req);
    }

    #[test]
    fn media_kind_accepts_the_kind_alias_and_degrades_unknown_values() {
        let req = parse_import_url("sundaystudio://import?path=/a.wav&kind=AUDIO").unwrap();
        assert_eq!(req.media_kind.as_deref(), Some("audio"));
        // An unknown kind never fails the import — it just degrades to None.
        let req =
            parse_import_url("sundaystudio://import?path=/a.wav&media_kind=hologram").unwrap();
        assert_eq!(req.media_kind, None);
    }

    #[test]
    fn language_accepts_the_lang_alias() {
        let req = parse_import_url("sundaystudio://import?path=/a.wav&lang=no").unwrap();
        assert_eq!(req.language.as_deref(), Some("no"));
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
    fn plus_decodes_to_space() {
        // x-www-form-urlencoded convention: a literal '+' in the value is a space.
        let req = parse_import_url("sundaystudio://import?path=/a+b/c.wav").unwrap();
        assert_eq!(req.path, "/a b/c.wav");
    }
}
