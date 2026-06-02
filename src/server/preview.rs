//! Dry-run compression preview.
//!
//! Pure function. Runs the production [`PayloadCompressor`] against a payload
//! and returns a structured summary (sizes, savings, audit events, token
//! estimates) without touching the network or any global state.
//!
//! The integration layer is expected to call this from a `/preview` endpoint or
//! similar inspection tool. The module is intentionally side-effect free so it
//! is safe to call on user-supplied payloads.

use crate::compression::{CompressRequestOptions, CompressionAudit, PayloadCompressor};
use crate::config::Settings;

/// Length of the JSON serialization. Used as the canonical "size" measure.
fn serialize_chars(payload: &serde_json::Value) -> usize {
    match serde_json::to_string(payload) {
        Ok(s) => s.chars().count(),
        Err(_) => format!("{:?}", payload).chars().count(),
    }
}

/// Cheap `len/4` estimate used because there is no `token_estimate` module
/// in the Rust port. Always used; therefore the returned summary always
/// carries `token_estimate_method: "fallback"`.
fn fallback_token_estimate(payload: &serde_json::Value) -> usize {
    serialize_chars(payload) / 4
}

/// Run compression against `payload` and return a structured summary.
///
/// The returned value is a plain JSON-serializable structure.
/// `compressed_payload` is the post-compression payload (a deep copy — the
/// input is never mutated).
///
/// `preview` never panics: if the underlying compressor returns an `Err`,
/// the result is computed against the original payload with zero savings
/// and an empty audit.
pub fn preview_compression(
    payload: &serde_json::Value,
    settings: &Settings,
    jl_dedupe: bool,
    caveman: Option<serde_json::Value>,
    rtk: Option<serde_json::Value>,
) -> serde_json::Value {
    let safe_payload = if payload.is_object() {
        payload.clone()
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let input_chars = serialize_chars(&safe_payload);

    let opts = CompressRequestOptions {
        jl_dedupe: Some(jl_dedupe),
        caveman,
        rtk,
        json_aware: None,
        lsh: None,
        max_text_chars: None,
        auto_insert_cache_wall: None,
    };

    let compressor = PayloadCompressor::new(settings.clone());

    let (compressed_payload, audit): (serde_json::Value, CompressionAudit) =
        match compressor.compress_request_payload(&safe_payload, "preview", Some(opts), false) {
            Ok(pair) => pair,
            Err(_) => (
                safe_payload.clone(),
                CompressionAudit {
                    endpoint: "preview".to_string(),
                    events: Vec::new(),
                    cache_hits: 0,
                    cache_misses: 0,
                    protected_blocks: 0,
                },
            ),
        };

    let output_chars = serialize_chars(&compressed_payload);
    let chars_saved = input_chars.saturating_sub(output_chars);
    let pct_saved = if input_chars > 0 {
        (chars_saved as f64) / (input_chars as f64) * 100.0
    } else {
        0.0
    };

    let input_token_estimate = fallback_token_estimate(&safe_payload);
    let output_token_estimate = fallback_token_estimate(&compressed_payload);

    let events = serde_json::to_value(&audit.events).unwrap_or(serde_json::Value::Array(Vec::new()));

    serde_json::json!({
        "input_chars": input_chars,
        "output_chars": output_chars,
        "chars_saved": chars_saved,
        "pct_saved": pct_saved,
        "events": events,
        "input_token_estimate": input_token_estimate,
        "output_token_estimate": output_token_estimate,
        "protected_blocks": audit.protected_blocks,
        "cache_hits": audit.cache_hits,
        "cache_misses": audit.cache_misses,
        "compressed_payload": compressed_payload,
        "token_estimate_method": "fallback",
    })
}
