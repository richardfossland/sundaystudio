//! AI show notes, chapters & clip suggestions (Phase 5.2, Pro).
//!
//! After a take is recorded (or imported), its transcript is run through Claude
//! to produce everything a church needs to publish the episode: a few title
//! options, a Norwegian *and* an English summary, timestamped chapters, topic
//! tags, and a handful of suggested clip in/out points for social snippets.
//!
//! The transcript itself comes from the existing `sundaystudio://` deep-link
//! handoff (which already carries captions from SundayRec) or from a plain paste
//! in the edit panel. We never transcribe here — we only *reason over* an
//! existing transcript.
//!
//! Mirrors the proven [`super::leveling`] pattern so the whole flow is unit-
//! tested without the network or a key:
//! - [`build_request_body`] turns a [`ShowNotesInput`] into the Anthropic
//!   Messages JSON, asking for a strict JSON reply.
//! - [`parse_response`] reads Claude's reply back into [`ShowNotes`] and
//!   sanitizes it hard against a strict schema (clamps timestamps to the take's
//!   length, orders/validates chapters, bounds list sizes, drops zero/negative
//!   clips) so a hallucinated reply can never corrupt app state — the model only
//!   *suggests*; the engine decides what is safe to embed.
//! - [`generate_show_notes`] wires the two around an [`HttpTransport`], so a test
//!   drives the whole flow with a canned response and no key / no network.
//!
//! The accepted chapters are embedded as ffmpeg chapter metadata on export
//! (see [`crate::export`]); manual chapters keep working with no key at all.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{HttpTransport, ANTHROPIC_MESSAGES_URL, ANTHROPIC_VERSION, SHOWNOTES_MODEL};

/// Upper bounds on the lists we'll accept from the model, so a runaway reply
/// can't balloon app state. These are generous for a real episode and brutal
/// for a hallucination.
pub const MAX_TITLE_OPTIONS: usize = 5;
pub const MAX_CHAPTERS: usize = 40;
pub const MAX_TAGS: usize = 15;
pub const MAX_CLIPS: usize = 5;
/// We ask for 3–5 clips; this is the floor the prompt requests (not enforced —
/// fewer is fine, never an error).
pub const MIN_CLIPS_REQUESTED: usize = 3;

/// The longest transcript (in characters) we'll send. A 90-minute sermon is well
/// under this; the cap is a guard against a pathological paste, keeping the
/// request within model limits. Longer input is truncated (with a marker) rather
/// than rejected, so the feature still produces useful notes.
pub const MAX_TRANSCRIPT_CHARS: usize = 200_000;

/// Everything the show-notes call needs: the transcript and a little context.
///
/// `duration_ms` lets us clamp every returned timestamp into the real take, so
/// the model can never place a chapter or clip past the end of the audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ShowNotesInput.ts")]
pub struct ShowNotesInput {
    /// The full transcript text. Plain text or lightly-timestamped captions; the
    /// model reads it as prose and infers chapter boundaries.
    pub transcript: String,
    /// Total programme length in milliseconds, used to clamp timestamps. `0`
    /// disables clamping (we then only enforce ordering and non-negativity).
    pub duration_ms: f64,
    /// Free-text priming carried from the deep link (e.g. "Sermon, speaker:
    /// Ola"), or typed by the user. Helps the model name chapters sensibly.
    #[serde(default)]
    pub context: Option<String>,
    /// Glossary terms (speaker names, ministry jargon) so the model spells them
    /// right in titles and chapter names. De-duplicated upstream by the deep-link
    /// parser; passed through verbatim here.
    #[serde(default)]
    pub glossary: Vec<String>,
}

/// One timestamped chapter. `start_ms` is where the chapter begins on the
/// timeline; the title is what the podcast player shows. These map directly to
/// ffmpeg chapter metadata on export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ShowNotesChapter.ts")]
pub struct ShowNotesChapter {
    /// Chapter start, in milliseconds from the top of the programme.
    pub start_ms: f64,
    /// Short, human chapter title (e.g. "Welcome & notices").
    pub title: String,
}

/// One suggested social/highlight clip: an in/out span plus why it's worth
/// clipping. The UI shows these as cards; the user picks which to export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ShowNotesClip.ts")]
pub struct ShowNotesClip {
    /// Clip start, in milliseconds.
    pub start_ms: f64,
    /// Clip end, in milliseconds (always strictly after `start_ms`).
    pub end_ms: f64,
    /// A short reason the clip is compelling ("Strong, quotable summary line").
    pub reason: String,
}

