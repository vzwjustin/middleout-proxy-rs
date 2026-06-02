use regex::{Regex, Captures};
use std::collections::HashMap;
use std::sync::OnceLock;

static URL_RE: OnceLock<Regex> = OnceLock::new();
static IDENT_RE: OnceLock<Regex> = OnceLock::new();
static CODE_FENCE_RE: OnceLock<Regex> = OnceLock::new();
static RESTORE_RE: OnceLock<Regex> = OnceLock::new();

fn get_url_re() -> &'static Regex {
    URL_RE.get_or_init(|| Regex::new(r"https?://\S+").unwrap())
}

fn get_ident_re() -> &'static Regex {
    IDENT_RE.get_or_init(|| {
        Regex::new(r"\b(?:[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9_]*|[A-Za-z_][A-Za-z0-9_]*_[A-Za-z0-9_]*|[A-Za-z0-9_]+\.[A-Za-z0-9_.]+|/[^\s]+|\.{1,2}/[^\s]+)\b").unwrap()
    })
}

fn get_code_fence_re() -> &'static Regex {
    CODE_FENCE_RE.get_or_init(|| Regex::new(r"```").unwrap())
}

fn get_restore_re() -> &'static Regex {
    RESTORE_RE.get_or_init(|| Regex::new(r"\x00(\d+)\x00").unwrap())
}

static MINIMAL_MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
static STANDARD_MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
static AGGRESSIVE_MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

fn get_minimal_map() -> &'static HashMap<&'static str, &'static str> {
    MINIMAL_MAP.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("function", "fn");
        m.insert("functions", "fns");
        m.insert("return", "ret");
        m.insert("returns", "rets");
        m.insert("import", "imp");
        m.insert("imports", "imps");
        m.insert("implementation", "impl");
        m.insert("implementations", "impls");
        m.insert("configuration", "cfg");
        m.insert("configurations", "cfgs");
        m.insert("documentation", "doc");
        m.insert("parameter", "param");
        m.insert("parameters", "params");
        m.insert("argument", "arg");
        m.insert("arguments", "args");
        m.insert("variable", "var");
        m.insert("variables", "vars");
        m.insert("directory", "dir");
        m.insert("directories", "dirs");
        m.insert("database", "db");
        m.insert("databases", "dbs");
        m.insert("application", "app");
        m.insert("applications", "apps");
        m.insert("environment", "env");
        m.insert("environments", "envs");
        m.insert("repository", "repo");
        m.insert("repositories", "repos");
        m.insert("command", "cmd");
        m.insert("commands", "cmds");
        m.insert("object", "obj");
        m.insert("objects", "objs");
        m.insert("request", "req");
        m.insert("response", "resp");
        m
    })
}

fn get_standard_map() -> &'static HashMap<&'static str, &'static str> {
    STANDARD_MAP.get_or_init(|| {
        let mut m = get_minimal_map().clone();
        m.insert("approximately", "~");
        m.insert("because", "bc");
        m.insert("without", "w/o");
        m.insert("with", "w/");
        m.insert("between", "btwn");
        m.insert("before", "b4");
        m.insert("after", "aft");
        m.insert("through", "thru");
        m.insert("though", "tho");
        m.insert("although", "altho");
        m.insert("however", "hwvr");
        m.insert("therefore", "thus");
        m.insert("something", "smth");
        m.insert("someone", "s1");
        m.insert("anyone", "any1");
        m.insert("everyone", "evry1");
        m.insert("people", "ppl");
        m.insert("about", "abt");
        m.insert("around", "arnd");
        m.insert("really", "rly");
        m.insert("should", "shd");
        m.insert("would", "wd");
        m.insert("could", "cd");
        m.insert("number", "num");
        m.insert("numbers", "nums");
        m.insert("message", "msg");
        m.insert("messages", "msgs");
        m.insert("package", "pkg");
        m.insert("packages", "pkgs");
        m.insert("version", "ver");
        m.insert("versions", "vers");
        m.insert("service", "svc");
        m.insert("services", "svcs");
        m.insert("context", "ctx");
        m.insert("contexts", "ctxs");
        m.insert("reference", "ref");
        m.insert("references", "refs");
        m.insert("performance", "perf");
        m.insert("operation", "op");
        m.insert("operations", "ops");
        m.insert("execute", "exec");
        m.insert("executes", "execs");
        m.insert("execution", "exec");
        m.insert("production", "prod");
        m.insert("development", "dev");
        m.insert("testing", "test");
        m.insert("example", "ex");
        m.insert("examples", "exs");
        m.insert("different", "diff");
        m.insert("difference", "diff");
        m.insert("specific", "spec");
        m.insert("specification", "spec");
        m.insert("previous", "prev");
        m.insert("current", "cur");
        m.insert("minimum", "min");
        m.insert("maximum", "max");
        m
    })
}

