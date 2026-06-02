use crate::compression::engines::base::{EngineResult, make_result, identity_result};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

static PATH_RE: OnceLock<Regex> = OnceLock::new();

fn get_path_re() -> &'static Regex {
    PATH_RE.get_or_init(|| {
        Regex::new(r"(?:[A-Za-z]:)?(?:/[\w.\-]+){2,}").unwrap()
    })
}

fn level_config(level: &str) -> (usize, usize) {
    match level {
        "lite" => (80, 5),
        "standard" => (60, 3),
        "aggressive" => (40, 2),
        _ => (60, 3),
    }
}

pub fn compress_path_collapse(text: &str, level: &str) -> Result<EngineResult, String> {
    if !["off", "lite", "standard", "aggressive"].contains(&level) {
        return Err(format!("level must be off, lite, standard, or aggressive, got {:?}", level));
    }
    if level == "off" || text.is_empty() {
        return Ok(identity_result(text));
    }

    let (min_len, min_occs) = level_config(level);
    let path_re = get_path_re();

    // Map: path string -> list of byte spans (start, end)
    let mut groups: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for m in path_re.find_iter(text) {
        let path = m.as_str().to_string();
        if path.chars().count() < min_len {
            continue;
        }
        groups.entry(path).or_default().push((m.start(), m.end()));
    }

    // Filter to candidates with at least min_occs occurrences
    let mut candidates: Vec<(String, Vec<(usize, usize)>)> = groups
        .into_iter()
        .filter(|(_, spans)| spans.len() >= min_occs)
        .collect();

    if candidates.is_empty() {
        return Ok(identity_result(text));
    }

    // Sort by first occurrence position for determinism
    candidates.sort_by_key(|(_, spans)| spans[0].0);

    // Filter to candidates that pay for themselves (savings > legend entry overhead)
    let mut selected: Vec<(String, String, Vec<(usize, usize)>)> = Vec::new();
    for (path, spans) in candidates {
        let alias = format!("<P{}>", selected.len() + 1);
        let rewrites = spans.len() - 1;
        
        let path_char_len = path.chars().count();
        let alias_char_len = alias.chars().count();
        let savings = rewrites * (path_char_len - alias_char_len);
        
        // Legend entry length: "<P1>=/path; "
        let legend_entry = alias_char_len + 1 + path_char_len + 2;
        if savings > legend_entry {
            selected.push((alias, path, spans));
        }
    }

    if selected.is_empty() {
        return Ok(identity_result(text));
    }

    // Build replacements list (start, end, alias_replacement)
    // We only replace subsequent (index >= 1) occurrences
    let mut ops = Vec::new();
    for (alias, _, spans) in &selected {
        for &(start, end) in &spans[1..] {
            ops.push((start, end, alias.clone()));
        }
    }

    // Sort replacements descending by start position to keep offsets valid
    ops.sort_by_key(|o| o.0);
    ops.reverse();

    let mut out = text.to_string();
    for (start, end, alias) in ops {
        out.replace_range(start..end, &alias);
    }

    // Build the legend prefix
    let mut legend_body = Vec::new();
    for (alias, path, _) in &selected {
        legend_body.push(format!("{}={}", alias, path));
    }
    let legend = format!("Path aliases: {}\n", legend_body.join("; "));
    let out_text = format!("{}{}", legend, out);

    let note = format!("{} path aliases", selected.len());
    Ok(make_result(text, &out_text, note))
}
