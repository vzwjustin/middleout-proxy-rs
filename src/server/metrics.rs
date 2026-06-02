//! Prometheus 0.0.4 text-format exporter.
//!
//! Renders a flat snapshot of audit + compression-cache statistics into the
//! Prometheus text exposition format.  The function is pure — no scraping, no
//! global registry, no HTTP.

use serde_json::Value;

use crate::config::Settings;

// ---------------------------------------------------------------------------
// Label-value escaping
// ---------------------------------------------------------------------------

fn escape_label_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

// ---------------------------------------------------------------------------
// Stats-field helpers
// ---------------------------------------------------------------------------

/// Pull an integer from a JSON Value, tolerating null / missing / wrong type.
fn get_int(stats: &Value, key: &str) -> i64 {
    match stats.get(key) {
        None | Some(Value::Null) => 0,
        Some(v) => {
            if let Some(n) = v.as_i64() {
                n
            } else if let Some(f) = v.as_f64() {
                f as i64
            } else {
                0
            }
        }
    }
}

/// Pull a float from a JSON Value, tolerating null / missing / wrong type.
fn get_float(stats: &Value, key: &str) -> f64 {
    match stats.get(key) {
        None | Some(Value::Null) => 0.0,
        Some(v) => v.as_f64().unwrap_or(0.0),
    }
}