fn get_aggressive_map() -> &'static HashMap<&'static str, &'static str> {
    AGGRESSIVE_MAP.get_or_init(|| {
        let mut m = get_standard_map().clone();
        m.insert("as soon as possible", "ASAP");
        m.insert("for your information", "FYI");
        m.insert("in my opinion", "IMO");
        m.insert("by the way", "BTW");
        m.insert("for example", "e.g.");
        m.insert("that is", "i.e.");
        m.insert("and so on", "etc");
        m.insert("et cetera", "etc");
        m.insert("in other words", "iow");
        m.insert("as a result", "thus");
        m.insert("in addition", "also");
        m.insert("in conclusion", "thus");
        m.insert("on the other hand", "OTOH");
        m.insert("in the meantime", "meanwhile");
        m.insert("make sure", "ensure");
        m.insert("make sure to", "ensure");
        m.insert("in order to", "to");
        m.insert("due to", "from");
        m.insert("according to", "per");
        m.insert("regardless of", "despite");
        m.insert("in spite of", "despite");
        m.insert("as well as", "and");
        m.insert("such as", "like");
        m.insert("depending on", "per");
        m.insert("based on", "per");
        m.insert("in case of", "if");
        m.insert("in terms of", "re");
        m.insert("with respect to", "re");
        m.insert("with regard to", "re");
        m.insert("necessary", "needed");
        m.insert("additional", "extra");
        m.insert("approximately equal", "~");
        m.insert("currently", "now");
        m.insert("recently", "lately");
        m.insert("frequently", "often");
        m.insert("occasionally", "sometimes");
        m.insert("immediately", "now");
        m.insert("subsequently", "then");
        m.insert("consequently", "thus");
        m.insert("particularly", "esp");
        m.insert("especially", "esp");
        m.insert("generally", "usually");
        m.insert("typically", "usually");
        m.insert("primarily", "mainly");
        m.insert("fundamentally", "basically");
        m.insert("essentially", "basically");
        m.insert("absolutely", "yes");
        m.insert("definitely", "yes");
        m.insert("certainly", "yes");
        m.insert("probably", "prob");
        m.insert("possibly", "maybe");
        m.insert("process", "proc");
        m.insert("processes", "procs");
        m.insert("module", "mod");
        m.insert("modules", "mods");
        m.insert("library", "lib");
        m.insert("libraries", "libs");
        m.insert("schedule", "sched");
        m.insert("validate", "chk");
        m.insert("validation", "chk");
        m.insert("deprecated", "dep");
        m.insert("asynchronous", "async");
        m.insert("synchronous", "sync");
        m.insert("concurrent", "concur");
        m.insert("transaction", "tx");
        m.insert("transactions", "txs");
        m.insert("category", "cat");
        m.insert("categories", "cats");
        m.insert("language", "lang");
        m.insert("languages", "langs");
        m.insert("client", "cli");
        m.insert("clients", "clis");
        m.insert("server", "srv");
        m.insert("servers", "srvs");
        m.insert("framework", "fw");
        m.insert("frameworks", "fws");
        m.insert("interface", "iface");
        m.insert("interfaces", "ifaces");
        m.insert("structure", "struct");
        m.insert("structures", "structs");
        m.insert("string", "str");
        m.insert("strings", "strs");
        m.insert("integer", "int");
        m.insert("integers", "ints");
        m.insert("boolean", "bool");
        m.insert("booleans", "bools");
        m.insert("regular expression", "regex");
        m.insert("regular expressions", "regexes");
        m
    })
}

fn is_code_line(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
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

fn apply_dict(text: &str, mapping: &HashMap<&'static str, &'static str>) -> String {
    let mut items: Vec<(&&'static str, &&'static str)> = mapping.iter().collect();
    // Sort longest phrases first to avoid prefix collisions
    items.sort_by_key(|(k, _)| -(k.len() as isize));

    let mut out = text.to_string();
    for (src, dst) in items {
        // Whole-word boundary case-insensitive match (but only replaces fully lowercase matches)
        let pattern_str = format!(r"\b{}\b", regex::escape(src));
        if let Ok(re) = Regex::new(&pattern_str) {
            out = re.replace_all(&out, *dst).into_owned();
        }
    }
    out
}

pub fn compress_rtk(text: &str, level: &str) -> Result<String, String> {
    if !["minimal", "standard", "aggressive"].contains(&level) {
        return Err(format!("rtk level must be minimal, standard, or aggressive, got {:?}", level));
    }
    if text.is_empty() {
        return Ok(text.to_string());
    }

    let mapping = match level {
        "minimal" => get_minimal_map(),
        "standard" => get_standard_map(),
        "aggressive" => get_aggressive_map(),
        _ => unreachable!(),
    };

    let fence_re = get_code_fence_re();
    let parts: Vec<&str> = fence_re.split(text).collect();
    let mut rebuilt = Vec::with_capacity(parts.len());

    for (i, segment) in parts.iter().enumerate() {
        if i % 2 == 1 {
            rebuilt.push(segment.to_string());
            continue;
        }

        let lines: Vec<&str> = segment.split('\n').collect();
        let mut new_lines = Vec::with_capacity(lines.len());
        for line in lines {
            if is_code_line(line) {
                new_lines.push(line.to_string());
                continue;
            }
            let (protected, placeholders) = protect(line);
            let replaced = apply_dict(&protected, mapping);
            new_lines.push(restore(&replaced, &placeholders));
        }
        rebuilt.push(new_lines.join("\n"));
    }

    Ok(rebuilt.join("```"))
}
