use std::sync::Arc;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use std::collections::HashMap;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use futures_util::Stream;
use serde_json::json;

use crate::audit::CompressionAudit as AuditCompressionAudit;
use crate::compression::{CompressRequestOptions, CompressionAudit as PayloadCompressionAudit};
use crate::server::ServerState;

const HOP_BY_HOP: &[&str] = &[
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailer", "transfer-encoding", "upgrade", "host",
    "content-length", "content-encoding", "accept-encoding",
];

struct OpenAIResponsesAccumulator {
    sse_buf: Vec<u8>,
    json_body: Vec<u8>,
    usage: std::collections::HashMap<String, i64>,
    model: Option<String>,
    saw_usage: bool,
    parse_sse: bool,
    /// Raw text of the most recent `data:` SSE event seen, truncated. Used to
    /// surface the upstream event shape when usage extraction fails on a 2xx.
    last_sse_event: Option<String>,
}

impl OpenAIResponsesAccumulator {
    fn new(parse_sse: bool) -> Self {
        OpenAIResponsesAccumulator {
            sse_buf: Vec::new(),
            json_body: Vec::new(),
            usage: crate::cost::extract_usage_from_openai_response(&serde_json::Value::Null),
            model: None,
            saw_usage: false,
            parse_sse,
            last_sse_event: None,
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if self.parse_sse {
            self.feed_sse(chunk);
        } else {
            self.json_body.extend_from_slice(chunk);
        }
    }

    fn finish(&mut self) {
        if self.parse_sse || self.json_body.is_empty() {
            return;
        }
        if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&self.json_body) {
            self.consume_response(&payload);
        }
    }

    fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn saw_usage(&self) -> bool {
        self.saw_usage
    }

    fn usage(&self) -> &std::collections::HashMap<String, i64> {
        &self.usage
    }

    fn feed_sse(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        self.sse_buf.extend_from_slice(chunk);
        while let Some(pos) = self.sse_buf.iter().position(|&b| b == b'\n') {
            let mut line_bytes = self.sse_buf[..pos].to_vec();
            self.sse_buf.drain(..=pos);

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
            // Capture the latest event so a 2xx-with-no-usage can report the shape.
            self.last_sse_event = Some(decoded.chars().take(800).collect());
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(decoded) {
                if let Some(response) = payload.get("response") {
                    self.consume_response(response);
                } else {
                    self.consume_response(&payload);
                }
            }
        }
    }

    fn consume_response(&mut self, payload: &serde_json::Value) {
        if let Some(model) = payload
            .get("model")
            .or_else(|| payload.get("response").and_then(|r| r.get("model")))
            .and_then(|v| v.as_str())
            .filter(|m| !m.is_empty())
        {
            self.model = Some(model.to_string());
        }

        let usage = crate::cost::extract_usage_from_openai_response(payload);
        let has_usage = usage.values().any(|v| *v > 0);
        if has_usage {
            self.saw_usage = true;
            for (key, val) in usage {
                if val > *self.usage.get(&key).unwrap_or(&0) {
                    self.usage.insert(key, val);
                }
            }
        }
    }
}

struct ResponsesLoggingStream<S> {
    inner_stream: S,
    acc: OpenAIResponsesAccumulator,
    state: Arc<ServerState>,
    request_audit: AuditCompressionAudit,
    request_model: Option<String>,
    bytes_in: usize,
    bytes_out: usize,
    started_perf: Instant,
    status_code: u16,
    request_id: Option<String>,
    logged: bool,
}

