//! Sunday-link deep-link import RECEIVER commands (Rec → Studio handoff).
//!
//! Thin layer over `services::deeplink`. The native deep-link plugin (wired in
//! `lib.rs`) emits the raw inbound URL to the renderer; the renderer calls
//! `deeplink_parse_import` to validate + structure it, then drives the normal
//! take-import flow itself (which lands the file on the timeline as a new take).

use crate::error::AppResult;
use crate::services::deeplink::{parse_import_url, ImportRequest};

/// Parse a `sundaystudio://import?…` URL into a validated [`ImportRequest`].
#[tauri::command]
pub fn deeplink_parse_import(url: String) -> AppResult<ImportRequest> {
    parse_import_url(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_delegates_to_parser() {
        let req = deeplink_parse_import(
            "sundaystudio://import?path=/Users/ola/sermon.wav&returnTo=sundayrec".to_string(),
        )
        .unwrap();
        assert_eq!(req.path, "/Users/ola/sermon.wav");
        assert_eq!(req.return_to.as_deref(), Some("sundayrec"));
    }

    #[test]
    fn command_surfaces_validation_errors() {
        let err = deeplink_parse_import("sundaystudio://import".to_string()).unwrap_err();
        assert_eq!(err.code(), "validation");
    }
}
