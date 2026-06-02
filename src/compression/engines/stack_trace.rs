use crate::compression::engines::base::{EngineResult, make_result, identity_result};
use regex::Regex;
use std::sync::OnceLock;

static PY_FRAME_RE: OnceLock<Regex> = OnceLock::new();
static JAVA_FRAME_RE: OnceLock<Regex> = OnceLock::new();
static JS_FRAME_RE: OnceLock<Regex> = OnceLock::new();
static JS_BARE_RE: OnceLock<Regex> = OnceLock::new();
static RUST_FRAME_RE: OnceLock<Regex> = OnceLock::new();
static MARKER_RE: OnceLock<Regex> = OnceLock::new();

fn get_py_frame_re() -> &'static Regex { PY_FRAME_RE.get_or_init(|| Regex::new(r#"^\s*File "(?P<file>[^"]+)", line (?P<line>\d+), in (?P<func>\S+)\s*$"#).unwrap()) }
fn get_java_frame_re() -> &'static Regex { JAVA_FRAME_RE.get_or_init(|| Regex::new(r"^\s*at\s+(?P<func>[\w$.<>]+)\((?P<file>[\w./$]+):(?P<line>\d+)\)\s*$").unwrap()) }
fn get_js_frame_re() -> &'static Regex { JS_FRAME_RE.get_or_init(|| Regex::new(r"^\s*at\s+(?P<func>[\w$.<>]+)\s+\((?P<file>[^():\s]+):(?P<line>\d+)(?::\d+)?\)\s*$").unwrap()) }
fn get_js_bare_re() -> &'static Regex { JS_BARE_RE.get_or_init(|| Regex::new(r"^\s*at\s+(?P<file>[^():\s]+):(?P<line>\d+)(?::\d+)?\s*$").unwrap()) }
fn get_rust_frame_re() -> &'static Regex { RUST_FRAME_RE.get_or_init(|| Regex::new(r"^\s*\d+:\s+(?P<func>[\w<>$]+(?:::[\w<>$]+)+)\s*$").unwrap()) }
fn get_marker_re() -> &'static Regex { MARKER_RE.get_or_init(|| Regex::new(r"^\s*\[\.\.\..*frames?.*\.\.\.\]\s*$").unwrap()) }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FrameIdentity {
    file: String,
    func: String,
    line: String,
    context: Option<String>,
}

