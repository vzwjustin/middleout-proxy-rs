use std::collections::HashMap;
use parking_lot::Mutex;
use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub struct PriceEntry {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_write_per_mtok: Option<f64>,
    pub cache_read_per_mtok: Option<f64>,
}

impl PriceEntry {
    pub fn total_usd(
        &self,
        input_tokens: i64,
        output_tokens: i64,
        cache_write_tokens: i64,
        cache_read_tokens: i64,
    ) -> f64 {
        let mut cost = 0.0;
        cost += (input_tokens.max(0) as f64) * self.input_per_mtok / 1_000_000.0;
        cost += (output_tokens.max(0) as f64) * self.output_per_mtok / 1_000_000.0;
        if let Some(write_rate) = self.cache_write_per_mtok {
            cost += (cache_write_tokens.max(0) as f64) * write_rate / 1_000_000.0;
        }
        if let Some(read_rate) = self.cache_read_per_mtok {
            cost += (cache_read_tokens.max(0) as f64) * read_rate / 1_000_000.0;
        }
        // Round to 8 decimal places
        (cost * 100_000_000.0).round() / 100_000_000.0
    }
}

pub static PRICE_TABLE: &[((&str, &str), PriceEntry)] = &[
    (("anthropic", "claude-opus-4"), PriceEntry { input_per_mtok: 15.00, output_per_mtok: 75.00, cache_write_per_mtok: Some(18.75), cache_read_per_mtok: Some(1.50) }),
    (("anthropic", "claude-sonnet-4"), PriceEntry { input_per_mtok: 3.00, output_per_mtok: 15.00, cache_write_per_mtok: Some(3.75), cache_read_per_mtok: Some(0.30) }),
    (("anthropic", "claude-haiku-4"), PriceEntry { input_per_mtok: 0.80, output_per_mtok: 4.00, cache_write_per_mtok: Some(1.00), cache_read_per_mtok: Some(0.08) }),
    (("anthropic", "claude-3-5-sonnet"), PriceEntry { input_per_mtok: 3.00, output_per_mtok: 15.00, cache_write_per_mtok: Some(3.75), cache_read_per_mtok: Some(0.30) }),
    (("anthropic", "claude-3-7-sonnet"), PriceEntry { input_per_mtok: 3.00, output_per_mtok: 15.00, cache_write_per_mtok: Some(3.75), cache_read_per_mtok: Some(0.30) }),
    (("anthropic", "claude-3-5-haiku"), PriceEntry { input_per_mtok: 0.80, output_per_mtok: 4.00, cache_write_per_mtok: Some(1.00), cache_read_per_mtok: Some(0.08) }),
    (("anthropic", "claude-3-opus"), PriceEntry { input_per_mtok: 15.00, output_per_mtok: 75.00, cache_write_per_mtok: Some(18.75), cache_read_per_mtok: Some(1.50) }),
    (("anthropic", "claude-3-haiku"), PriceEntry { input_per_mtok: 0.25, output_per_mtok: 1.25, cache_write_per_mtok: Some(0.30), cache_read_per_mtok: Some(0.03) }),
    (("openai", "gpt-4o-mini"), PriceEntry { input_per_mtok: 0.15, output_per_mtok: 0.60, cache_write_per_mtok: None, cache_read_per_mtok: None }),
    (("openai", "gpt-4o"), PriceEntry { input_per_mtok: 2.50, output_per_mtok: 10.00, cache_write_per_mtok: None, cache_read_per_mtok: None }),
    (("openai", "gpt-4-turbo"), PriceEntry { input_per_mtok: 10.00, output_per_mtok: 30.00, cache_write_per_mtok: None, cache_read_per_mtok: None }),
    (("google", "gemini-1.5-flash"), PriceEntry { input_per_mtok: 0.075, output_per_mtok: 0.30, cache_write_per_mtok: None, cache_read_per_mtok: None }),
    (("google", "gemini-1.5-pro"), PriceEntry { input_per_mtok: 1.25, output_per_mtok: 5.00, cache_write_per_mtok: None, cache_read_per_mtok: None }),
    (("google", "gemini-2.0-flash"), PriceEntry { input_per_mtok: 0.10, output_per_mtok: 0.40, cache_write_per_mtok: None, cache_read_per_mtok: None }),
    (("ollama", ""), PriceEntry { input_per_mtok: 0.0, output_per_mtok: 0.0, cache_write_per_mtok: None, cache_read_per_mtok: None }),
    (("local", ""), PriceEntry { input_per_mtok: 0.0, output_per_mtok: 0.0, cache_write_per_mtok: None, cache_read_per_mtok: None }),
];

