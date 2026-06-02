use serde_json::Value;

const WHITESPACE_SAFE_LANGS: &[&str] = &[
    "python", "py", "yaml", "yml", "makefile", "make", "haml", "coffee", "coffeescript",
    "fsharp", "f#", "sass"
];

fn parse_json_strict(text: &str) -> Option<Value> {
    let stripped = text.trim();
    if stripped.is_empty() {
        return None;
    }
    let first = stripped.chars().next()?;
    if first != '{' && first != '[' {
        return None;
    }
    serde_json::from_str(stripped).ok()
}

fn strip_jsonc(text: &str) -> Option<String> {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let n = chars.len();
    let mut in_string = false;
    let mut escape = false;

    while i < n {
        let ch = chars[i];
        if in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        // Outside string
        if ch == '"' {
            in_string = true;
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == '/' && i + 1 < n && chars[i + 1] == '/' {
            // Line comment runs to EOF or next newline
            let mut j = i + 2;
            while j < n && chars[j] != '\n' {
                j += 1;
            }
            i = j; // Keep the newline or stop at EOF
            continue;
        }

        if ch == '/' && i + 1 < n && chars[i + 1] == '*' {
            // Block comment runs to next */
            let mut j = i + 2;
            let mut found = false;
            while j + 1 < n {
                if chars[j] == '*' && chars[j + 1] == '/' {
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                return None; // Unterminated block comment — unsafe
            }
            i = j + 2;
            continue;
        }

        if ch == ',' {
            let mut j = i + 1;
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            if j < n && (chars[j] == '}' || chars[j] == ']') {
                // Drop trailing comma, keep following whitespace
                i += 1;
                continue;
            }
        }

        out.push(ch);
        i += 1;
    }
    Some(out)
}

fn try_minify_block(text: &str, level: &str) -> (String, bool) {
    if let Some(parsed) = parse_json_strict(text) {
        if let Ok(minified) = serde_json::to_string(&parsed) {
            return (minified, true);
        }
    }
    if level == "aggressive" {
        if let Some(stripped) = strip_jsonc(text) {
            if let Some(parsed) = parse_json_strict(&stripped) {
                if let Ok(minified) = serde_json::to_string(&parsed) {
                    return (minified, true);
                }
            }
        }
    }
    (text.to_string(), false)
}

fn collapse_prose_whitespace(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut blank_run = 0;
    for ln in lines {
        let trimmed = ln.trim_end();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run <= 2 {
                out.push(trimmed);
            }
        } else {
            blank_run = 0;
            out.push(trimmed);
        }
    }
    out.join("\n")
}

fn get_lang(header: &str) -> String {
    let mut lang = String::new();
    for c in header.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-' {
            lang.push(c);
        } else {
            break;
        }
    }
    lang.to_lowercase()
}

fn is_whitespace_safe_lang(lang: &str) -> bool {
    WHITESPACE_SAFE_LANGS.contains(&lang)
}

fn process_fenced_segment(segment: &str, level: &str) -> (String, usize) {
    let header_end = match segment.find('\n') {
        Some(pos) => pos,
        None => return (segment.to_string(), 0),
    };
    let header = &segment[..header_end];
    let body = &segment[header_end + 1..];
    let lang = get_lang(header);

    let mut blocks_minified = 0;
    let mut new_body = body.to_string();

    let is_json_like = matches!(lang.as_str(), "json" | "jsonc" | "json5")
        || parse_json_strict(body).is_some()
        || (level == "aggressive" && body.trim_start().starts_with(|c| c == '{' || c == '['));

    if is_json_like {
        let (minified, did) = try_minify_block(body, level);
        if did {
            let trailing_whitespace_len = body.len() - body.trim_end().len();
            let trailing = &body[body.len() - trailing_whitespace_len..];
            new_body = format!("{}{}", minified, trailing);
            blocks_minified += 1;
        }
    }

    if (level == "standard" || level == "aggressive") && !is_whitespace_safe_lang(&lang) {
        if blocks_minified == 0 {
            new_body = collapse_prose_whitespace(&new_body);
        }
    }

    (format!("{}\n{}", header, new_body), blocks_minified)
}

fn process_prose_segment(segment: &str, level: &str) -> (String, usize) {
    let mut blocks_minified = 0;
    let candidate = segment.trim();
    let mut new_segment = segment.to_string();
    if !candidate.is_empty() && candidate.starts_with(|c| c == '{' || c == '[') {
        let (minified, did) = try_minify_block(segment, level);
        if did {
            let leading = &segment[..segment.len() - segment.trim_start().len()];
            let trailing = &segment[segment.trim_end().len()..];
            new_segment = format!("{}{}{}", leading, minified, trailing);
            blocks_minified += 1;
        }
    }
    if (level == "standard" || level == "aggressive") && blocks_minified == 0 {
        new_segment = collapse_prose_whitespace(&new_segment);
    }
    (new_segment, blocks_minified)
}

pub fn compress_json_aware(text: &str, level: &str) -> Result<(String, usize), String> {
    if !["safe", "standard", "aggressive"].contains(&level) {
        return Err(format!("json_aware level must be safe, standard, or aggressive, got {:?}", level));
    }
    if text.is_empty() {
        return Ok((String::new(), 0));
    }

    let parts: Vec<&str> = text.split("```").collect();
    let mut rebuilt = Vec::with_capacity(parts.len());
    let mut blocks_found = 0;
    for (i, segment) in parts.iter().enumerate() {
        if i % 2 == 1 {
            let (new_seg, hits) = process_fenced_segment(segment, level);
            rebuilt.push(new_seg);
            blocks_found += hits;
        } else {
            let (new_seg, hits) = process_prose_segment(segment, level);
            rebuilt.push(new_seg);
            blocks_found += hits;
        }
    }
    Ok((rebuilt.join("```"), blocks_found))
}