fn parse_frame(line: &str) -> Option<(String, String, String)> {
    if let Some(caps) = get_py_frame_re().captures(line) {
        return Some((caps["file"].to_string(), caps["func"].to_string(), caps["line"].to_string()));
    }
    if let Some(caps) = get_java_frame_re().captures(line) {
        return Some((caps["file"].to_string(), caps["func"].to_string(), caps["line"].to_string()));
    }
    if let Some(caps) = get_js_frame_re().captures(line) {
        return Some((caps["file"].to_string(), caps["func"].to_string(), caps["line"].to_string()));
    }
    if let Some(caps) = get_js_bare_re().captures(line) {
        return Some((caps["file"].to_string(), "<anon>".to_string(), caps["line"].to_string()));
    }
    if let Some(caps) = get_rust_frame_re().captures(line) {
        return Some(("<rust>".to_string(), caps["func"].to_string(), "0".to_string()));
    }
    None
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn is_context_line(line: &str, frame_line: &str) -> bool {
    if line.trim().is_empty() {
        return false;
    }
    if parse_frame(line).is_some() {
        return false;
    }
    indent(line) > indent(frame_line)
}

#[derive(Debug, Clone)]
enum UnitKind {
    Marker,
    Other,
    Frame(FrameIdentity),
}

#[derive(Debug, Clone)]
struct TraceUnit {
    kind: UnitKind,
    start: usize,
    end: usize,
}

fn scan_units(lines: &[String]) -> Vec<TraceUnit> {
    let mut units = Vec::new();
    let mut i = 0;
    let n = lines.len();
    let marker_re = get_marker_re();

    while i < n {
        let line = &lines[i];
        if marker_re.is_match(line) {
            units.push(TraceUnit { kind: UnitKind::Marker, start: i, end: i + 1 });
            i += 1;
            continue;
        }
        if let Some((file, func, line_no)) = parse_frame(line) {
            let mut consumed = 1;
            let mut context = None;
            if i + 1 < n && is_context_line(&lines[i + 1], line) {
                context = Some(lines[i + 1].clone());
                consumed = 2;
            }
            let id = FrameIdentity { file, func, line: line_no, context };
            units.push(TraceUnit { kind: UnitKind::Frame(id), start: i, end: i + consumed });
            i += consumed;
        } else {
            units.push(TraceUnit { kind: UnitKind::Other, start: i, end: i + 1 });
            i += 1;
        }
    }
    units
}

fn level_config(level: &str) -> (usize, Option<usize>, bool) {
    match level {
        "lite" => (5, None, false),
        "standard" => (3, Some(3), false),
        "aggressive" => (2, Some(2), true),
        _ => (3, Some(3), false),
    }
}

fn collapse_runs(
    lines: &[String],
    ident_thresh: usize,
    similar_thresh: Option<usize>,
) -> (Vec<String>, usize) {
    let units = scan_units(lines);
    let mut out = Vec::new();
    let mut collapsed = 0;
    let mut i = 0;
    
    while i < units.len() {
        let unit = &units[i];
        match &unit.kind {
            UnitKind::Marker | UnitKind::Other => {
                for line in &lines[unit.start..unit.end] {
                    out.push(line.clone());
                }
                i += 1;
            }
            UnitKind::Frame(id) => {
                // Run of identical frame+context units
                let mut j = i + 1;
                while j < units.len() {
                    if let UnitKind::Frame(ref next_id) = units[j].kind {
                        if next_id == id {
                            j += 1;
                            continue;
                        }
                    }
                    break;
                }
                let run = j - i;
                if run >= ident_thresh {
                    out.push(format!("[... {} identical frames collapsed ...]", run));
                    collapsed += run - 1;
                    i = j;
                    continue;
                }

                // Run of similar frames (same file + func, different line)
                if let Some(similar_th) = similar_thresh {
                    let file = &id.file;
                    let func = &id.func;
                    let mut j = i + 1;
                    while j < units.len() {
                        if let UnitKind::Frame(ref next_id) = units[j].kind {
                            if &next_id.file == file && &next_id.func == func {
                                j += 1;
                                continue;
                            }
                        }
                        break;
                    }
                    let run = j - i;
                    if run >= similar_th {
                        out.push(format!("[... {} similar frames in {}() at {} collapsed ...]", run, func, file));
                        collapsed += run - 1;
                        i = j;
                        continue;
                    }
                }

                // Verbatim
                for line in &lines[unit.start..unit.end] {
                    out.push(line.clone());
                }
                i += 1;
            }
        }
    }
    (out, collapsed)
}

fn truncate_trace_blocks(lines: &[String]) -> (Vec<String>, usize) {
    let units = scan_units(lines);
    let mut out = Vec::new();
    let mut truncated = 0;
    let mut i = 0;
    let n = units.len();

    while i < n {
        let unit = &units[i];
        if let UnitKind::Other = unit.kind {
            for line in &lines[unit.start..unit.end] {
                out.push(line.clone());
            }
            i += 1;
            continue;
        }

        let mut j = i;
        while j < n {
            match units[j].kind {
                UnitKind::Frame(_) | UnitKind::Marker => j += 1,
                UnitKind::Other => break,
            }
        }

        let block = &units[i..j];
        if block.len() > 5 {
            let omitted = block.len() - 3;
            // Head 2
            for u in &block[..2] {
                for line in &lines[u.start..u.end] {
                    out.push(line.clone());
                }
            }
            // Marker
            out.push(format!("[... {} frames omitted; truncated trace ...]", omitted));
            // Tail 1
            if let Some(u) = block.last() {
                for line in &lines[u.start..u.end] {
                    out.push(line.clone());
                }
            }
            truncated += omitted;
        } else {
            for u in block {
                for line in &lines[u.start..u.end] {
                    out.push(line.clone());
                }
            }
        }
        i = j;
    }
    (out, truncated)
}

pub fn compress_stack_trace(text: &str, level: &str) -> Result<EngineResult, String> {
    if !["off", "lite", "standard", "aggressive"].contains(&level) {
        return Err(format!("level must be off, lite, standard, or aggressive, got {:?}", level));
    }
    if level == "off" || text.is_empty() {
        return Ok(identity_result(text));
    }

    let (ident_thresh, similar_thresh, truncate) = level_config(level);

    let lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    let (mut new_lines, collapsed) = collapse_runs(&lines, ident_thresh, similar_thresh);
    
    let mut truncated = 0;
    if truncate {
        let (t_lines, t_count) = truncate_trace_blocks(&new_lines);
        new_lines = t_lines;
        truncated = t_count;
    }

    let out_text = new_lines.join("\n");
    if out_text.chars().count() >= text.chars().count() {
        return Ok(identity_result(text));
    }

    let mut parts = Vec::new();
    if collapsed > 0 {
        parts.push(format!("collapsed {} frames", collapsed));
    }
    if truncated > 0 {
        parts.push(format!("truncated {} frames", truncated));
    }
    let note = parts.join("; ");

    Ok(make_result(text, &out_text, note))
}