/// The full show-notes result returned to the renderer.
///
/// Everything here has already been sanitized by [`parse_response`]: titles are
/// trimmed and bounded, chapters are ordered and clamped into the take, clips are
/// valid spans, and the model that produced it is echoed for the "Suggested by
/// Claude" label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ShowNotes.ts")]
pub struct ShowNotes {
    /// A few episode title options for the user to pick from.
    pub title_options: Vec<String>,
    /// Norwegian episode summary (Norwegian-first, per Sunday house style).
    pub summary_no: String,
    /// English episode summary.
    pub summary_en: String,
    /// Timestamped chapters, ordered by `start_ms`.
    pub chapters: Vec<ShowNotesChapter>,
    /// Topic tags for the podcast platforms.
    pub tags: Vec<String>,
    /// Suggested highlight/social clips (3–5 when the audio supports it).
    pub clips: Vec<ShowNotesClip>,
    /// The model that produced this (for the UI label).
    pub model: String,
}

/// Truncate the transcript to [`MAX_TRANSCRIPT_CHARS`] on a char boundary,
/// appending a marker so the model knows it's seeing a head slice. Returns the
/// (possibly truncated) string. Pure helper, kept separate so it's testable.
fn clip_transcript(transcript: &str) -> String {
    if transcript.chars().count() <= MAX_TRANSCRIPT_CHARS {
        return transcript.to_string();
    }
    let head: String = transcript.chars().take(MAX_TRANSCRIPT_CHARS).collect();
    format!("{head}\n\n[transcript truncated for length]")
}

/// Build the Anthropic Messages request body for a show-notes input.
///
/// We hand Claude the transcript plus any context/glossary and ask for a strict
/// JSON reply matching [`ShowNotes`]'s shape. The schema is spelled out in the
/// prompt so [`parse_response`] is reliable. Norwegian-first, church-appropriate
/// wording is requested explicitly.
pub fn build_request_body(input: &ShowNotesInput) -> serde_json::Value {
    let transcript = clip_transcript(&input.transcript);

    let context_line = match input.context.as_deref().map(str::trim) {
        Some(c) if !c.is_empty() => format!("Context for this recording: {c}\n"),
        _ => String::new(),
    };
    let glossary_line = if input.glossary.is_empty() {
        String::new()
    } else {
        format!(
            "Spell these names/terms exactly as given: {}.\n",
            input.glossary.join(", ")
        )
    };
    // Tell the model the real length so it never proposes a timestamp past the
    // end of the audio. We still clamp on parse regardless.
    let duration_line = if input.duration_ms > 0.0 {
        format!(
            "The recording is {:.0} ms long ({}). Every timestamp MUST fall within it.\n",
            input.duration_ms,
            human_ms(input.duration_ms),
        )
    } else {
        String::new()
    };

    let prompt = format!(
        "You are a producer preparing a church/community podcast episode for \
         publication. Below is the transcript of the recording. Produce \
         publication-ready show notes.\n\n\
         {context}{glossary}{duration}\n\
         Transcript:\n\"\"\"\n{transcript}\n\"\"\"\n\n\
         Produce, in warm but concise language appropriate for a church audience \
         (no hype, no clickbait):\n\
         - {max_titles} or fewer episode title options.\n\
         - A Norwegian summary (2-4 sentences) and an English summary (2-4 \
           sentences). Norwegian first.\n\
         - Timestamped chapters covering the whole episode, each with a short \
           title. Use the transcript's flow to place them; the first chapter \
           starts at 0 ms.\n\
         - Up to {max_tags} short topic tags.\n\
         - {min_clips} to {max_clips} suggested highlight clips (each an in/out \
           point in ms with a one-line reason it's worth sharing).\n\n\
         Reply with ONLY a JSON object of exactly this form (times are integer \
         milliseconds):\n\
         {{\"title_options\": [\"...\"], \"summary_no\": \"...\", \
         \"summary_en\": \"...\", \"chapters\": [{{\"start_ms\": 0, \
         \"title\": \"...\"}}], \"tags\": [\"...\"], \"clips\": \
         [{{\"start_ms\": 0, \"end_ms\": 30000, \"reason\": \"...\"}}]}}\n\
         No prose, no markdown fences.",
        context = context_line,
        glossary = glossary_line,
        duration = duration_line,
        transcript = transcript,
        max_titles = MAX_TITLE_OPTIONS,
        max_tags = MAX_TAGS,
        min_clips = MIN_CLIPS_REQUESTED,
        max_clips = MAX_CLIPS,
    );

    serde_json::json!({
        "model": SHOWNOTES_MODEL,
        "max_tokens": 4096,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    })
}