impl<S> Stream for ResponsesLoggingStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner_stream).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.bytes_out += chunk.len();
                self.acc.feed(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(e))) => {
                let error = e.to_string();
                self.record_final_stats(Some(error.clone()));
                Poll::Ready(Some(Err(std::io::Error::other(error))))
            }
            Poll::Ready(None) => {
                self.record_final_stats(None);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> ResponsesLoggingStream<S> {
    fn record_final_stats(&mut self, error: Option<String>) {
        if self.logged {
            return;
        }
        self.logged = true;
        self.acc.finish();

        // A successful streamed response that yields no usage means cost/token
        // stats silently drop for this request. Surface the event shape so the
        // mismatch can be fixed rather than failing invisibly.
        if !self.acc.saw_usage() && (200..300).contains(&self.status_code) {
            eprintln!(
                "[middleout] /v1/responses {} streamed but no usage parsed (parse_sse={}, bytes_out={}); last event: {}",
                self.status_code,
                self.acc.parse_sse,
                self.bytes_out,
                self.acc.last_sse_event.as_deref().unwrap_or("<none>")
            );
        }

        let final_model = self
            .acc
            .model()
            .map(|m| m.to_string())
            .or_else(|| self.request_model.clone());

        if self.acc.saw_usage() {
            let usage = self.acc.usage();
            let cost_record = crate::cost::estimate(
                "openai",
                final_model.as_deref().unwrap_or(""),
                *usage.get("input_tokens").unwrap_or(&0),
                *usage.get("output_tokens").unwrap_or(&0),
                *usage.get("cache_write_tokens").unwrap_or(&0),
                *usage.get("cache_read_tokens").unwrap_or(&0),
            );
            self.state.cost_tracker.record(&cost_record);
            self.state.usage_budget.record(
                self.bytes_in as i64,
                *usage.get("input_tokens").unwrap_or(&0)
                    + *usage.get("output_tokens").unwrap_or(&0),
            );
        }

        let latency_ms = self.started_perf.elapsed().as_secs_f64() * 1000.0;
        self.state.audit_logger.record(
            "POST",
            "/v1/responses",
            Some(self.status_code),
            &self.request_audit,
            None,
            self.request_id.as_deref(),
            error.as_deref(),
            Some(latency_ms),
            self.bytes_in,
            self.bytes_out,
            final_model.as_deref(),
            None,
        );
    }
}

fn request_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|payload| {
            payload
                .get("model")
                .and_then(|v| v.as_str())
                .filter(|m| !m.is_empty())
                .map(|m| m.to_string())
        })
}

fn is_streaming_request(payload: Option<&serde_json::Value>) -> bool {
    payload
        .and_then(|v| v.get("stream"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn forwarded_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (name, value) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_str.as_str()) {
            continue;
        }
        if let Ok(v) = value.to_str() {
            out.insert(name_str, v.to_string());
        }
    }
    out
}

fn openai_cache_key_context(headers: &HashMap<String, String>) -> serde_json::Value {
    let auth = headers.get("authorization").map(|s| s.as_str()).unwrap_or("");
    let mut ctx = crate::server::auth::cache_key_context(headers, auth);
    if let Some(obj) = ctx.as_object_mut() {
        for key in ["chatgpt-account-id", "openai-organization", "openai-project", "openai-beta"] {
            if let Some(value) = headers.get(key) {
                obj.insert(key.replace('-', "_"), json!(value));
            }
        }
    }
    ctx
}

fn map_compression_audit(src: &PayloadCompressionAudit) -> AuditCompressionAudit {
    let mut dest = AuditCompressionAudit::new(&src.endpoint);
    dest.cache_hits = src.cache_hits;
    dest.cache_misses = src.cache_misses;
    dest.protected_blocks = src.protected_blocks;
    for ev in &src.events {
        dest.events.push(crate::audit::CompressionEvent {
            path: ev.path.clone(),
            mode: ev.mode.clone(),
            original_chars: ev.original_chars,
            compressed_chars: ev.compressed_chars,
            sha256: ev.sha256.clone(),
            note: ev.note.clone(),
            sample_before: ev.sample_before.clone(),
            sample_after: ev.sample_after.clone(),
        });
    }
    dest
}

