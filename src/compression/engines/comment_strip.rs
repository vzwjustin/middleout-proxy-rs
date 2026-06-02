use crate::compression::engines::base::{EngineResult, make_result, identity_result};
use regex::Regex;
use std::sync::OnceLock;

static FENCE_RE: OnceLock<Regex> = OnceLock::new();
static PY_DEF_RE: OnceLock<Regex> = OnceLock::new();

fn get_fence_re() -> &'static Regex {
    FENCE_RE.get_or_init(|| Regex::new(r"^(?P<indent>\s*)```(?P<info>\S*)\s*$").unwrap())
}

fn get_py_def_re() -> &'static Regex {
    PY_DEF_RE.get_or_init(|| Regex::new(r"^\s*(?:async\s+def|def|class)\s").unwrap())
}

#[derive(Debug, Clone)]
struct LangStyle {
    line: Vec<&'static str>,
    block: Option<(&'static str, &'static str)>,
    docstring: bool,
}

fn python_style() -> LangStyle {
    LangStyle { line: vec!["#"], block: None, docstring: true }
}

fn c_style() -> LangStyle {
    LangStyle { line: vec!["//"], block: Some(("/*", "*/")), docstring: false }
}

static LANG_STYLES: OnceLock<std::collections::HashMap<&'static str, LangStyle>> = OnceLock::new();

fn get_lang_styles() -> &'static std::collections::HashMap<&'static str, LangStyle> {
    LANG_STYLES.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        let py = python_style();
        m.insert("python", py.clone());
        m.insert("py", py);
        
        let hash_only = LangStyle { line: vec!["#"], block: None, docstring: false };
        m.insert("ruby", hash_only.clone());
        m.insert("rb", hash_only.clone());
        m.insert("yaml", hash_only.clone());
        m.insert("yml", hash_only.clone());
        m.insert("sh", hash_only.clone());
        m.insert("bash", hash_only.clone());
        m.insert("zsh", hash_only.clone());
        m.insert("shell", hash_only.clone());
        m.insert("perl", hash_only.clone());
        m.insert("r", hash_only.clone());
        m.insert("toml", hash_only.clone());
        
        m.insert("ini", LangStyle { line: vec!["#", ";"], block: None, docstring: false });

        let c = c_style();
        m.insert("javascript", c.clone());
        m.insert("js", c.clone());
        m.insert("typescript", c.clone());
        m.insert("ts", c.clone());
        m.insert("tsx", c.clone());
        m.insert("jsx", c.clone());
        m.insert("java", c.clone());
        m.insert("c", c.clone());
        m.insert("cpp", c.clone());
        m.insert("c++", c.clone());
        m.insert("h", c.clone());
        m.insert("hpp", c.clone());
        m.insert("cs", c.clone());
        m.insert("csharp", c.clone());
        m.insert("go", c.clone());
        m.insert("rust", c.clone());
        m.insert("rs", c.clone());
        m.insert("swift", c.clone());
        m.insert("kotlin", c.clone());
        m.insert("kt", c.clone());
        m.insert("scala", c.clone());
        m.insert("dart", c.clone());
        
        m.insert("php", LangStyle { line: vec!["//", "#"], block: Some(("/*", "*/")), docstring: false });
        m
    })
}

fn detect_style(info: &str) -> Option<LangStyle> {
    let info = info.trim().to_lowercase();
    if info.is_empty() {
        return None;
    }
    // Take first token before space, comma, or brace
    let info_clean = Regex::new(r"[\s,{]").unwrap().split(&info).next().unwrap_or("");
    get_lang_styles().get(info_clean).cloned()
}

fn string_flags(line: &str) -> Vec<bool> {
    let mut flags = vec![false; line.len()];
    let mut delim: Option<char> = None;
    let mut esc = false;

    // We do index-based iteration. Note that line.chars() can have multi-byte chars,
    // so we operate on byte indices safely or char indices. In comments and code,
    // ASCII quotes and backslashes dominate, but let's handle characters safely.
    let char_indices: Vec<(usize, char)> = line.char_indices().collect();
    for i in 0..char_indices.len() {
        let (byte_idx, c) = char_indices[i];
        let byte_len = c.len_utf8();
        
        if delim.is_some() {
            for offset in 0..byte_len {
                flags[byte_idx + offset] = true;
            }
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if Some(c) == delim {
                delim = None;
            }
        } else if c == '"' || c == '\'' || c == '`' {
            delim = Some(c);
        }
    }
    flags
}

