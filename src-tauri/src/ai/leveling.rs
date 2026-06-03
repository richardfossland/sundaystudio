//! AI auto-leveling (Phase 5.1).
//!
//! The first AI move offered after import: Claude looks at a [`LevelingSnapshot`]
//! — one row per track with its current gain, measured loudness and clip count —
//! and returns a [`LevelingSuggestion`] per track: a suggested gain (dB) plus a
//! one-line human reason. The renderer shows these as preview chips ("Pulpit mic
//! −3.5 dB — loudest track, pull it down to sit with the others") and applies the
//! ones the user accepts.
//!
//! Everything here except the actual network send is pure and unit-tested:
//! - [`build_request_body`] turns a snapshot into the Anthropic Messages JSON.
//! - [`parse_response`] reads Claude's reply back into suggestions and sanity-
//!   clamps them (so a hallucinated +40 dB can never blast a track).
//! - [`auto_level`] wires the two around an [`HttpTransport`], so a test drives
//!   the whole flow with a canned response and no key / no network.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{HttpTransport, ANTHROPIC_MESSAGES_URL, ANTHROPIC_VERSION, LEVELING_MODEL};

/// The largest gain change we'll ever apply from an AI suggestion, in dB. Claude
/// is asked to stay within this; we clamp regardless so a bad reply is harmless.
pub const MAX_GAIN_ADJUST_DB: f64 = 12.0;

/// One track's current state, fed to the model. Loudness is optional because a
/// freshly-imported or empty track may not have been measured yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LevelingTrack.ts")]
pub struct LevelingTrack {
    /// Stable track id (echoed back so we can map a suggestion to its track).
    pub track_id: String,
    /// Display name — the model leans on this for its human reason ("Guest mic").
    pub name: String,
    /// The track's current fader gain in dB.
    pub current_gain_db: f64,
    /// Integrated loudness in LUFS, if measured (lower = quieter).
    pub integrated_lufs: Option<f32>,
    /// True-peak in dBTP, if measured (near 0 = at risk of clipping).
    pub true_peak_dbtp: Option<f32>,
    /// How many clips this track holds on the timeline (0 = empty track).
    pub clip_count: u32,
}

/// The whole project's leveling input — the tracks plus the loudness target the
/// final mix is aiming for, so the model can level *toward* the platform goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LevelingSnapshot.ts")]
pub struct LevelingSnapshot {
    pub tracks: Vec<LevelingTrack>,
    /// The integrated-LUFS goal of the chosen platform target (e.g. -16 for
    /// Spotify), so suggestions push the mix the right direction.
    pub target_lufs: f32,
}

/// One per-track suggestion the model returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LevelingSuggestion.ts")]
pub struct LevelingSuggestion {
    /// Which track this applies to (matches a [`LevelingTrack::track_id`]).
    pub track_id: String,
    /// The suggested new fader gain in dB (absolute, not a delta) — already
    /// clamped to a safe range relative to the track's current gain.
    pub suggested_gain_db: f64,
    /// A short, human reason the UI shows next to the chip.
    pub reason: String,
}

/// The full result returned to the renderer: the suggestions plus the model that
/// produced them (so the UI can label "Suggested by Claude").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LevelingResult.ts")]
pub struct LevelingResult {
    pub suggestions: Vec<LevelingSuggestion>,
    pub model: String,
}