pub fn lookup_price(provider: &str, model: &str) -> Option<PriceEntry> {
    let mut best_prefix = "";
    let mut best_entry = None;
    for &((prov, prefix), entry) in PRICE_TABLE {
        if prov != provider {
            continue;
        }
        if model.starts_with(prefix) {
            if best_entry.is_none() || prefix.len() > best_prefix.len() {
                best_prefix = prefix;
                best_entry = Some(entry);
            }
        }
    }
    best_entry
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RequestCost {
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_write_tokens: i64,
    pub cache_read_tokens: i64,
    pub usd: f64,
    pub matched: bool,
}

pub fn estimate(
    provider: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
) -> RequestCost {
    let entry = lookup_price(provider, model);
    match entry {
        Some(e) => {
            let usd = e.total_usd(input_tokens, output_tokens, cache_write_tokens, cache_read_tokens);
            RequestCost {
                provider: provider.to_string(),
                model: model.to_string(),
                input_tokens,
                output_tokens,
                cache_write_tokens,
                cache_read_tokens,
                usd,
                matched: true,
            }
        }
        None => {
            RequestCost {
                provider: provider.to_string(),
                model: model.to_string(),
                input_tokens,
                output_tokens,
                cache_write_tokens,
                cache_read_tokens,
                usd: 0.0,
                matched: false,
            }
        }
    }
}

pub fn extract_usage_from_anthropic(payload: &Value) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    out.insert("input_tokens".to_string(), 0);
    out.insert("output_tokens".to_string(), 0);
    out.insert("cache_write_tokens".to_string(), 0);
    out.insert("cache_read_tokens".to_string(), 0);

    let usage = match payload.get("usage") {
        Some(u) => u,
        None => return out,
    };

    let parse_int = |key: &str| -> i64 {
        usage.get(key)
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0)
    };

    out.insert("input_tokens".to_string(), parse_int("input_tokens"));
    out.insert("output_tokens".to_string(), parse_int("output_tokens"));
    out.insert("cache_write_tokens".to_string(), parse_int("cache_creation_input_tokens"));
    out.insert("cache_read_tokens".to_string(), parse_int("cache_read_input_tokens"));
    out
}

pub struct SSEUsageAccumulator {
    buf: Vec<u8>,
    usage: HashMap<String, i64>,
    model: Option<String>,
    got_message_start: bool,
}

impl SSEUsageAccumulator {
    pub fn new() -> Self {
        let mut usage = HashMap::new();
        usage.insert("input_tokens".to_string(), 0);
        usage.insert("output_tokens".to_string(), 0);
        usage.insert("cache_write_tokens".to_string(), 0);
        usage.insert("cache_read_tokens".to_string(), 0);

        SSEUsageAccumulator {
            buf: Vec::new(),
            usage,
            model: None,
            got_message_start: false,
        }
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn saw_message_start(&self) -> bool {
        self.got_message_start
    }

    pub fn snapshot(&self) -> HashMap<String, i64> {
        self.usage.clone()
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        self.buf.extend_from_slice(chunk);
        
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line_bytes = self.buf[..pos].to_vec();
            self.buf.drain(..=pos);
            
            if line_bytes.ends_with(b"\r") {
                line_bytes.pop();
            }

            if !line_bytes.starts_with(b"data:") {
                continue;
            }

            let mut payload_bytes = &line_bytes[5..];
            while !payload_bytes.is_empty() && payload_bytes[0].is_ascii_whitespace() {
                payload_bytes = &payload_bytes[1..];
            }

            if payload_bytes.is_empty() || payload_bytes == b"[DONE]" {
                continue;
            }

            let decoded = match std::str::from_utf8(payload_bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let event: Value = match serde_json::from_str(decoded) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Value::Object(map) = event {
                self.consume(map);
            }
        }
    }

    fn consume(&mut self, event: serde_json::Map<String, Value>) {
        let ev_type = event.get("type").and_then(|v| v.as_str());
        if ev_type == Some("message_start") {
            if let Some(message) = event.get("message").and_then(|v| v.as_object()) {
                if let Some(model) = message.get("model").and_then(|v| v.as_str()) {
                    if !model.is_empty() {
                        self.model = Some(model.to_string());
                    }
                }
                self.merge_usage(message.get("usage"));
            }
            self.got_message_start = true;
        } else if ev_type == Some("message_delta") {
            self.merge_usage(event.get("usage"));
        }
    }

    fn merge_usage(&mut self, usage_val: Option<&Value>) {
        let usage = match usage_val.and_then(|v| v.as_object()) {
            Some(u) => u,
            None => return,
        };

        for &(key, json_key) in &[
            ("input_tokens", "input_tokens"),
            ("output_tokens", "output_tokens"),
            ("cache_write_tokens", "cache_creation_input_tokens"),
            ("cache_read_tokens", "cache_read_input_tokens"),
        ] {
            let val = usage.get(json_key)
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0);
            
            if val > *self.usage.get(key).unwrap_or(&0) {
                self.usage.insert(key.to_string(), val);
            }
        }
    }
}