fn find_comment_pos(line: &str, marker: &str) -> Option<usize> {
    let flags = string_flags(line);
    let mut start = 0;
    while let Some(pos) = line[start..].find(marker).map(|p| start + p) {
        if pos < flags.len() && flags[pos] {
            start = pos + marker.len();
            continue;
        }
        if marker == "//" && pos > 0 && line.as_bytes()[pos - 1] == b':' {
            start = pos + marker.len();
            continue;
        }
        return Some(pos);
    }
    None
}

fn remove_inline_block(line: &str, open_tok: &str, close_tok: &str) -> String {
    let flags = string_flags(line);
    let mut out = String::new();
    let mut i = 0;
    
    // Convert to bytes for fast prefix comparison, keeping in mind boundary alignment
    let bytes = line.as_bytes();
    let open_bytes = open_tok.as_bytes();
    let close_bytes = close_tok.as_bytes();
    let n = bytes.len();
    
    while i < n {
        if i + open_bytes.len() <= n && &bytes[i..i+open_bytes.len()] == open_bytes && (i >= flags.len() || !flags[i]) {
            if let Some(end) = line[i + open_bytes.len()..].find(close_tok).map(|p| i + open_bytes.len() + p) {
                i = end + close_bytes.len();
                continue;
            }
        }
        // Safely push next character
        if let Some(c) = line[i..].chars().next() {
            out.push(c);
            i += c.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn strip_trailing(line: &str, style: &LangStyle) -> String {
    let mut line_stripped = line.to_string();
    if let Some((open, close)) = style.block {
        line_stripped = remove_inline_block(&line_stripped, open, close);
    }
    let mut best_pos: Option<usize> = None;
    for &marker in &style.line {
        if let Some(pos) = find_comment_pos(&line_stripped, marker) {
            if best_pos.is_none() || pos < best_pos.unwrap() {
                best_pos = Some(pos);
            }
        }
    }
    if let Some(pos) = best_pos {
        line_stripped[..pos].trim_end().to_string()
    } else {
        line_stripped
    }
}

fn strip_multiline_c(content: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    let bytes = content.as_bytes();
    let n = bytes.len();
    let mut delim: Option<char> = None;
    let mut esc = false;

    while i < n {
        if let Some(c) = content[i..].chars().next() {
            let byte_len = c.len_utf8();
            if delim.is_some() {
                out.push(c);
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if Some(c) == delim {
                    delim = None;
                } else if c == '\n' && (delim == Some('"') || delim == Some('\'')) {
                    delim = None;
                }
                i += byte_len;
                continue;
            }
            if c == '"' || c == '\'' || c == '`' {
                delim = Some(c);
                out.push(c);
                i += byte_len;
                continue;
            }
            if i + 2 <= n && &bytes[i..i+2] == b"/*" {
                if let Some(end) = content[i + 2..].find("*/").map(|p| i + 2 + p) {
                    i = end + 2;
                    continue;
                } else {
                    out.push_str(&content[i..]);
                    return out;
                }
            }
            out.push(c);
            i += byte_len;
        } else {
            break;
        }
    }
    out
}

fn strip_python_docstrings(content: &str) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    let n = lines.len();
    let py_def_re = get_py_def_re();

    while i < n {
        out.push(lines[i].to_string());
        if !py_def_re.is_match(lines[i]) || !lines[i].trim_end().ends_with(':') {
            i += 1;
            continue;
        }
        i += 1;
        let mut blanks = Vec::new();
        while i < n && lines[i].trim().is_empty() {
            blanks.push(lines[i].to_string());
            i += 1;
        }
        if i >= n {
            out.extend(blanks);
            continue;
        }
        let ds_line = lines[i].trim_start();
        let mut quote: Option<&str> = None;
        for q in &["\"\"\"", "'''"] {
            if ds_line.starts_with(q) {
                quote = Some(q);
                break;
            }
        }
        if quote.is_none() {
            out.extend(blanks);
            continue;
        }
        let q = quote.unwrap();
        let rest = &ds_line[q.len()..];
        if rest.contains(q) {
            i += 1;
            continue;
        }
        i += 1;
        while i < n {
            if lines[i].contains(q) {
                i += 1;
                break;
            }
            i += 1;
        }
    }
    out.join("\n")
}

fn ml_delims(style: &LangStyle) -> Vec<&'static str> {
    let mut delims = Vec::new();
    if style.docstring {
        delims.push("\"\"\"");
        delims.push("'''");
    }
    if style.block.is_some() {
        delims.push("`");
    }
    delims
}

fn advance_ml(line: &str, mut state: Option<&'static str>, delims: &[&'static str]) -> Option<&'static str> {
    if delims.is_empty() {
        return None;
    }
    let mut i = 0;
    let n = line.len();
    while i < n {
        if let Some(s) = state {
            if line[i..].starts_with(s) {
                i += s.len();
                state = None;
                continue;
            }
            i += 1;
            continue;
        }
        let mut matched = None;
        for &d in delims {
            if line[i..].starts_with(d) {
                matched = Some(d);
                break;
            }
        }
        if let Some(m) = matched {
            state = Some(m);
            i += m.len();
            continue;
        }
        i += 1;
    }
    state
}

