use regex::{Regex, Captures};
use std::collections::HashSet;
use std::sync::OnceLock;

static FILLER: OnceLock<HashSet<&'static str>> = OnceLock::new();
static ARTICLES: OnceLock<HashSet<&'static str>> = OnceLock::new();
static ULTRA_DROP: OnceLock<HashSet<&'static str>> = OnceLock::new();
static AGGRESSIVE_ABBR: OnceLock<HashSet<(&'static str, &'static str)>> = OnceLock::new();

fn get_filler() -> &'static HashSet<&'static str> {
    FILLER.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("very");
        s.insert("really");
        s.insert("just");
        s.insert("quite");
        s.insert("actually");
        s.insert("basically");
        s.insert("literally");
        s.insert("simply");
        s.insert("essentially");
        s
    })
}

fn get_articles() -> &'static HashSet<&'static str> {
    ARTICLES.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("the");
        s.insert("a");
        s.insert("an");
        s
    })
}

fn get_ultra_drop() -> &'static HashSet<&'static str> {
    ULTRA_DROP.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert("is");
        s.insert("are");
        s.insert("was");
        s.insert("were");
        s.insert("am");
        s.insert("be");
        s.insert("been");
        s.insert("being");
        s.insert("and");
        s.insert("or");
        s
    })
}

fn get_aggressive_abbr() -> &'static HashSet<(&'static str, &'static str)> {
    AGGRESSIVE_ABBR.get_or_init(|| {
        let mut s = HashSet::new();
        s.insert(("function", "fn"));
        s.insert(("functions", "fns"));
        s.insert(("return", "ret"));
        s.insert(("returns", "rets"));
        s.insert(("should", "shd"));
        s.insert(("would", "wd"));
        s.insert(("could", "cd"));
        s.insert(("implementation", "impl"));
        s.insert(("configuration", "cfg"));
        s.insert(("documentation", "doc"));
        s.insert(("parameter", "param"));
        s.insert(("parameters", "params"));
        s.insert(("argument", "arg"));
        s.insert(("arguments", "args"));
        s.insert(("variable", "var"));
        s.insert(("variables", "vars"));
        s.insert(("between", "btwn"));
        s.insert(("approximately", "~"));
        s
    })
}

fn get_abbr_value(low: &str) -> Option<&'static str> {
    for (k, v) in get_aggressive_abbr() {
        if *k == low {
            return Some(*v);
        }
    }
    None
}

// Pleasantry patterns with regexes
struct RegexReplacement {
    pattern: Regex,
    replacement: &'static str,
}

static PLEASANTRY_PATTERNS: OnceLock<Vec<RegexReplacement>> = OnceLock::new();
static PHRASE_COLLAPSES: OnceLock<Vec<RegexReplacement>> = OnceLock::new();

fn get_pleasantry_patterns() -> &'static Vec<RegexReplacement> {
    PLEASANTRY_PATTERNS.get_or_init(|| {
        vec![
            RegexReplacement { pattern: Regex::new(r"(?i)\bplease\s+").unwrap(), replacement: "" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bthanks?(?:\s+you)?\b[,.!]?\s*").unwrap(), replacement: "" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bthank you\b[,.!]?\s*").unwrap(), replacement: "" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bcould you\b\s*").unwrap(), replacement: "" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bwould you\b\s*").unwrap(), replacement: "" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bcan you\b\s*").unwrap(), replacement: "" },
        ]
    })
}

fn get_phrase_collapses() -> &'static Vec<RegexReplacement> {
    PHRASE_COLLAPSES.get_or_init(|| {
        vec![
            RegexReplacement { pattern: Regex::new(r"(?i)\bin order to\b").unwrap(), replacement: "to" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bmake sure to\b").unwrap(), replacement: "ensure" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bmake sure that\b").unwrap(), replacement: "ensure" },
            RegexReplacement { pattern: Regex::new(r"(?i)\byou should\b").unwrap(), replacement: "do" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bin terms of\b").unwrap(), replacement: "re" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bas well as\b").unwrap(), replacement: "and" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bdue to the fact that\b").unwrap(), replacement: "because" },
            RegexReplacement { pattern: Regex::new(r"(?i)\bat this point in time\b").unwrap(), replacement: "now" },
        ]
    })
}