/// The shape of Claude's reply inside the Messages-API envelope. We only read the
/// first `text` content block; the rest of the envelope is ignored.
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

/// The raw JSON object we ask Claude to emit, before sanitization.
#[derive(Deserialize)]
struct RawShowNotes {
    #[serde(default)]
    title_options: Vec<String>,
    #[serde(default)]
    summary_no: String,
    #[serde(default)]
    summary_en: String,
    #[serde(default)]
    chapters: Vec<RawChapter>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    clips: Vec<RawClip>,
}

#[derive(Deserialize)]
struct RawChapter {
    #[serde(default)]
    start_ms: f64,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct RawClip {
    #[serde(default)]
    start_ms: f64,
    #[serde(default)]
    end_ms: f64,
    #[serde(default)]
    reason: String,
}

/// Parse an Anthropic Messages-API response body into a sanitized [`ShowNotes`].
///
/// Robust by construction — the model only suggests; this function decides what
/// is safe to keep:
/// - reads the first `text` content block and slices to the outermost `{...}`
///   (tolerating prose / markdown fences);
/// - trims and drops empty strings, then bounds every list to its `MAX_*`;
/// - clamps every timestamp to `[0, duration_ms]` (when `duration_ms > 0`),
///   sorts chapters by start, drops chapters with empty titles and any whose
///   start duplicates an earlier one;
/// - keeps only clips that are a strictly-positive span after clamping.
pub fn parse_response(body: &str, input: &ShowNotesInput) -> Result<ShowNotes, String> {
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

    let raw: RawShowNotes =
        serde_json::from_str(json_slice).map_err(|e| format!("parsing show-notes JSON: {e}"))?;

    let max_ms = if input.duration_ms > 0.0 {
        Some(input.duration_ms)
    } else {
        None
    };

    Ok(ShowNotes {
        title_options: clean_strings(raw.title_options, MAX_TITLE_OPTIONS),
        summary_no: raw.summary_no.trim().to_string(),
        summary_en: raw.summary_en.trim().to_string(),
        chapters: sanitize_chapters(raw.chapters, max_ms),
        tags: clean_strings(raw.tags, MAX_TAGS),
        clips: sanitize_clips(raw.clips, max_ms),
        model: SHOWNOTES_MODEL.to_string(),
    })
}

/// Trim, drop blanks, and bound a list of strings.
fn clean_strings(items: Vec<String>, max: usize) -> Vec<String> {
    items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(max)
        .collect()
}

/// Clamp a timestamp to `[0, max]` (or just `>= 0` when no max is known).
fn clamp_ms(ms: f64, max: Option<f64>) -> f64 {
    let lo = ms.max(0.0);
    match max {
        Some(m) => lo.min(m),
        None => lo,
    }
}

/// Sanitize chapters: clamp starts into the take, drop empty titles, sort by
/// start, drop duplicate starts (keep the first), and bound the count. A clean
/// ordered list is what the export step turns into ffmpeg chapter metadata.
fn sanitize_chapters(raw: Vec<RawChapter>, max_ms: Option<f64>) -> Vec<ShowNotesChapter> {
    let mut chapters: Vec<ShowNotesChapter> = raw
        .into_iter()
        .filter_map(|c| {
            let title = c.title.trim().to_string();
            if title.is_empty() {
                return None;
            }
            Some(ShowNotesChapter {
                start_ms: clamp_ms(c.start_ms, max_ms),
                title,
            })
        })
        .collect();

    // Order by start time so chapters are monotonic for the player / ffmpeg.
    chapters.sort_by(|a, b| {
        a.start_ms
            .partial_cmp(&b.start_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Drop chapters whose start duplicates the previous one (a player needs
    // strictly-increasing chapter starts; ffmpeg rejects zero-length chapters).
    let mut deduped: Vec<ShowNotesChapter> = Vec::with_capacity(chapters.len());
    for ch in chapters {
        if deduped.last().map(|p| p.start_ms) == Some(ch.start_ms) {
            continue;
        }
        deduped.push(ch);
    }
    deduped.truncate(MAX_CHAPTERS);
    deduped
}

/// Sanitize clips: clamp both ends into the take, keep only strictly-positive
/// spans, trim reasons, and bound the count.
fn sanitize_clips(raw: Vec<RawClip>, max_ms: Option<f64>) -> Vec<ShowNotesClip> {
    raw.into_iter()
        .filter_map(|c| {
            let start = clamp_ms(c.start_ms, max_ms);
            let end = clamp_ms(c.end_ms, max_ms);
            if end <= start {
                return None; // not a real span after clamping
            }
            Some(ShowNotesClip {
                start_ms: start,
                end_ms: end,
                reason: c.reason.trim().to_string(),
            })
        })
        .take(MAX_CLIPS)
        .collect()
}

/// Slice out the first top-level `{...}` object from a string, tolerating
/// leading/trailing prose or markdown fences. Same approach as the leveling
/// parser.
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end > start {
        Some(&text[start..=end])
    } else {
        None
    }
}

/// Format a millisecond duration as `H:MM:SS` / `M:SS` for the prompt, so the
/// model has a human anchor alongside the raw ms.
fn human_ms(ms: f64) -> String {
    let total_secs = (ms / 1000.0).round().max(0.0) as u64;
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The complete show-notes call: build the request, send it through `transport`,
/// and parse the reply into a sanitized [`ShowNotes`].
///
/// `transport` is the seam: production passes a [`super::ReqwestTransport`],
/// tests pass a mock. `api_key` is the caller's Anthropic key. A non-2xx HTTP
/// status surfaces the API's error body so the UI can show "your key is invalid"
/// etc. An empty transcript is rejected before any network call.
pub fn generate_show_notes(
    transport: &dyn HttpTransport,
    api_key: &str,
    input: &ShowNotesInput,
) -> Result<ShowNotes, String> {
    if input.transcript.trim().is_empty() {
        return Err(
            "no transcript to summarise — import captions or paste a transcript".to_string(),
        );
    }

    let body = build_request_body(input).to_string();
    let headers = [
        ("x-api-key", api_key),
        ("anthropic-version", ANTHROPIC_VERSION),
    ];

    let (status, resp_body) = transport.post_json(ANTHROPIC_MESSAGES_URL, &headers, &body)?;
    if !(200..300).contains(&status) {
        return Err(format!("Anthropic API returned HTTP {status}: {resp_body}"));
    }

    parse_response(&resp_body, input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn input() -> ShowNotesInput {
        ShowNotesInput {
            transcript: "Welcome everyone to today's episode. We start with notices, then \
                         move into the main interview about youth ministry, and close with a \
                         prayer."
                .to_string(),
            duration_ms: 600_000.0, // 10 minutes
            context: Some("Sermon recap, speaker: Ola".to_string()),
            glossary: vec!["Ola Nordmann".to_string()],
        }
    }

    /// A canned Anthropic Messages reply with `text` set to `inner`.
    fn anthropic_envelope(inner: &str) -> String {
        serde_json::json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": SHOWNOTES_MODEL,
            "content": [ { "type": "text", "text": inner } ],
            "stop_reason": "end_turn"
        })
        .to_string()
    }

    fn full_reply() -> String {
        serde_json::json!({
            "title_options": ["Youth Ministry Today", "  ", "A Conversation on Faith"],
            "summary_no": "  En samtale om ungdomsarbeid.  ",
            "summary_en": "A conversation about youth ministry.",
            "chapters": [
                { "start_ms": 0, "title": "Welcome & notices" },
                { "start_ms": 120000, "title": "Interview" },
                { "start_ms": 540000, "title": "Closing prayer" }
            ],
            "tags": ["youth", "ministry", "faith"],
            "clips": [
                { "start_ms": 130000, "end_ms": 160000, "reason": "Strong opening of the interview" },
                { "start_ms": 540000, "end_ms": 570000, "reason": "Heartfelt closing prayer" }
            ]
        })
        .to_string()
    }

    struct SeenRequest {
        url: String,
        headers: Vec<(String, String)>,
        body: String,
    }

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
    fn parses_a_full_reply_into_clean_show_notes() {
        let notes = parse_response(&anthropic_envelope(&full_reply()), &input()).unwrap();

        // Titles: blanks dropped, trimmed.
        assert_eq!(
            notes.title_options,
            vec!["Youth Ministry Today", "A Conversation on Faith"]
        );
        // Summaries trimmed.
        assert_eq!(notes.summary_no, "En samtale om ungdomsarbeid.");
        assert_eq!(notes.summary_en, "A conversation about youth ministry.");
        // Chapters ordered, all within the 10-minute take.
        assert_eq!(notes.chapters.len(), 3);
        assert_eq!(notes.chapters[0].start_ms, 0.0);
        assert!(notes
            .chapters
            .windows(2)
            .all(|w| w[0].start_ms < w[1].start_ms));
        assert_eq!(notes.tags, vec!["youth", "ministry", "faith"]);
        assert_eq!(notes.clips.len(), 2);
        assert!(notes.clips.iter().all(|c| c.end_ms > c.start_ms));
        assert_eq!(notes.model, SHOWNOTES_MODEL);
    }

    #[test]
    fn clamps_timestamps_past_the_end_of_the_take() {
        let mut inp = input();
        inp.duration_ms = 60_000.0; // a 1-minute take
        let inner = serde_json::json!({
            "title_options": ["X"],
            "summary_no": "no", "summary_en": "en",
            "chapters": [
                { "start_ms": 0, "title": "Start" },
                { "start_ms": 999999, "title": "Way past the end" }
            ],
            "tags": [],
            "clips": [
                { "start_ms": 30000, "end_ms": 999999, "reason": "runs past the end" }
            ]
        })
        .to_string();

        let notes = parse_response(&anthropic_envelope(&inner), &inp).unwrap();
        // The runaway chapter is clamped to the take length.
        assert!(notes.chapters.iter().all(|c| c.start_ms <= 60_000.0));
        assert!(notes.chapters.iter().any(|c| c.start_ms == 60_000.0));
        // The clip's end is clamped, but it's still a positive span (kept).
        assert_eq!(notes.clips.len(), 1);
        assert_eq!(notes.clips[0].end_ms, 60_000.0);
        assert!(notes.clips[0].end_ms > notes.clips[0].start_ms);
    }

    #[test]
    fn drops_empty_titled_chapters_and_zero_length_clips() {
        let inner = serde_json::json!({
            "title_options": ["X"],
            "summary_no": "no", "summary_en": "en",
            "chapters": [
                { "start_ms": 0, "title": "Keep" },
                { "start_ms": 1000, "title": "   " }
            ],
            "tags": [],
            "clips": [
                { "start_ms": 5000, "end_ms": 5000, "reason": "zero length" },
                { "start_ms": 9000, "end_ms": 8000, "reason": "inverted" },
                { "start_ms": 10000, "end_ms": 20000, "reason": "good" }
            ]
        })
        .to_string();

        let notes = parse_response(&anthropic_envelope(&inner), &input()).unwrap();
        assert_eq!(notes.chapters.len(), 1);
        assert_eq!(notes.chapters[0].title, "Keep");
        // Only the genuine positive span survives.
        assert_eq!(notes.clips.len(), 1);
        assert_eq!(notes.clips[0].reason, "good");
    }

    #[test]
    fn sorts_chapters_and_dedupes_identical_starts() {
        let inner = serde_json::json!({
            "title_options": [], "summary_no": "", "summary_en": "",
            "chapters": [
                { "start_ms": 30000, "title": "Third" },
                { "start_ms": 0, "title": "First" },
                { "start_ms": 0, "title": "Duplicate start" },
                { "start_ms": 10000, "title": "Second" }
            ],
            "tags": [], "clips": []
        })
        .to_string();

        let notes = parse_response(&anthropic_envelope(&inner), &input()).unwrap();
        let starts: Vec<f64> = notes.chapters.iter().map(|c| c.start_ms).collect();
        assert_eq!(starts, vec![0.0, 10000.0, 30000.0]);
        // The first chapter at 0 ms wins; the duplicate is dropped.
        assert_eq!(notes.chapters[0].title, "First");
    }

    #[test]
    fn bounds_runaway_lists() {
        let titles: Vec<String> = (0..50).map(|i| format!("Title {i}")).collect();
        let tags: Vec<String> = (0..50).map(|i| format!("tag{i}")).collect();
        let clips: Vec<serde_json::Value> = (0..50)
            .map(|i| {
                let s = (i as f64) * 1000.0;
                serde_json::json!({ "start_ms": s, "end_ms": s + 500.0, "reason": "x" })
            })
            .collect();
        let inner = serde_json::json!({
            "title_options": titles, "summary_no": "n", "summary_en": "e",
            "chapters": [], "tags": tags, "clips": clips
        })
        .to_string();

        let mut inp = input();
        inp.duration_ms = 0.0; // no clamping → exercise pure list bounding
        let notes = parse_response(&anthropic_envelope(&inner), &inp).unwrap();
        assert_eq!(notes.title_options.len(), MAX_TITLE_OPTIONS);
        assert_eq!(notes.tags.len(), MAX_TAGS);
        assert_eq!(notes.clips.len(), MAX_CLIPS);
    }

    #[test]
    fn tolerates_prose_and_fences_around_the_json() {
        let inner = "Here are your show notes!\n```json\n{\"title_options\": [\"Ep 1\"], \
            \"summary_no\": \"no\", \"summary_en\": \"en\", \"chapters\": \
            [{\"start_ms\": 0, \"title\": \"Intro\"}], \"tags\": [\"a\"], \"clips\": []}\n```";
        let notes = parse_response(&anthropic_envelope(inner), &input()).unwrap();
        assert_eq!(notes.title_options, vec!["Ep 1"]);
        assert_eq!(notes.chapters.len(), 1);
    }

    #[test]
    fn generate_drives_the_whole_flow_with_a_mock() {
        let transport = MockTransport::ok(anthropic_envelope(&full_reply()));
        let notes = generate_show_notes(&transport, "sk-ant-test", &input()).unwrap();
        assert_eq!(notes.model, SHOWNOTES_MODEL);
        assert!(!notes.chapters.is_empty());

        // The request carried our key + version header to the right URL, and the
        // prompt mentions the model, the context and the glossary term.
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
        assert!(req.body.contains(SHOWNOTES_MODEL));
        assert!(req.body.contains("Ola Nordmann"));
        assert!(req.body.contains("Sermon recap"));
    }

    #[test]
    fn generate_rejects_an_empty_transcript_before_calling_out() {
        let mut inp = input();
        inp.transcript = "   ".to_string();
        let transport = MockTransport::ok(anthropic_envelope(&full_reply()));
        let err = generate_show_notes(&transport, "key", &inp).unwrap_err();
        assert!(err.contains("no transcript"));
        // No request must have been sent — the empty-transcript guard fires first.
        assert!(transport.seen.borrow().is_none());
    }

    #[test]
    fn generate_surfaces_an_http_error_status() {
        let transport = MockTransport {
            status: 401,
            body: r#"{"error":{"message":"invalid x-api-key"}}"#.to_string(),
            seen: RefCell::new(None),
        };
        let err = generate_show_notes(&transport, "bad-key", &input()).unwrap_err();
        assert!(err.contains("401"), "error names the status: {err}");
    }

    #[test]
    fn long_transcript_is_truncated_not_rejected() {
        let big = "word ".repeat(MAX_TRANSCRIPT_CHARS); // way over the cap
        let clipped = clip_transcript(&big);
        assert!(clipped.chars().count() <= MAX_TRANSCRIPT_CHARS + 64);
        assert!(clipped.contains("[transcript truncated for length]"));
        // A short transcript passes through untouched.
        assert_eq!(clip_transcript("short"), "short");
    }

    #[test]
    fn human_ms_formats_hours_minutes_seconds() {
        assert_eq!(human_ms(0.0), "0:00");
        assert_eq!(human_ms(65_000.0), "1:05");
        assert_eq!(human_ms(3_661_000.0), "1:01:01");
    }

    #[test]
    fn build_request_body_includes_the_transcript_and_schema() {
        let body = build_request_body(&input()).to_string();
        assert!(body.contains("youth ministry"));
        assert!(body.contains("summary_no"));
        assert!(body.contains("summary_en"));
        assert!(body.contains(SHOWNOTES_MODEL));
        // The duration anchor is present for a timed input.
        assert!(body.contains("10:00"));
    }
}
