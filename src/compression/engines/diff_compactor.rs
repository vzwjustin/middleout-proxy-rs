use crate::compression::engines::base::{EngineResult, make_result, identity_result};

fn level_config(level: &str) -> (usize, usize, bool) {
    match level {
        "lite" => (20, 3, false),
        "standard" => (10, 2, false),
        _ => (5, 1, true), // aggressive
    }
}

fn is_context(line: &str) -> bool {
    line.starts_with(' ')
}

fn collapse_context(lines: &[&str], threshold: usize, keep: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut collapsed = 0;
    let mut i = 0;
    let n = lines.len();

    while i < n {
        if !is_context(lines[i]) {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }

        let mut j = i;
        while j < n && is_context(lines[j]) {
            j += 1;
        }

        let run = j - i;
        if run >= threshold && run > 2 * keep {
            let omitted = run - 2 * keep;
            let marker = format!("[... {} unchanged lines ...]", omitted);
            
            for k in 0..keep {
                out.push(lines[i + k].to_string());
            }
            out.push(marker);
            for k in (j - keep)..j {
                out.push(lines[k].to_string());
            }
            collapsed += omitted;
        } else {
            for k in i..j {
                out.push(lines[k].to_string());
            }
        }
        i = j;
    }

    (out, collapsed)
}

fn drop_revert_pairs(lines: &[String]) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut removed = 0;
    let mut i = 0;
    let n = lines.len();

    while i < n {
        let cur = &lines[i];
        let nxt = if i + 1 < n { Some(&lines[i + 1]) } else { None };

        if let Some(nxt) = nxt {
            if cur.starts_with('-')
                && !cur.starts_with("---")
                && nxt.starts_with('+')
                && !nxt.starts_with("+++")
                && cur[1..] == nxt[1..]
            {
                removed += 1;
                i += 2;
                continue;
            }
        }
        out.push(cur.clone());
        i += 1;
    }

    (out, removed)
}

pub fn compress_diff_compactor(text: &str, level: &str) -> Result<EngineResult, String> {
    if !["off", "lite", "standard", "aggressive"].contains(&level) {
        return Err(format!("level must be off, lite, standard, or aggressive, got {:?}", level));
    }
    if level == "off" || text.is_empty() {
        return Ok(identity_result(text));
    }
    if !text.contains("@@") {
        return Ok(identity_result(text));
    }

    let (threshold, keep, do_reverts) = level_config(level);
    let lines: Vec<&str> = text.split('\n').collect();

    let (mut new_lines, collapsed_context) = collapse_context(&lines, threshold, keep);

    let mut reverts = 0;
    if do_reverts {
        let (temp_lines, r) = drop_revert_pairs(&new_lines);
        new_lines = temp_lines;
        reverts = r;
    }

    let out_text = new_lines.join("\n");

    // Never emit a longer payload than we received. Compare character counts.
    if out_text.chars().count() >= text.chars().count() {
        return Ok(identity_result(text));
    }

    let mut parts = Vec::new();
    if collapsed_context > 0 {
        parts.push(format!("collapsed {} context lines", collapsed_context));
    }
    if reverts > 0 {
        parts.push(format!("dropped {} revert pairs", reverts));
    }

    Ok(make_result(text, &out_text, parts.join("; ")))
}