static URL_RE: OnceLock<Regex> = OnceLock::new();
static IDENT_RE: OnceLock<Regex> = OnceLock::new();
static CODE_FENCE_RE: OnceLock<Regex> = OnceLock::new();
static KEEP_PUNCT_RE: OnceLock<Regex> = OnceLock::new();
static RESTORE_RE: OnceLock<Regex> = OnceLock::new();
static SPLIT_WORD_RE: OnceLock<Regex> = OnceLock::new();
static WHITESPACE_RE: OnceLock<Regex> = OnceLock::new();
static WHITESPACE_NL_RE: OnceLock<Regex> = OnceLock::new();
static PUNCT_SPACE_RE: OnceLock<Regex> = OnceLock::new();

fn get_url_re() -> &'static Regex { URL_RE.get_or_init(|| Regex::new(r"https?://\S+").unwrap()) }
fn get_ident_re() -> &'static Regex {
    IDENT_RE.get_or_init(|| {
        Regex::new(r"\b(?:[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9_]*|[A-Za-z_][A-Za-z0-9_]*_[A-Za-z0-9_]*|[A-Za-z0-9_]+\.[A-Za-z0-9_.]+|/[^\s]+|\.{1,2}/[^\s]+)\b").unwrap()
    })
}
fn get_code_fence_re() -> &'static Regex { CODE_FENCE_RE.get_or_init(|| Regex::new(r"```").unwrap()) }
fn get_keep_punct_re() -> &'static Regex { KEEP_PUNCT_RE.get_or_init(|| Regex::new(r"[,.;:!?]+$").unwrap()) }
fn get_restore_re() -> &'static Regex { RESTORE_RE.get_or_init(|| Regex::new(r"\x00(\d+)\x00").unwrap()) }
fn get_split_word_re() -> &'static Regex { SPLIT_WORD_RE.get_or_init(|| Regex::new(r"^(\W*)(.*?)(\W*)$").unwrap()) }
fn get_whitespace_re() -> &'static Regex { WHITESPACE_RE.get_or_init(|| Regex::new(r"[ \t]+").unwrap()) }
fn get_whitespace_nl_re() -> &'static Regex { WHITESPACE_NL_RE.get_or_init(|| Regex::new(r" *\n *").unwrap()) }
fn get_punct_space_re() -> &'static Regex { PUNCT_SPACE_RE.get_or_init(|| Regex::new(r"\s+([,.;:!?])").unwrap()) }

