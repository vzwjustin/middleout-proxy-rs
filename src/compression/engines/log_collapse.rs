use crate::compression::engines::base::{EngineResult, make_result, identity_result};
use regex::Regex;
use std::sync::OnceLock;

static TS_PATS: OnceLock<Vec<Regex>> = OnceLock::new();
static NUMBER_RE: OnceLock<Regex> = OnceLock::new();

fn get_ts_pats() -> &'static Vec<Regex> {
    TS_PATS.get_or_init(|| {
        vec![
            // [YYYY-MM-DD HH:MM:SS] or [YYYY-MM-DD HH:MM:SS.ms]
            Regex::new(r"^\s*\[\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?\]\s*").unwrap(),
            // ISO 8601: 2023-01-15T10:30:45(.ms)?(Z|+HH:MM)?
            Regex::new(
                r"^\s*\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?\s*"
            ).unwrap(),
            // bare HH:MM:SS or HH:MM:SS.ms at line start
            Regex::new(r"^\s*\d{2}:\d{2}:\d{2}(?:\.\d+)?\s*").unwrap(),
        ]
    })
}

fn get_number_re() -> &'static Regex {
    NUMBER_RE.get_or_init(|| Regex::new(r"\d+(?:\.\d+)?").unwrap())
}

fn strip_timestamps(line: &str) -> String {
    for pat in get_ts_pats() {
        let new_line = pat.replace(line, "");
        if new_line != line {
            return new_line.into_owned();
        }
    }
    line.to_string()
}

fn normalize(line: &str, level: &str) -> String {
    if level == "lite" {
        return line.to_string();
    }
    let s = strip_timestamps(line);
    if level == "aggressive" {
        get_number_re().replace_all(&s, "#").into_owned()
    } else {
        s
    }
}

fn threshold(level: &str) -> usize {
    if level == "lite" {
        10
    } else {
        5
    }
}

pub fn compress_log_collapse(text: &str, level: &str) -> Result<EngineResult, String> {
    if !["off", "lite", "standard", "aggressive"].contains(&level) {
        return Err(format!("level must be off, lite, standard, or aggressive, got {:?}", level));
    }
    if level == "off" || text.is_empty() {
        return Ok(identity_result(text));
    }

    let thresh = threshold(level);
    let lines: Vec<&str> = text.split('\n').collect();
    let keys: Vec<String> = lines.iter().map(|line| normalize(line, level)).collect();

    let mut out = Vec::new();
    let mut collapsed_lines = 0;
    let mut i = 0;
    let n = lines.len();

    while i < n {
        let mut j = i + 1;
        while j < n && keys[j] == keys[i] {
            j += 1;
        }
        let run = j - i;
        if run >= thresh {
            let omitted = run - 2;
            let marker = format!("[... {} identical lines collapsed ...]", omitted);
            
            // Measure string length using characters (matching Python's character-based check)
            let proposed = format!("{}\n{}\n{}", lines[i], marker, lines[j - 1]);
            let proposed_len = proposed.chars().count();
            
            let original_len: usize = lines[i..j].iter().map(|l| l.chars().count()).sum::<usize>() + (run - 1); // add newlines
            
            if proposed_len < original_len {
                out.push(lines[i].to_string());
                out.push(marker);
                out.push(lines[j - 1].to_string());
                collapsed_lines += omitted;
                i = j;
                continue;
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }

    let out_text = out.join("\n");
    let note = if collapsed_lines > 0 {
        format!("collapsed {} lines", collapsed_lines)
    } else {
        "".to_string()
    };

    Ok(make_result(text, &out_text, note))
}
