//! AI feature wrappers — HTTP-based, deliberately isolated from the audio
//! engine (no AI call ever touches the real-time thread).
//!
//! Phase 5/6 add: auto-leveling, noise/breath/click cleanup, speaker isolation,
//! auto-ducking suggestions (Anthropic), and jingle music generation (Suno, via
//! an Edge Function so the key never reaches the client). Each AI feature is
//! opt-in with explicit first-use consent, and gated to the Pro tier.
//!
//! Phase 5.1 ships the first move: [`leveling`] — Claude looks at a project
//! snapshot (per-track gains, loudness, clip counts) and suggests per-track gain
//! adjustments so a multi-mic recording sits balanced before mastering.
//!
//! Design notes that the whole module follows:
//! - The Anthropic call is plain HTTPS against the Messages API. The transport
//!   is a small [`HttpTransport`] trait so the request-build / response-parse
//!   logic is unit-tested with a mock — the default `cargo test` gate never
//!   touches the network or needs a key.
//! - The real transport is `reqwest`'s **blocking** client, run only via
//!   `tokio::task::spawn_blocking` from the command layer: AI is network I/O,
//!   not real-time, so it stays off both the async runtime and the audio thread.

pub mod jingle;
pub mod leveling;

/// The Anthropic API key, read from the environment. `None` (the common case in
/// the default gate and for Free-tier users) means the AI path is unavailable
/// and the command returns a clean validation error rather than calling out.
pub fn anthropic_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
}

/// The Anthropic Messages endpoint and the API version header we pin to.
pub const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The model we ask for leveling suggestions. A small, fast model is plenty for
/// reasoning over a handful of numbers; kept here so all AI features share one
/// knob.
pub const LEVELING_MODEL: &str = "claude-haiku-4-5";

/// A minimal HTTP seam so the AI logic is testable without the network.
///
/// One method: POST a JSON body to a URL with headers, get a status + body
/// string back. That's all the Messages API needs, and it lets a test inject a
/// canned Anthropic response (see [`leveling`] tests). The real implementation,
/// [`ReqwestTransport`], is a thin wrapper over `reqwest::blocking`.
pub trait HttpTransport {
    /// POST `body` to `url` with the given `(name, value)` headers. Returns the
    /// HTTP status code and the response body as text. A transport-level failure
    /// (DNS, TLS, connect) is the `Err` arm; HTTP error statuses come back as
    /// `Ok` so the caller can read the API's error body.
    fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<(u16, String), String>;
}

/// The production transport: `reqwest`'s blocking client. Constructed per call —
/// AI requests are rare and one-shot, so a long-lived pooled client buys nothing
/// and a fresh client keeps the type free of lifetimes/state.
pub struct ReqwestTransport;

impl HttpTransport for ReqwestTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<(u16, String), String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("building http client: {e}"))?;

        let mut req = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_owned());
        for (name, value) in headers {
            req = req.header(*name, *value);
        }

        let resp = req.send().map_err(|e| format!("sending request: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .map_err(|e| format!("reading response body: {e}"))?;
        Ok((status, text))
    }
}