/// Build the Anthropic Messages request body for a leveling snapshot.
///
/// We hand Claude the numbers as JSON and ask for a strict JSON reply — a
/// `suggestions` array of `{track_id, suggested_gain_db, reason}`. Keeping the
/// schema explicit in the prompt is what makes [`parse_response`] reliable.
pub fn build_request_body(snapshot: &LevelingSnapshot) -> serde_json::Value {
    let tracks_json =
        serde_json::to_string_pretty(&snapshot.tracks).unwrap_or_else(|_| "[]".to_string());

    let prompt = format!(
        "You are an audio engineer leveling a multi-track podcast recording.\n\
         The final mix targets {target} LUFS integrated.\n\
         Here are the tracks (current fader gains, measured loudness, clip counts):\n\
         {tracks}\n\n\
         For each track that has audio (clip_count > 0), suggest a new absolute \
         fader gain in dB so the tracks sit balanced with each other and the mix \
         heads toward the target loudness. Louder tracks should get a lower gain, \
         quieter tracks a higher gain. Keep every change within {max} dB of the \
         track's current gain. Skip empty tracks.\n\n\
         Reply with ONLY a JSON object of the form:\n\
         {{\"suggestions\": [{{\"track_id\": \"...\", \"suggested_gain_db\": -3.5, \
         \"reason\": \"one short sentence\"}}]}}\n\
         No prose, no markdown fences.",
        target = snapshot.target_lufs,
        tracks = tracks_json,
        max = MAX_GAIN_ADJUST_DB,
    );

    serde_json::json!({
        "model": LEVELING_MODEL,
        "max_tokens": 1024,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    })
}

/// The shape of Claude's reply inside the Messages-API envelope. We only read the
/// text content block; the rest of the envelope is ignored.
#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

/// The JSON object we ask Claude to emit.
#[derive(Deserialize)]
struct SuggestionsPayload {
    suggestions: Vec<RawSuggestion>,
}

#[derive(Deserialize)]
struct RawSuggestion {
    track_id: String,
    suggested_gain_db: f64,
    #[serde(default)]
    reason: String,
}

/// Parse an Anthropic Messages-API response body into clamped suggestions for
/// the tracks in `snapshot`.
///
/// Robust by construction:
/// - reads the first `text` content block and parses the JSON object out of it
///   (tolerating stray prose around it by slicing to the outermost braces);
/// - drops suggestions for unknown track ids (anti-hallucination);
/// - clamps each suggested gain to within [`MAX_GAIN_ADJUST_DB`] of the track's
///   current gain, so a wild number can never damage a mix.
pub fn parse_response(
    body: &str,
    snapshot: &LevelingSnapshot,
) -> Result<Vec<LevelingSuggestion>, String> {
    let resp: AnthropicResponse =
        serde_json::from_str(body).map_err(|e| format!("decoding Anthropic envelope: {e}"))?;

    let text = resp
        .content
        .iter()
        .find(|b| b.kind == "text")
        .map(|b| b.text.as_str())
        .ok_or_else(|| "Anthropic reply had no text content block".to_string())?;

    let json_slice = extract_json_object(text)
        .ok_or_else(|| "no JSON object found in the model's reply".to_string())?;

    let payload: SuggestionsPayload =
        serde_json::from_str(json_slice).map_err(|e| format!("parsing suggestions JSON: {e}"))?;

    let out = payload
        .suggestions
        .into_iter()
        .filter_map(|raw| {
            // Only accept suggestions for tracks we actually sent — never let the
            // model invent a track id.
            let track = snapshot
                .tracks
                .iter()
                .find(|t| t.track_id == raw.track_id)?;
            let clamped = clamp_gain(raw.suggested_gain_db, track.current_gain_db);
            Some(LevelingSuggestion {
                track_id: raw.track_id,
                suggested_gain_db: clamped,
                reason: raw.reason,
            })
        })
        .collect();

    Ok(out)
}

/// Clamp a suggested absolute gain to within `MAX_GAIN_ADJUST_DB` of the track's
/// current gain.
fn clamp_gain(suggested: f64, current: f64) -> f64 {
    let lo = current - MAX_GAIN_ADJUST_DB;
    let hi = current + MAX_GAIN_ADJUST_DB;
    suggested.clamp(lo, hi)
}

/// Slice out the first top-level `{...}` object from a string, tolerating
/// leading/trailing prose or markdown fences the model might add despite the
/// instruction not to.
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end > start {
        Some(&text[start..=end])
    } else {
        None
    }
}

