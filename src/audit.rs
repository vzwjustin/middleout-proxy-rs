use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use parking_lot::Mutex;
use serde_json::Value;
use crate::config::Settings;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressionEvent {
    pub path: String,
    pub mode: String,
    pub original_chars: usize,
    pub compressed_chars: usize,
    pub sha256: String,
    #[serde(default)]
    pub note: String,
    pub sample_before: Option<String>,
    pub sample_after: Option<String>,
}

impl CompressionEvent {
    pub fn chars_saved(&self) -> usize {
        self.original_chars.saturating_sub(self.compressed_chars)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressionAudit {
    pub endpoint: String,
    #[serde(default)]
    pub events: Vec<CompressionEvent>,
    #[serde(default)]
    pub cache_hits: usize,
    #[serde(default)]
    pub cache_misses: usize,
    #[serde(default)]
    pub protected_blocks: usize,
}

impl CompressionAudit {
    pub fn new(endpoint: &str) -> Self {
        CompressionAudit {
            endpoint: endpoint.to_string(),
            events: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
            protected_blocks: 0,
        }
    }

    pub fn original_chars(&self) -> usize {
        self.events.iter().map(|e| e.original_chars).sum()
    }

    pub fn compressed_chars(&self) -> usize {
        self.events.iter().map(|e| e.compressed_chars).sum()
    }

    pub fn chars_saved(&self) -> usize {
        self.events.iter().map(|e| e.chars_saved()).sum()
    }

    pub fn touched(&self) -> bool {
        !self.events.is_empty()
    }
}

pub static LATENCY_BINS_MS: &[f64] = &[
    1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0,
    1000.0, 2500.0, 5000.0, 10000.0, 30000.0, 60000.0, 120000.0,
];

fn bin_index(latency_ms: f64) -> usize {
    for (i, &edge) in LATENCY_BINS_MS.iter().enumerate() {
        if latency_ms <= edge {
            return i;
        }
    }
    LATENCY_BINS_MS.len()
}

fn quantile_from_hist(counts: &[usize], total: usize, q: f64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let target = ((q * (total as f64)).ceil() as usize).max(1);
    let mut cum = 0;
    for (i, &c) in counts.iter().enumerate() {
        cum += c;
        if cum >= target {
            if i < LATENCY_BINS_MS.len() {
                return LATENCY_BINS_MS[i];
            }
            return LATENCY_BINS_MS[LATENCY_BINS_MS.len() - 1] * 2.0;
        }
    }
    LATENCY_BINS_MS[LATENCY_BINS_MS.len() - 1] * 2.0
}

#[derive(Debug, Clone)]
struct Bucket {
    minute_ts: i64,
    requests: usize,
    errors: usize,
    chars_saved_in: usize,
    chars_saved_out: usize,
    bytes_in: usize,
    bytes_out: usize,
    engines: HashMap<String, usize>,
    by_provider: HashMap<String, ProviderTrafficSummary>,
    latency_counts: Vec<usize>,
    latency_total: usize,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProviderTrafficSummary {
    requests: usize,
    compressed_requests: usize,
    errors: usize,
    chars_saved_in: usize,
    chars_saved_out: usize,
    bytes_in: usize,
    bytes_out: usize,
    engines: HashMap<String, usize>,
}

impl ProviderTrafficSummary {
    fn observe(
        &mut self,
        is_error: bool,
        chars_saved_in: usize,
        chars_saved_out: usize,
        bytes_in: usize,
        bytes_out: usize,
        engines: &HashMap<String, usize>,
    ) {
        self.requests += 1;
        if chars_saved_in > 0 || chars_saved_out > 0 {
            self.compressed_requests += 1;
        }
        if is_error {
            self.errors += 1;
        }
        self.chars_saved_in += chars_saved_in;
        self.chars_saved_out += chars_saved_out;
        self.bytes_in += bytes_in;
        self.bytes_out += bytes_out;

        for (engine, &saved) in engines {
            if saved == 0 {
                continue;
            }
            *self.engines.entry(engine.clone()).or_insert(0) += saved;
        }
    }
}

impl Bucket {
    fn new(minute_ts: i64) -> Self {
        Bucket {
            minute_ts,
            requests: 0,
            errors: 0,
            chars_saved_in: 0,
            chars_saved_out: 0,
            bytes_in: 0,
            bytes_out: 0,
            engines: HashMap::new(),
            by_provider: HashMap::new(),
            latency_counts: vec![0; LATENCY_BINS_MS.len() + 1],
            latency_total: 0,
        }
    }

    fn to_dict(&self) -> Value {
        serde_json::json!({
            "minute_ts": self.minute_ts,
            "requests": self.requests,
            "errors": self.errors,
            "chars_saved_in": self.chars_saved_in,
            "chars_saved_out": self.chars_saved_out,
            "bytes_in": self.bytes_in,
            "bytes_out": self.bytes_out,
            "engines": self.engines,
            "by_provider": self.by_provider,
            "p50_ms": (quantile_from_hist(&self.latency_counts, self.latency_total, 0.50) * 100.0).round() / 100.0,
            "p95_ms": (quantile_from_hist(&self.latency_counts, self.latency_total, 0.95) * 100.0).round() / 100.0,
        })
    }
}

pub fn provider_for_request(path: &str, model: Option<&str>) -> &'static str {
    let normalized = path.trim().trim_start_matches('/').trim_end_matches('/');
    if normalized == "v1/messages" || normalized.starts_with("v1/messages/") {
        return "anthropic";
    }
    if normalized == "v1/responses"
        || normalized.starts_with("v1/responses/")
        || normalized == "v1/models"
        || normalized.starts_with("v1/models/")
        || normalized == "v1/chat/completions"
        || normalized.starts_with("v1/chat/completions/")
        || normalized == "v1/completions"
        || normalized.starts_with("v1/completions/")
        || normalized == "v1/embeddings"
        || normalized.starts_with("v1/embeddings/")
    {
        return "openai";
    }

    let Some(model) = model else {
        return "unknown";
    };
    let model = model.trim().to_ascii_lowercase();
    if model.starts_with("claude-") {
        return "anthropic";
    }
    if model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
        || model.starts_with("codex-")
    {
        return "openai";
    }
    "unknown"
}

pub struct ProxyStats {
    pub started_at: f64,
    pub requests_total: usize,
    pub compressed_requests: usize,
    pub chars_saved_in: usize,
    pub chars_saved_out: usize,
    pub upstream_errors: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub protected_blocks: usize,
    pub bytes_in_total: usize,
    pub bytes_out_total: usize,
    pub engines_total: HashMap<String, usize>,
    pub by_provider: HashMap<String, ProviderTrafficSummary>,
    window_minutes: usize,
    recent_max: usize,
    buckets: HashMap<i64, Bucket>,
    recent: VecDeque<Value>,
    latency_global_counts: Vec<usize>,
    latency_global_total: usize,
}

impl ProxyStats {
    pub fn new(window_minutes: usize, recent_max: usize) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        ProxyStats {
            started_at: now,
            requests_total: 0,
            compressed_requests: 0,
            chars_saved_in: 0,
            chars_saved_out: 0,
            upstream_errors: 0,
            cache_hits: 0,
            cache_misses: 0,
            protected_blocks: 0,
            bytes_in_total: 0,
            bytes_out_total: 0,
            engines_total: HashMap::new(),
            by_provider: HashMap::new(),
            window_minutes,
            recent_max,
            buckets: HashMap::new(),
            recent: VecDeque::with_capacity(recent_max),
            latency_global_counts: vec![0; LATENCY_BINS_MS.len() + 1],
            latency_global_total: 0,
        }
    }

    pub fn snapshot(&self) -> Value {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        serde_json::json!({
            "started_at": self.started_at,
            "requests_total": self.requests_total,
            "compressed_requests": self.compressed_requests,
            "chars_saved_in": self.chars_saved_in,
            "chars_saved_out": self.chars_saved_out,
            "upstream_errors": self.upstream_errors,
            "cache_hits": self.cache_hits,
            "cache_misses": self.cache_misses,
            "protected_blocks": self.protected_blocks,
            "bytes_in_total": self.bytes_in_total,
            "bytes_out_total": self.bytes_out_total,
            "engines_total": self.engines_total,
            "by_provider": self.by_provider,
            "uptime_s": (now - self.started_at).round(),
            "p50_ms": (quantile_from_hist(&self.latency_global_counts, self.latency_global_total, 0.50) * 100.0).round() / 100.0,
            "p95_ms": (quantile_from_hist(&self.latency_global_counts, self.latency_global_total, 0.95) * 100.0).round() / 100.0,
        })
    }

    pub fn timeseries(&mut self, now: Option<f64>) -> Vec<Value> {
        let current = now.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        });
        self.evict(current);