/// Read a cache field that may be top-level (flat merge) or nested under
/// `result_cache` — mirrors the Python `_cache_field` helper.
fn cache_field(stats: &Value, key: &str) -> i64 {
    // Prefer top-level if the key is present and non-null.
    if let Some(v) = stats.get(key) {
        if !v.is_null() {
            return get_int(stats, key);
        }
    }
    // Fall back to the nested `result_cache` sub-object.
    if let Some(Value::Object(nested)) = stats.get("result_cache") {
        if let Some(v) = nested.get(key) {
            if !v.is_null() {
                if let Some(n) = v.as_i64() {
                    return n;
                } else if let Some(f) = v.as_f64() {
                    return f as i64;
                }
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render `stats` and `settings` to a Prometheus 0.0.4 text-format payload.
///
/// `stats` is expected to be a JSON object (the shape produced by
/// `stats_handler`: an `audit_logger.stats.snapshot()` merged with a
/// `"result_cache"` sub-object).  Missing / null keys default to zero.
///
/// Returns a newline-terminated string (the Python equivalent appends an
/// empty element and joins with `"\n"`, producing the same effect).
pub fn render_prometheus(stats: &Value, settings: &Settings) -> String {
    let mut lines: Vec<String> = Vec::new();

    // ------------------------------------------------------------------
    // Inner helpers (closures over `lines`)
    // ------------------------------------------------------------------

    macro_rules! emit_counter {
        ($name:expr, $help:expr, $value:expr) => {
            lines.push(format!("# HELP {} {}", $name, $help));
            lines.push(format!("# TYPE {} counter", $name));
            lines.push(format!("{} {}", $name, $value));
        };
    }

    // Emit HELP + TYPE then the metric line (no labels).
    macro_rules! emit_gauge_plain {
        ($name:expr, $help:expr, $value:expr) => {
            lines.push(format!("# HELP {} {}", $name, $help));
            lines.push(format!("# TYPE {} gauge", $name));
            lines.push(format!("{} {}", $name, $value));
        };
    }

    // Emit HELP + TYPE then a single labeled metric line.
    macro_rules! emit_gauge_labeled {
        ($name:expr, $help:expr, $label_key:expr, $label_val:expr, $value:expr) => {
            lines.push(format!("# HELP {} {}", $name, $help));
            lines.push(format!("# TYPE {} gauge", $name));
            lines.push(format!(
                "{}{{{}=\"{}\"}} {}",
                $name,
                $label_key,
                escape_label_value($label_val),
                $value
            ));
        };
    }

    // Emit a labeled metric line WITHOUT a preceding HELP/TYPE block.
    macro_rules! emit_gauge_labeled_no_meta {
        ($name:expr, $label_key:expr, $label_val:expr, $value:expr) => {
            lines.push(format!(
                "{}{{{}=\"{}\"}} {}",
                $name,
                $label_key,
                escape_label_value($label_val),
                $value
            ));
        };
    }

    // ------------------------------------------------------------------
    // Counters
    // ------------------------------------------------------------------

    emit_counter!(
        "middleout_requests_total",
        "Total HTTP requests handled by the proxy.",
        get_int(stats, "requests_total")
    );
    emit_counter!(
        "middleout_compressed_requests_total",
        "Requests that had at least one compression event.",
        get_int(stats, "compressed_requests")
    );
    emit_counter!(
        "middleout_upstream_errors_total",
        "Requests that failed to reach the upstream cleanly.",
        get_int(stats, "upstream_errors")
    );
    emit_counter!(
        "middleout_chars_saved_in_total",
        "Characters saved on inbound (request) payloads.",
        get_int(stats, "chars_saved_in")
    );
    emit_counter!(
        "middleout_chars_saved_out_total",
        "Characters saved on outbound (response) payloads.",
        get_int(stats, "chars_saved_out")
    );
    emit_counter!(
        "middleout_protected_blocks_total",
        "Blocks skipped to preserve the Anthropic prompt cache.",
        get_int(stats, "protected_blocks")
    );
    emit_counter!(
        "middleout_cache_hits_total",
        "Local LRU compression cache hits.",
        get_int(stats, "cache_hits")
    );
    emit_counter!(
        "middleout_cache_misses_total",
        "Local LRU compression cache misses.",
        get_int(stats, "cache_misses")
    );

    // ------------------------------------------------------------------
    // Gauges
    // ------------------------------------------------------------------

    emit_gauge_plain!(
        "middleout_uptime_seconds",
        "Seconds since the proxy process started.",
        format!("{:.6}", get_float(stats, "uptime_s"))
    );
    emit_gauge_plain!(
        "middleout_cache_size",
        "Current number of entries in the LRU compression cache.",
        cache_field(stats, "size")
    );
    emit_gauge_plain!(
        "middleout_cache_max_entries",
        "Configured maximum entries in the LRU compression cache.",
        cache_field(stats, "max_entries")
    );

    // `middleout_input_compression_enabled` — one labeled value, own HELP/TYPE
    emit_gauge_labeled!(
        "middleout_input_compression_enabled",
        "Whether the named compression engine is enabled (1 = on, 0 = off).",
        "engine",
        "input",
        if settings.input_compression_enabled { 1 } else { 0 }
    );

    // ------------------------------------------------------------------
    // Per-engine gauges — single HELP/TYPE block, multiple labeled lines
    // ------------------------------------------------------------------

    lines.push(
        "# HELP middleout_engine_enabled \
         Whether a named compression engine is enabled (1 = on, 0 = off)."
            .to_string(),
    );
    lines.push("# TYPE middleout_engine_enabled gauge".to_string());

    let engine_states: &[(&str, bool)] = &[
        ("caveman", settings.caveman_enabled),
        ("rtk", settings.rtk_enabled),
        ("jl_dedupe", settings.jl_dedupe_enabled),
        ("output", settings.output_compression_enabled),
    ];
    for (engine_name, enabled) in engine_states {
        emit_gauge_labeled_no_meta!(
            "middleout_engine_enabled",
            "engine",
            engine_name,
            if *enabled { 1 } else { 0 }
        );
    }

    emit_gauge_plain!(
        "middleout_jl_similarity_threshold",
        "Configured JL-style near-duplicate similarity threshold (0.0-1.0).",
        format!("{:.6}", settings.jl_similarity_threshold)
    );

    // Trailing newline: Python does `lines.append("")` then `"\n".join(lines)`,
    // which appends a final "\n" after the last real line.
    lines.push(String::new());

    lines.join("\n")
}