fn process_block(content: &str, style: &LangStyle, level: &str) -> String {
    let mut content_stripped = content.to_string();
    if level == "aggressive" {
        if style.block.is_some() {
            content_stripped = strip_multiline_c(&content_stripped);
        }
        if style.docstring {
            content_stripped = strip_python_docstrings(&content_stripped);
        }
    }

    let lines: Vec<&str> = content_stripped.split('\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    let delims = ml_delims(style);
    let mut ml_state: Option<&'static str> = None;

    for line in lines {
        if ml_state.is_some() {
            out.push(line.to_string());
            ml_state = advance_ml(line, ml_state, &delims);
            continue;
        }
        ml_state = advance_ml(line, ml_state, &delims);
        let stripped = line.trim_start();
        if stripped.starts_with("#!") {
            out.push(line.to_string());
            continue;
        }
        let mut is_full = false;
        for &marker in &style.line {
            if stripped.starts_with(marker) {
                is_full = true;
                break;
            }
        }
        if !is_full {
            if let Some((open, close)) = style.block {
                if stripped.starts_with(open) && stripped.trim_end().ends_with(close) {
                    is_full = true;
                }
            }
        }
        if is_full {
            continue;
        }
        if level == "lite" {
            out.push(line.to_string());
        } else {
            out.push(strip_trailing(line, style));
        }
    }
    out.join("\n")
}

pub fn compress_comment_strip(text: &str, level: &str) -> Result<EngineResult, String> {
    if !["off", "lite", "standard", "aggressive"].contains(&level) {
        return Err(format!("level must be off, lite, standard, or aggressive, got {:?}", level));
    }
    if level == "off" || text.is_empty() || !text.contains("```") {
        return Ok(identity_result(text));
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let mut out_lines = Vec::new();
    let mut in_fence = false;
    let mut fence_style: Option<LangStyle> = None;
    let mut buf = Vec::new();
    let mut stripped_lines = 0;
    
    let fence_re = get_fence_re();

    for line in lines {
        if let Some(caps) = fence_re.captures(line) {
            if in_fence {
                // Flush the block
                if fence_style.is_none() {
                    out_lines.extend(buf.clone());
                } else {
                    let content = buf.join("\n");
                    let processed = process_block(&content, fence_style.as_ref().unwrap(), level);
                    stripped_lines += content.matches('\n').count() - processed.matches('\n').count();
                    for l in processed.split('\n') {
                        out_lines.push(l.to_string());
                    }
                }
                buf.clear();
                in_fence = false;
                fence_style = None;
            } else {
                in_fence = true;
                fence_style = detect_style(&caps["info"]);
            }
            out_lines.push(line.to_string());
            continue;
        }
        if in_fence {
            buf.push(line.to_string());
        } else {
            out_lines.push(line.to_string());
        }
    }

    if in_fence {
        out_lines.extend(buf);
    }

    let out_text = out_lines.join("\n");
    let note = if stripped_lines > 0 {
        format!("stripped {} comment lines", stripped_lines)
    } else {
        "".to_string()
    };
    
    Ok(make_result(text, &out_text, note))
}