        let mut keys: Vec<i64> = self.buckets.keys().cloned().collect();
        keys.sort();

        keys.iter()
            .map(|k| self.buckets.get(k).unwrap().to_dict())
            .collect()
    }

    pub fn recent_records(&self, n: usize) -> Vec<Value> {
        let len = self.recent.len();
        let take_n = n.min(len);
        let start = len - take_n;
        self.recent.iter().skip(start).cloned().collect()
    }

    pub fn observe(
        &mut self,
        method: &str,
        path: &str,
        status_code: Option<u16>,
        chars_saved_in: usize,
        chars_saved_out: usize,
        engines: &HashMap<String, usize>,
        latency_ms: f64,
        bytes_in: usize,
        bytes_out: usize,
        request_id: Option<&str>,
        is_error: bool,
        request_audit_summary: Option<Value>,
        response_audit_summary: Option<Value>,
        model: Option<&str>,
        now: Option<f64>,
    ) {
        let ts = now.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        });

        self.requests_total += 1;
        if chars_saved_in > 0 || chars_saved_out > 0 {
            self.compressed_requests += 1;
        }
        self.chars_saved_in += chars_saved_in;
        self.chars_saved_out += chars_saved_out;
        self.bytes_in_total += bytes_in;
        self.bytes_out_total += bytes_out;
        if is_error {
            self.upstream_errors += 1;
        }

        for (engine, &saved) in engines {
            if saved == 0 {
                continue;
            }
            *self.engines_total.entry(engine.clone()).or_insert(0) += saved;
        }

        let provider = provider_for_request(path, model);
        self.by_provider
            .entry(provider.to_string())
            .or_default()
            .observe(is_error, chars_saved_in, chars_saved_out, bytes_in, bytes_out, engines);

        let idx = bin_index(latency_ms.max(0.0));
        self.latency_global_counts[idx] += 1;
        self.latency_global_total += 1;

        let bucket = self.bucket_for(ts);
        bucket.requests += 1;
        if is_error {
            bucket.errors += 1;
        }
        bucket.chars_saved_in += chars_saved_in;
        bucket.chars_saved_out += chars_saved_out;
        bucket.bytes_in += bytes_in;
        bucket.bytes_out += bytes_out;

        for (engine, &saved) in engines {
            if saved == 0 {
                continue;
            }
            *bucket.engines.entry(engine.clone()).or_insert(0) += saved;
        }
        bucket
            .by_provider
            .entry(provider.to_string())
            .or_default()
            .observe(is_error, chars_saved_in, chars_saved_out, bytes_in, bytes_out, engines);
        bucket.latency_counts[idx] += 1;
        bucket.latency_total += 1;

        let record = serde_json::json!({
            "ts": ts,
            "method": method,
            "path": path,
            "status_code": status_code,
            "ms": (latency_ms * 100.0).round() / 100.0,
            "chars_saved_in": chars_saved_in,
            "chars_saved_out": chars_saved_out,
            "bytes_in": bytes_in,
            "bytes_out": bytes_out,
            "engines": engines,
            "provider": provider,
            "request_id": request_id,
            "is_error": is_error,
            "model": model,
            "request_audit": sanitize_audit_summary(request_audit_summary),
            "response_audit": sanitize_audit_summary(response_audit_summary),
        });

        if self.recent.len() >= self.recent_max {
            self.recent.pop_front();
        }
        self.recent.push_back(record);

        self.evict(ts);
    }

    fn bucket_for(&mut self, ts: f64) -> &mut Bucket {
        let minute_ts = (ts as i64) - ((ts as i64) % 60);
        self.buckets.entry(minute_ts).or_insert_with(|| Bucket::new(minute_ts))
    }

    fn evict(&mut self, ts: f64) {
        let cutoff = (ts as i64) - ((self.window_minutes as i64) * 60);
        self.buckets.retain(|&k, _| k > cutoff);
    }
}