fn is_code_line(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn emit_dropped_punct(out: &mut Vec<String>, trail: &str) {
    let re = get_keep_punct_re();
    if let Some(m) = re.find(trail) {
        out.push(m.as_str().to_string());
    }
}

fn protect(text: &str) -> (String, Vec<String>) {
    let text = text.replace('\x00', "");
    let mut placeholders = Vec::new();

    let url_re = get_url_re();
    let text = url_re.replace_all(&text, |caps: &Captures| {
        placeholders.push(caps[0].to_string());
        format!("\x00{}\x00", placeholders.len() - 1)
    }).into_owned();

    let ident_re = get_ident_re();
    let text = ident_re.replace_all(&text, |caps: &Captures| {
        placeholders.push(caps[0].to_string());
        format!("\x00{}\x00", placeholders.len() - 1)
    }).into_owned();

    (text, placeholders)
}

fn restore(text: &str, placeholders: &[String]) -> String {
    let re = get_restore_re();
    re.replace_all(text, |caps: &Captures| {
        let idx = caps[1].parse::<usize>().unwrap_or(usize::MAX);
        if idx < placeholders.len() {
            placeholders[idx].clone()
        } else {
            caps[0].to_string()
        }
    }).into_owned()
}

fn process_prose(text: &str, level: &str) -> String {
    let (mut text, placeholders) = protect(text);

    if ["standard", "aggressive", "ultra"].contains(&level) {
        for replacement in get_pleasantry_patterns() {
            text = replacement.pattern.replace_all(&text, replacement.replacement).into_owned();
        }
        for replacement in get_phrase_collapses() {
            text = replacement.pattern.replace_all(&text, replacement.replacement).into_owned();
        }
    }

    // Split on whitespace tokens
    let mut tokens = Vec::new();
    let mut last_idx = 0;
    for m in Regex::new(r"\s+").unwrap().find_iter(&text) {
        if m.start() > last_idx {
            tokens.push(&text[last_idx..m.start()]);
        }
        tokens.push(m.as_str());
        last_idx = m.end();
    }
    if last_idx < text.len() {
        tokens.push(&text[last_idx..]);
    }

    let mut out = Vec::with_capacity(tokens.len());
    for tok in tokens {
        if tok.is_empty() || tok.chars().all(|c| c.is_whitespace()) {
            out.push(tok.to_string());
            continue;
        }

        let split_word_re = get_split_word_re();
        if let Some(caps) = split_word_re.captures(tok) {
            let lead = &caps[1];
            let mut core = caps[2].to_string();
            let trail = &caps[3];
            let low = core.to_lowercase();
            let is_fully_lower = core.chars().all(|c| c.is_lowercase());

            // Lite+: drop filler
            if get_filler().contains(low.as_str()) {
                emit_dropped_punct(&mut out, trail);
                continue;
            }
            // Lite+: drop articles (only when fully lowercase to avoid sentence starts)
            if get_articles().contains(low.as_str()) && is_fully_lower {
                emit_dropped_punct(&mut out, trail);
                continue;
            }
            // Aggressive: abbreviate
            if ["aggressive", "ultra"].contains(&level) {
                if let Some(abbr) = get_abbr_value(low.as_str()) {
                    core = abbr.to_string();
                }
            }
            // Aggressive: drop "that" before clauses
            if ["aggressive", "ultra"].contains(&level) && low == "that" && is_fully_lower {
                emit_dropped_punct(&mut out, trail);
                continue;
            }
            // Ultra: drop copulas/conjunctions
            if level == "ultra" && get_ultra_drop().contains(low.as_str()) && is_fully_lower {
                emit_dropped_punct(&mut out, trail);
                continue;
            }

            out.push(format!("{}{}{}", lead, core, trail));
        } else {
            out.push(tok.to_string());
        }
    }

    let joined = out.join("");
    // Collapse runs of whitespace introduced by deletions, but preserve newlines
    let joined = get_whitespace_re().replace_all(&joined, " ").into_owned();
    let joined = get_whitespace_nl_re().replace_all(&joined, "\n").into_owned();
    let joined = get_punct_space_re().replace_all(&joined, "$1").into_owned();

    restore(&joined, &placeholders)
}

pub fn compress_caveman(text: &str, level: &str) -> Result<String, String> {
    if !["lite", "standard", "aggressive", "ultra"].contains(&level) {
        return Err(format!("caveman level must be lite, standard, aggressive, or ultra, got {:?}", level));
    }
    if text.is_empty() {
        return Ok(text.to_string());
    }

    let code_fence_re = get_code_fence_re();
    let parts: Vec<&str> = code_fence_re.split(text).collect();
    let mut rebuilt = Vec::with_capacity(parts.len());

    for (i, segment) in parts.iter().enumerate() {
        if i % 2 == 1 {
            // Inside code block - pass through
            rebuilt.push(segment.to_string());
            continue;
        }

        let lines: Vec<&str> = segment.split('\n').collect();
        let mut processed = Vec::with_capacity(lines.len());
        for line in lines {
            if is_code_line(line) {
                processed.push(line.to_string());
            } else {
                processed.push(process_prose(line, level));
            }
        }
        rebuilt.push(processed.join("\n"));
    }

    Ok(rebuilt.join("```"))
}