pub struct CostTracker {
    inner: Mutex<CostTrackerInner>,
}

struct CostTrackerInner {
    by_model: HashMap<String, ModelCostSummary>,
    total_usd: f64,
    total_requests: i64,
    unmatched_requests: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ModelCostSummary {
    requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    usd: f64,
}

impl CostTracker {
    pub fn new() -> Self {
        CostTracker {
            inner: Mutex::new(CostTrackerInner {
                by_model: HashMap::new(),
                total_usd: 0.0,
                total_requests: 0,
                unmatched_requests: 0,
            }),
        }
    }

    pub fn record(&self, cost: &RequestCost) {
        let mut inner = self.inner.lock();
        let key = format!("{}:{}", cost.provider, cost.model);
        
        let row = inner.by_model.entry(key).or_insert(ModelCostSummary {
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            usd: 0.0,
        });

        row.requests += 1;
        row.input_tokens += cost.input_tokens;
        row.output_tokens += cost.output_tokens;
        row.cache_write_tokens += cost.cache_write_tokens;
        row.cache_read_tokens += cost.cache_read_tokens;
        row.usd += cost.usd;

        inner.total_usd += cost.usd;
        inner.total_requests += 1;
        if !cost.matched {
            inner.unmatched_requests += 1;
        }
    }

    pub fn snapshot(&self) -> Value {
        let inner = self.inner.lock();
        serde_json::json!({
            "total_usd": (inner.total_usd * 1_000_000.0).round() / 1_000_000.0,
            "total_requests": inner.total_requests,
            "unmatched_requests": inner.unmatched_requests,
            "by_model": inner.by_model,
        })
    }

    pub fn reset(&self) {
        let mut inner = self.inner.lock();
        inner.by_model.clear();
        inner.total_usd = 0.0;
        inner.total_requests = 0;
        inner.unmatched_requests = 0;
    }
}

pub struct UsageBudget {
    pub char_limit: Option<i64>,
    pub token_limit: Option<i64>,
    inner: Mutex<UsageBudgetInner>,
}

struct UsageBudgetInner {
    chars: i64,
    tokens: i64,
}

impl UsageBudget {
    pub fn new(char_limit: Option<i64>, token_limit: Option<i64>) -> Self {
        UsageBudget {
            char_limit,
            token_limit,
            inner: Mutex::new(UsageBudgetInner { chars: 0, tokens: 0 }),
        }
    }

    pub fn record(&self, chars: i64, tokens: i64) {
        let mut inner = self.inner.lock();
        inner.chars += chars.max(0);
        inner.tokens += tokens.max(0);
    }

    pub fn reset(&self) {
        let mut inner = self.inner.lock();
        inner.chars = 0;
        inner.tokens = 0;
    }

    pub fn snapshot(&self) -> Value {
        let inner = self.inner.lock();
        let exceeded = self.exceeded_locked(&inner);
        serde_json::json!({
            "chars_used": inner.chars,
            "tokens_used": inner.tokens,
            "char_limit": self.char_limit,
            "token_limit": self.token_limit,
            "exceeded": exceeded,
        })
    }

    pub fn exceeded(&self) -> bool {
        let inner = self.inner.lock();
        self.exceeded_locked(&inner)
    }

    fn exceeded_locked(&self, inner: &UsageBudgetInner) -> bool {
        if let Some(limit) = self.char_limit {
            if inner.chars >= limit {
                return true;
            }
        }
        if let Some(limit) = self.token_limit {
            if inner.tokens >= limit {
                return true;
            }
        }
        false
    }
}