fn sanitize_audit_summary(summary: Option<Value>) -> Option<Value> {
    let mut sum = summary?;
    if let Some(obj) = sum.as_object_mut() {
        if let Some(events) = obj.get_mut("events").and_then(|e| e.as_array_mut()) {
            for ev in events {
                if let Some(ev_obj) = ev.as_object_mut() {
                    ev_obj.remove("sample_before");
                    ev_obj.remove("sample_after");
                }
            }
        }
    }
    Some(sum)
}

fn audit_to_engine_chars(audit: &CompressionAudit) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for ev in &audit.events {
        let mode = if ev.mode.is_empty() { "unknown".to_string() } else { ev.mode.clone() };
        *out.entry(mode).or_insert(0) += ev.chars_saved();
    }
    out
}

pub struct AuditLogger {
    pub stats: Mutex<ProxyStats>,
    log_path: Option<PathBuf>,
    log_json: bool,
}

impl AuditLogger {
    pub fn new(settings: &Settings) -> Self {
        let stats = ProxyStats::new(
            settings.timeseries_minutes as usize,
            settings.recent_max as usize,
        );

        let mut log_path = None;
        if settings.audit_enabled {
            let dir = PathBuf::from(&settings.audit_log_dir);
            let _ = create_dir_all(&dir);
            log_path = Some(dir.join("audit.jsonl"));
        }

        AuditLogger {
            stats: Mutex::new(stats),
            log_path,
            log_json: settings.log_json,
        }
    }