/// The complete leveling call: build the request, send it through `transport`,
/// and parse the reply into a [`LevelingResult`].
///
/// `transport` is the seam: production passes a `ReqwestTransport`, tests pass a
/// mock. `api_key` is the caller's Anthropic key. A non-2xx HTTP status surfaces
/// the API's error body so the UI can show "your key is invalid" etc.
pub fn auto_level(
    transport: &dyn HttpTransport,
    api_key: &str,
    snapshot: &LevelingSnapshot,
) -> Result<LevelingResult, String> {
    if snapshot.tracks.is_empty() {
        return Err("no tracks to level".to_string());
    }

    let body = build_request_body(snapshot).to_string();
    let headers = [
        ("x-api-key", api_key),
        ("anthropic-version", ANTHROPIC_VERSION),
    ];

    let (status, resp_body) = transport.post_json(ANTHROPIC_MESSAGES_URL, &headers, &body)?;
    if !(200..300).contains(&status) {
        return Err(format!("Anthropic API returned HTTP {status}: {resp_body}"));
    }

    let suggestions = parse_response(&resp_body, snapshot)?;
    Ok(LevelingResult {
        suggestions,
        model: LEVELING_MODEL.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn track(id: &str, name: &str, gain: f64, lufs: f32, clips: u32) -> LevelingTrack {
        LevelingTrack {
            track_id: id.to_string(),
            name: name.to_string(),
            current_gain_db: gain,
            integrated_lufs: Some(lufs),
            true_peak_dbtp: Some(-1.0),
            clip_count: clips,
        }
    }

    fn two_track_snapshot() -> LevelingSnapshot {
        LevelingSnapshot {
            tracks: vec![
                // Loud track (closer to 0 LUFS).
                track("loud", "Pulpit mic", 0.0, -12.0, 3),
                // Quiet track.
                track("quiet", "Guest mic", 0.0, -26.0, 2),
            ],
            target_lufs: -16.0,
        }
    }

    /// A canned Anthropic Messages reply, with `text` set to `inner`.
    fn anthropic_envelope(inner: &str) -> String {
        serde_json::json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": LEVELING_MODEL,
            "content": [ { "type": "text", "text": inner } ],
            "stop_reason": "end_turn"
        })
        .to_string()
    }

    /// What a [`MockTransport`] recorded about the request it was handed.
    struct SeenRequest {
        url: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    /// A mock transport that returns a fixed `(status, body)` and records the
    /// request it was given, so we can assert on headers/body too.
    struct MockTransport {
        status: u16,
        body: String,
        seen: RefCell<Option<SeenRequest>>,
    }

    impl MockTransport {
        fn ok(body: String) -> Self {
            Self {
                status: 200,
                body,
                seen: RefCell::new(None),
            }
        }
    }

    impl HttpTransport for MockTransport {
        fn post_json(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            body: &str,
        ) -> Result<(u16, String), String> {
            *self.seen.borrow_mut() = Some(SeenRequest {
                url: url.to_string(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: body.to_string(),
            });
            Ok((self.status, self.body.clone()))
        }
    }

    #[test]
    fn parses_a_known_response_into_reasonable_gains() {
        let snapshot = two_track_snapshot();
        let inner = serde_json::json!({
            "suggestions": [
                { "track_id": "loud", "suggested_gain_db": -4.0, "reason": "loudest track" },
                { "track_id": "quiet", "suggested_gain_db": 5.0, "reason": "quiet, bring it up" }
            ]
        })
        .to_string();

        let suggestions = parse_response(&anthropic_envelope(&inner), &snapshot).unwrap();
        assert_eq!(suggestions.len(), 2);

        let loud = suggestions.iter().find(|s| s.track_id == "loud").unwrap();
        let quiet = suggestions.iter().find(|s| s.track_id == "quiet").unwrap();

        // The louder track gets a negative gain; the quieter one a positive gain.
        assert!(loud.suggested_gain_db < 0.0, "loud track pulled down");
        assert!(quiet.suggested_gain_db > 0.0, "quiet track brought up");
        assert!(quiet.suggested_gain_db > loud.suggested_gain_db);
    }

    #[test]
    fn clamps_wild_suggestions_to_a_safe_range() {
        let snapshot = two_track_snapshot();
        let inner = serde_json::json!({
            "suggestions": [
                { "track_id": "loud", "suggested_gain_db": -40.0, "reason": "way too much" },
                { "track_id": "quiet", "suggested_gain_db": 99.0, "reason": "blast it" }
            ]
        })
        .to_string();

        let suggestions = parse_response(&anthropic_envelope(&inner), &snapshot).unwrap();
        for s in &suggestions {
            // Current gain is 0.0, so the clamp window is ±MAX_GAIN_ADJUST_DB.
            assert!(s.suggested_gain_db >= -MAX_GAIN_ADJUST_DB);
            assert!(s.suggested_gain_db <= MAX_GAIN_ADJUST_DB);
        }
    }

    #[test]
    fn drops_suggestions_for_unknown_tracks() {
        let snapshot = two_track_snapshot();
        let inner = serde_json::json!({
            "suggestions": [
                { "track_id": "loud", "suggested_gain_db": -3.0, "reason": "ok" },
                { "track_id": "ghost", "suggested_gain_db": -3.0, "reason": "hallucinated" }
            ]
        })
        .to_string();

        let suggestions = parse_response(&anthropic_envelope(&inner), &snapshot).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].track_id, "loud");
    }

    #[test]
    fn tolerates_prose_and_fences_around_the_json() {
        let snapshot = two_track_snapshot();
        let inner = "Sure! Here are my suggestions:\n```json\n{\"suggestions\": \
            [{\"track_id\": \"loud\", \"suggested_gain_db\": -2.0, \"reason\": \"x\"}]}\n```";

        let suggestions = parse_response(&anthropic_envelope(inner), &snapshot).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].suggested_gain_db, -2.0);
    }

    #[test]
    fn auto_level_drives_the_whole_flow_with_a_mock() {
        let snapshot = two_track_snapshot();
        let inner = serde_json::json!({
            "suggestions": [
                { "track_id": "loud", "suggested_gain_db": -4.0, "reason": "loudest" },
                { "track_id": "quiet", "suggested_gain_db": 4.0, "reason": "quietest" }
            ]
        })
        .to_string();
        let transport = MockTransport::ok(anthropic_envelope(&inner));

        let result = auto_level(&transport, "sk-ant-test", &snapshot).unwrap();
        assert_eq!(result.model, LEVELING_MODEL);
        assert_eq!(result.suggestions.len(), 2);

        // The request carried our key + the API version header, to the right URL.
        let seen = transport.seen.borrow();
        let req = seen.as_ref().unwrap();
        assert_eq!(req.url, ANTHROPIC_MESSAGES_URL);
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-ant-test"));
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "anthropic-version" && v == ANTHROPIC_VERSION));
        // The body mentions the model and the target loudness.
        assert!(req.body.contains(LEVELING_MODEL));
        assert!(req.body.contains("-16"));
    }

    #[test]
    fn auto_level_surfaces_an_http_error_status() {
        let snapshot = two_track_snapshot();
        let transport = MockTransport {
            status: 401,
            body: r#"{"error":{"message":"invalid x-api-key"}}"#.to_string(),
            seen: RefCell::new(None),
        };

        let err = auto_level(&transport, "bad-key", &snapshot).unwrap_err();
        assert!(err.contains("401"), "error names the status: {err}");
    }

    #[test]
    fn auto_level_rejects_an_empty_snapshot() {
        let snapshot = LevelingSnapshot {
            tracks: vec![],
            target_lufs: -16.0,
        };
        let transport = MockTransport::ok(anthropic_envelope("{}"));
        assert!(auto_level(&transport, "key", &snapshot).is_err());
    }

    #[test]
    fn build_request_body_includes_every_track_and_the_target() {
        let snapshot = two_track_snapshot();
        let body = build_request_body(&snapshot).to_string();
        assert!(body.contains("Pulpit mic"));
        assert!(body.contains("Guest mic"));
        assert!(body.contains(LEVELING_MODEL));
    }
}