fn compression_headers(audit: &PayloadCompressionAudit, prefix: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        format!("x-middleout-{}-original-chars", prefix).parse::<axum::http::HeaderName>().unwrap(),
        audit.original_chars().to_string().parse().unwrap(),
    );
    h.insert(
        format!("x-middleout-{}-compressed-chars", prefix).parse::<axum::http::HeaderName>().unwrap(),
        audit.compressed_chars().to_string().parse().unwrap(),
    );
    h.insert(
        format!("x-middleout-{}-chars-saved", prefix).parse::<axum::http::HeaderName>().unwrap(),
        audit.chars_saved().to_string().parse().unwrap(),
    );
    h
}

fn response_with_headers(status_code: u16, headers: HeaderMap, body: Vec<u8>) -> Response {
    let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        if let Some(name) = name {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(axum::body::Body::from(body))
        .unwrap_or_default()
}

fn record_openai_cost(
    state: &ServerState,
    model: Option<&str>,
    usage: &HashMap<String, i64>,
    bytes_in: usize,
) {
    let cost_record = crate::cost::estimate(
        "openai",
        model.unwrap_or(""),
        *usage.get("input_tokens").unwrap_or(&0),
        *usage.get("output_tokens").unwrap_or(&0),
        *usage.get("cache_write_tokens").unwrap_or(&0),
        *usage.get("cache_read_tokens").unwrap_or(&0),
    );
    state.cost_tracker.record(&cost_record);
    state.usage_budget.record(
        bytes_in as i64,
        *usage.get("input_tokens").unwrap_or(&0)
            + *usage.get("output_tokens").unwrap_or(&0),
    );
}

/// Transparent reverse proxy for Codex's OpenAI Responses API.
///
/// Codex (ChatGPT-subscription auth) already produces a fully valid request for
/// the ChatGPT Codex backend — its OAuth Bearer, `chatgpt-account-id`, `OpenAI-Beta`,
/// and other headers are all present. We forward the request verbatim (headers + body)
/// to `{openai_upstream_url}/responses`, changing only the destination host. This is
/// the same request Codex would send natively; middleout is just in the path.
pub async fn responses_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let started_perf = Instant::now();
    let bytes_in = body.len();
    let mut request_model = request_model(&body);
    let request_headers = forwarded_headers(&headers);
    let rt = state.runtime_settings.read().await.clone();

    let input_compression = rt.get("input_compression").and_then(|v| v.as_bool()).unwrap_or(false);
    let output_compression = rt.get("output_compression").and_then(|v| v.as_bool()).unwrap_or(false);
    let l1_cache_enabled = rt.get("l1_cache").and_then(|v| v.as_bool()).unwrap_or(false);
    let l2_cache_enabled = rt.get("l2_cache").and_then(|v| v.as_bool()).unwrap_or(false);
    let jl_active = rt.get("jl_dedupe").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_text_chars_active = state.settings.max_text_chars;
    let caveman_cfg = rt.get("caveman").cloned().unwrap_or(json!({"enabled": false, "level": "standard"}));
    let rtk_cfg = rt.get("rtk").cloned().unwrap_or(json!({"enabled": false, "level": "minimal"}));
    let json_aware_cfg = rt.get("json_aware").cloned().unwrap_or(json!({"enabled": false, "level": "safe"}));
    let lsh_cfg = rt.get("lsh").cloned().unwrap_or(json!({"enabled": false, "level": "standard"}));

    let mut outgoing_body = body.to_vec();
    let mut request_audit = PayloadCompressionAudit::new("/v1/responses");
    let parsed_payload = serde_json::from_slice::<serde_json::Value>(&body).ok();
    let mut cache_lookup_payload = parsed_payload.clone();

    if let Some(payload) = parsed_payload.clone() {
        if let Some(model) = payload.get("model").and_then(|v| v.as_str()).filter(|m| !m.is_empty()) {
            request_model = Some(model.to_string());
        }

        if input_compression {
            let opts = CompressRequestOptions {
                jl_dedupe: Some(jl_active),
                caveman: Some(caveman_cfg),
                rtk: Some(rtk_cfg),
                json_aware: Some(json_aware_cfg),
                lsh: Some(lsh_cfg),
                max_text_chars: Some(max_text_chars_active),
                auto_insert_cache_wall: None,
            };
            let compressor = state.compressor.clone();
            if let Ok((transformed, audit)) = tokio::task::spawn_blocking(move || {
                compressor.compress_openai_responses_request_payload(
                    &payload,
                    "/v1/responses",
                    Some(opts),
                    true,
                )
            })
            .await
            .unwrap_or(Err("spawn_blocking panicked".to_string())) {
                request_audit = audit;
                if request_audit.touched() {
                    if let Ok(out) = serde_json::to_vec(&transformed) {
                        outgoing_body = out;
                    }
                }
                cache_lookup_payload = Some(transformed);
            }
        }
    }

    let stream_request = is_streaming_request(cache_lookup_payload.as_ref());
    let cacheable = cache_lookup_payload.is_some() && !stream_request;
    let cache_active = cacheable && l1_cache_enabled && state.l1_cache.is_some();
    let l2_active = cacheable
        && l2_cache_enabled
        && state.l2_cache.enabled.load(std::sync::atomic::Ordering::Relaxed);
    let key_ctx = openai_cache_key_context(&request_headers);
    let mut l1_status: Option<&str> = None;
    let mut l1_key: Option<String> = None;
    let mut l2_status: Option<&str> = None;

    if cache_active {
        if let (Some(l1), Some(payload)) = (state.l1_cache.as_ref(), cache_lookup_payload.as_ref()) {
            let key = crate::cache::normalize::cache_key(payload, Some(&key_ctx));
            l1_key = Some(key.clone());
            if let Some(cached) = l1.get(&key) {
                let mut response_headers = HeaderMap::new();
                for (k, v) in &cached.headers {
                    if let Ok(name) = axum::http::HeaderName::from_bytes(k.as_bytes()) {
                        if let Ok(val) = axum::http::HeaderValue::from_str(v) {
                            response_headers.insert(name, val);
                        }
                    }
                }
                response_headers.insert("x-brain-l1-cache", "hit".parse().unwrap());
                response_headers.insert("x-brain-l1-hit-count", cached.hit_count.to_string().parse().unwrap());
                response_headers.insert("x-brain-proxy", "middleout-proxy-rs/0.2.0".parse().unwrap());
                if !response_headers.contains_key(header::CONTENT_TYPE) {
                    let media_type = cached.media_type.clone().unwrap_or_else(|| "application/json".to_string());
                    if let Ok(value) = axum::http::HeaderValue::from_str(&media_type) {
                        response_headers.insert(header::CONTENT_TYPE, value);
                    }
                }

                let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
                let mapped_req_audit = map_compression_audit(&request_audit);
                state.audit_logger.record(
                    "POST",
                    "/v1/responses",
                    Some(cached.status_code),
                    &mapped_req_audit,
                    None,
                    response_headers
                        .get("request-id")
                        .or_else(|| response_headers.get("x-request-id"))
                        .and_then(|v| v.to_str().ok()),
                    None,
                    Some(latency_ms),
                    bytes_in,
                    cached.body.len(),
                    request_model.as_deref(),
                    None,
                );

                return response_with_headers(cached.status_code, response_headers, cached.body);
            }
            l1_status = Some("miss");
        }
    }

    if l2_active && (l1_status.is_none() || l1_status == Some("miss")) {
        if let Some(payload) = cache_lookup_payload.as_ref() {
            let embed_text = crate::cache::normalize::canonical_text(payload, Some(&key_ctx));
            if let Some(hit) = state.l2_cache.get_similar(&embed_text, None).await {
                let mut response_headers = HeaderMap::new();
                for (k, v) in &hit.response.headers {
                    if let Ok(name) = axum::http::HeaderName::from_bytes(k.as_bytes()) {
                        if let Ok(val) = axum::http::HeaderValue::from_str(v) {
                            response_headers.insert(name, val);
                        }
                    }
                }
                response_headers.insert("x-brain-l2-cache", "hit".parse().unwrap());
                response_headers.insert("x-brain-l2-similarity", format!("{:.4}", hit.similarity).parse().unwrap());
                if let Some(status) = l1_status {
                    response_headers.insert("x-brain-l1-cache", status.parse().unwrap());
                }
                response_headers.insert("x-brain-proxy", "middleout-proxy-rs/0.2.0".parse().unwrap());
                if !response_headers.contains_key(header::CONTENT_TYPE) {
                    let media_type = hit.response.media_type.clone().unwrap_or_else(|| "application/json".to_string());
                    if let Ok(value) = axum::http::HeaderValue::from_str(&media_type) {
                        response_headers.insert(header::CONTENT_TYPE, value);
                    }
                }

                let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
                let mapped_req_audit = map_compression_audit(&request_audit);
                state.audit_logger.record(
                    "POST",
                    "/v1/responses",
                    Some(hit.response.status_code),
                    &mapped_req_audit,
                    None,
                    response_headers
                        .get("request-id")
                        .or_else(|| response_headers.get("x-request-id"))
                        .and_then(|v| v.to_str().ok()),
                    None,
                    Some(latency_ms),
                    bytes_in,
                    hit.response.body.len(),
                    request_model.as_deref(),
                    None,
                );

                return response_with_headers(hit.response.status_code, response_headers, hit.response.body);
            }
            l2_status = Some("miss");
        }
    }

    let upstream = format!(
        "{}/responses",
        state.settings.openai_upstream_url.trim_end_matches('/')
    );

    let mut req = state.http_client.post(&upstream);
    for (name, value) in &request_headers {
        req = req.header(name, value);
    }
    req = req.body(outgoing_body);

    let upstream_res = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let mapped_req_audit = map_compression_audit(&request_audit);
            let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
            state.audit_logger.record(
                "POST",
                "/v1/responses",
                Some(StatusCode::BAD_GATEWAY.as_u16()),
                &mapped_req_audit,
                None,
                None,
                Some(&e.to_string()),
                Some(latency_ms),
                bytes_in,
                0,
                request_model.as_deref(),
                None,
            );
            return (StatusCode::BAD_GATEWAY, Json(json!({
                "error": {
                    "message": format!("middleout could not reach Codex upstream: {}", e),
                    "type": "api_error"
                }
            }))).into_response();
        }
    };

    let status = upstream_res.status();
    let upstream_headers = upstream_res.headers().clone();
    let request_id = upstream_headers
        .get("request-id")
        .or_else(|| upstream_headers.get("x-request-id"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let parse_sse = upstream_headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false);

    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream_headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if HOP_BY_HOP.contains(&name_str.as_str()) {
            continue;
        }
        response_headers.insert(name.clone(), value.clone());
    }
    for (name, value) in compression_headers(&request_audit, "input") {
        if let Some(name) = name {
            response_headers.insert(name, value);
        }
    }
    response_headers.insert("x-brain-proxy", "middleout-proxy-rs/0.2.0".parse().unwrap());
    if let Some(status) = l1_status {
        response_headers.insert("x-brain-l1-cache", status.parse().unwrap());
    }
    if let Some(status) = l2_status {
        response_headers.insert("x-brain-l2-cache", status.parse().unwrap());
    }

    if parse_sse {
        let mut builder = Response::builder().status(status);
        for (name, value) in &response_headers {
            builder = builder.header(name, value);
        }
        let stream = upstream_res.bytes_stream();
        let logging_stream = ResponsesLoggingStream {
            inner_stream: stream,
            acc: OpenAIResponsesAccumulator::new(true),
            state,
            request_audit: map_compression_audit(&request_audit),
            request_model,
            bytes_in,
            bytes_out: 0,
            started_perf,
            status_code: status.as_u16(),
            request_id,
            logged: false,
        };

        return builder
            .body(axum::body::Body::from_stream(logging_stream))
            .unwrap_or_else(|_| {
                (StatusCode::INTERNAL_SERVER_ERROR, "response build failed").into_response()
            });
    }

    let res_bytes = match upstream_res.bytes().await {
        Ok(bytes) => bytes.to_vec(),
        Err(e) => {
            let mapped_req_audit = map_compression_audit(&request_audit);
            let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
            state.audit_logger.record(
                "POST",
                "/v1/responses",
                Some(status.as_u16()),
                &mapped_req_audit,
                None,
                request_id.as_deref(),
                Some(&e.to_string()),
                Some(latency_ms),
                bytes_in,
                0,
                request_model.as_deref(),
                None,
            );
            return (StatusCode::BAD_GATEWAY, Json(json!({
                "error": {
                    "message": format!("middleout could not read Codex upstream body: {}", e),
                    "type": "api_error"
                }
            }))).into_response();
        }
    };

    let mut final_res_bytes = res_bytes.clone();
    let mut response_audit: Option<PayloadCompressionAudit> = None;
    let mut final_model = request_model.clone();

    if status.is_success() {
        if let Ok(res_json) = serde_json::from_slice::<serde_json::Value>(&res_bytes) {
            if let Some(model) = res_json.get("model").and_then(|v| v.as_str()).filter(|m| !m.is_empty()) {
                final_model = Some(model.to_string());
            }

            let usage = crate::cost::extract_usage_from_openai_response(&res_json);
            if usage.values().any(|v| *v > 0) {
                record_openai_cost(&state, final_model.as_deref(), &usage, bytes_in);
            }

            if output_compression {
                let (transformed, audit) = state
                    .compressor
                    .compress_openai_responses_response_payload(&res_json, "/v1/responses", true);
                response_audit = Some(audit);
                if response_audit.as_ref().unwrap().touched() {
                    if let Ok(out) = serde_json::to_vec(&transformed) {
                        final_res_bytes = out;
                        for (name, value) in compression_headers(response_audit.as_ref().unwrap(), "output") {
                            if let Some(name) = name {
                                response_headers.insert(name, value);
                            }
                        }
                        response_headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
                    }
                }
            }
        }
    }

    let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
    let mapped_req_audit = map_compression_audit(&request_audit);
    let mapped_res_audit = response_audit.as_ref().map(map_compression_audit);
    state.audit_logger.record(
        "POST",
        "/v1/responses",
        Some(status.as_u16()),
        &mapped_req_audit,
        mapped_res_audit.as_ref(),
        request_id.as_deref(),
        None,
        Some(latency_ms),
        bytes_in,
        final_res_bytes.len(),
        final_model.as_deref(),
        None,
    );

    if (cache_active || l2_active) && status.is_success() && cache_lookup_payload.is_some() {
        let cached_resp = crate::cache::l1::CachedResponse::new(
            status.as_u16(),
            {
                let mut map = HashMap::new();
                for (k, v) in &response_headers {
                    map.insert(k.as_str().to_string(), v.to_str().unwrap_or("").to_string());
                }
                map
            },
            final_res_bytes.clone(),
            upstream_headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok().map(|s| s.to_string())),
        );

        if cache_active {
            if let (Some(l1), Some(key)) = (state.l1_cache.as_ref(), l1_key.as_ref()) {
                l1.put(key, &cached_resp);
            }
        }

        if l2_active {
            if let Some(payload) = cache_lookup_payload.as_ref() {
                let point_id = l1_key
                    .clone()
                    .unwrap_or_else(|| crate::cache::normalize::cache_key(payload, Some(&key_ctx)));
                let embed_text = crate::cache::normalize::canonical_text(payload, Some(&key_ctx));
                state.l2_cache.put_similar(&embed_text, &cached_resp, &point_id).await;
            }
        }
    }

    response_with_headers(status.as_u16(), response_headers, final_res_bytes)
}