    pub fn record(
        &self,
        method: &str,
        path: &str,
        status_code: Option<u16>,
        request_audit: &CompressionAudit,
        response_audit: Option<&CompressionAudit>,
        request_id: Option<&str>,
        error: Option<&str>,
        latency_ms: Option<f64>,
        bytes_in: usize,
        bytes_out: usize,
        model: Option<&str>,
        now: Option<f64>,
    ) {
        let chars_saved_in = request_audit.chars_saved();
        let chars_saved_out = response_audit.map(|r| r.chars_saved()).unwrap_or(0);

        let mut engines = audit_to_engine_chars(request_audit);
        if let Some(resp) = response_audit {
            for (mode, saved) in audit_to_engine_chars(resp) {
                let key = if mode.ends_with("-response") { mode } else { format!("{}-response", mode) };
                *engines.entry(key).or_insert(0) += saved;
            }
        }

        let is_error = error.is_some() || status_code.map(|s| s >= 500).unwrap_or(false);
        let latency = latency_ms.unwrap_or(0.0);

        let request_summary = serde_json::to_value(request_audit).ok();
        let response_summary = response_audit.and_then(|r| serde_json::to_value(r).ok());

        {
            let mut stats = self.stats.lock();
            stats.cache_hits += request_audit.cache_hits;
            stats.cache_misses += request_audit.cache_misses;
            stats.protected_blocks += request_audit.protected_blocks;

            if let Some(resp) = response_audit {
                stats.cache_hits += resp.cache_hits;
                stats.cache_misses += resp.cache_misses;
            }

            stats.observe(
                method,
                path,
                status_code,
                chars_saved_in,
                chars_saved_out,
                &engines,
                latency,
                bytes_in,
                bytes_out,
                request_id,
                is_error,
                request_summary.clone(),
                response_summary.clone(),
                model,
                now,
            );
        }

        let ts = now.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        });

        if let Some(ref path_buf) = self.log_path {
            let entry = serde_json::json!({
                "ts": ts,
                "method": method,
                "path": path,
                "status_code": status_code,
                "request_id": request_id,
                "model": model,
                "ms": (latency * 100.0).round() / 100.0,
                "bytes_in": bytes_in,
                "bytes_out": bytes_out,
                "request_compression": request_summary,
                "response_compression": response_summary,
                "error": error,
            });

            if let Ok(line) = serde_json::to_string(&entry) {
                let line_nl = format!("{}\n", line);
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path_buf) {
                    let _ = file.write_all(line_nl.as_bytes());
                }
            }
        }

        if self.log_json {
            let formatted_time = chrono::Utc::now().to_rfc3339();
            let structured = serde_json::json!({
                "ts": formatted_time,
                "method": method,
                "path": path,
                "status": status_code,
                "ms": (latency * 100.0).round() / 100.0,
                "model": model,
                "chars_saved_input": chars_saved_in,
                "chars_saved_output": chars_saved_out,
                "engines_active": engines.keys().cloned().collect::<Vec<_>>(),
                "request_id": request_id,
            });
            if let Ok(s) = serde_json::to_string(&structured) {
                eprintln!("{}", s);
            }
        }
    }
}
