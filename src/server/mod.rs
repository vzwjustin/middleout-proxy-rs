pub mod auth;
pub mod policies;
pub mod streaming;
pub mod dashboard;
pub mod metrics;
pub mod preview;
pub mod responses;

use std::sync::Arc;
use tokio::sync::RwLock;
use axum::{
    Router,
    routing::{get, post},
    response::{Response, IntoResponse, Html},
    extract::{State, Query, Request},
    http::{StatusCode, HeaderMap},
    Json,
};

use crate::config::Settings;
use crate::compression::PayloadCompressor;
use crate::cache::l1::L1Cache;
use crate::cache::l2::L2Cache;
use crate::rate_limit::RequestLimiter;
use crate::cost::{CostTracker, UsageBudget};
use crate::audit::AuditLogger;
use crate::server::policies::PolicyRouter;

pub struct ServerState {
    pub settings: Settings,
    pub compressor: PayloadCompressor,
    pub l1_cache: Option<L1Cache>,
    pub l2_cache: L2Cache,
    pub rate_limiter: RequestLimiter,
    pub cost_tracker: Arc<CostTracker>,
    pub usage_budget: UsageBudget,
    pub audit_logger: Arc<AuditLogger>,
    pub policy_router: PolicyRouter,
    pub runtime_settings: RwLock<serde_json::Value>,
    pub http_client: reqwest::Client,
}

pub fn default_runtime_settings(settings: &Settings) -> serde_json::Value {
    serde_json::json!({
        "input_compression": settings.input_compression_enabled,
        "output_compression": settings.output_compression_enabled,
        "jl_dedupe": false,
        "auto_insert_wall": settings.auto_insert_cache_wall,
        "l1_cache": settings.l1_cache_enabled,
        "l2_cache": settings.l2_cache_enabled,
        "rate_limit": settings.rate_limit_enabled,
        "adaptive": settings.adaptive_enabled,
        "caveman": {
            "enabled": settings.caveman_enabled,
            "level": settings.caveman_level,
        },
        "rtk": {
            "enabled": settings.rtk_enabled,
            "level": settings.rtk_level,
        },
        "json_aware": {
            "enabled": settings.json_aware_enabled,
            "level": settings.json_aware_level,
        },
        "lsh": {
            "enabled": settings.lsh_enabled,
            "level": settings.lsh_level,
        },
        "lingua": {
            "enabled": false,
            "ratio": 0.0,
        }
    })
}

fn l2_cache_available(state: &ServerState) -> bool {
    state.l2_cache.vector_store.is_some() && state.l2_cache.embedding_client.is_some()
}

fn effective_runtime_settings(state: &ServerState, rt: &serde_json::Value) -> serde_json::Value {
    let mut effective = rt.clone();
    if state.l1_cache.is_none() {
        effective["l1_cache"] = serde_json::json!(false);
    }
    if !l2_cache_available(state) {
        effective["l2_cache"] = serde_json::json!(false);
    }
    effective
}

fn should_transform_json_request(path: &str, method: &str, headers: &HeaderMap) -> bool {
    let method = method.to_uppercase();
    if method != "POST" {
        return false;
    }
    let p = path.trim_start_matches('/').trim_end_matches('/');
    if p != "v1/messages" {
        return false;
    }
    if let Some(ct) = headers.get("content-type") {
        if let Ok(ct_str) = ct.to_str() {
            return ct_str.to_lowercase().contains("application/json");
        }
    }
    true
}

fn should_transform_json_response(path: &str, headers: &HeaderMap) -> bool {
    let p = path.trim_start_matches('/').trim_end_matches('/');
    if p != "v1/messages" {
        return false;
    }
    if let Some(ct) = headers.get("content-type") {
        if let Ok(ct_str) = ct.to_str() {
            return ct_str.to_lowercase().contains("application/json");
        }
    }
    false
}

fn is_streaming_messages(path: &str, payload: &serde_json::Value) -> bool {
    let p = path.trim_start_matches('/').trim_end_matches('/');
    if p != "v1/messages" {
        return false;
    }
    payload.get("stream").and_then(|v| v.as_bool()).unwrap_or(false)
}

fn brain_headers(lingua_chars_saved: usize, wall_auto_inserted: bool) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("x-brain-proxy", "middleout-proxy-rs/0.2.0".parse().unwrap());
    if lingua_chars_saved > 0 {
        h.insert("x-brain-lingua-chars-saved", lingua_chars_saved.to_string().parse().unwrap());
    }
    if wall_auto_inserted {
        h.insert("x-brain-cache-wall-auto", "true".parse().unwrap());
    }
    h
}

fn compression_headers(audit: &crate::compression::CompressionAudit, prefix: &str) -> HeaderMap {
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

fn map_compression_audit(src: &crate::compression::CompressionAudit) -> crate::audit::CompressionAudit {
    let mut dest = crate::audit::CompressionAudit::new(&src.endpoint);
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

fn map_response_audit(src: &crate::compression::CompressionAudit) -> crate::audit::CompressionAudit {
    map_compression_audit(src)
}

// Route Handlers

async fn healthz_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let rt = state.runtime_settings.read().await;
    let auth_mode = state.settings.auth_mode.clone();

    let body = serde_json::json!({
        "ok": true,
        "upstream": state.settings.upstream_base_url,
        "input_compression": rt.get("input_compression").and_then(|v| v.as_bool()).unwrap_or(false),
        "jl_dedupe": rt.get("jl_dedupe").and_then(|v| v.as_bool()).unwrap_or(false),
        "output_compression": rt.get("output_compression").and_then(|v| v.as_bool()).unwrap_or(false),
        "caveman_enabled": rt.get("caveman").and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false),
        "rtk_enabled": rt.get("rtk").and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false),
        "json_aware_enabled": rt.get("json_aware").and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false),
        "json_aware_level": rt.get("json_aware").and_then(|v| v.get("level")).and_then(|v| v.as_str()).unwrap_or("safe"),
        "lsh_enabled": rt.get("lsh").and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false),
        "lsh_level": rt.get("lsh").and_then(|v| v.get("level")).and_then(|v| v.as_str()).unwrap_or("standard"),
        "adaptive_enabled": rt.get("adaptive").and_then(|v| v.as_bool()).unwrap_or(false),
        "lingua_enabled": false,
        "lingua_ratio": 0.0,
        "lingua_model_loaded": false,
        "auto_insert_cache_wall": rt.get("auto_insert_wall").and_then(|v| v.as_bool()).unwrap_or(false),
        "l1_cache_enabled": rt.get("l1_cache").and_then(|v| v.as_bool()).unwrap_or(false) && state.l1_cache.is_some(),
        "l1_cache_backend": state.settings.l1_cache_db_path,
        "l2_cache_enabled": state.l2_cache.enabled.load(std::sync::atomic::Ordering::Relaxed),
        "l2_cache_misconfigured": false,
        "l2_cache_backend": if state.l2_cache.enabled.load(std::sync::atomic::Ordering::Relaxed) { Some(state.settings.l2_backend.clone()) } else { None },
        "l2_embedder": if state.l2_cache.enabled.load(std::sync::atomic::Ordering::Relaxed) { Some(state.settings.l2_embedder.clone()) } else { None },
        "l2_similarity_threshold": state.settings.l2_similarity_threshold,
        "l2_init_error": serde_json::Value::Null,
        "preserve_anthropic_cache": state.settings.preserve_anthropic_cache,
        "compression_cache_enabled": state.settings.compression_cache_enabled,
        "auth_mode": auth_mode,
        "api_key_injection": false,
        "api_key_headers_rejected": true,
        "api_keys_supported": false,
        "providers": vec!["anthropic", "openai"],
        "rate_limit_enabled": rt.get("rate_limit").and_then(|v| v.as_bool()).unwrap_or(false),
        "rate_limit_capacity": state.rate_limiter.stats().get("capacity").and_then(|v| v.as_i64()).unwrap_or(0),
        "rate_limit_refill_per_second": state.rate_limiter.stats().get("refill_per_second").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "phase": "1-cache-aware-compression + 2a-l1-cache + 2b-l2-stub + 3-provider-scaffold + 4-rate-limit+policies (Rust)",
    });

    Json(body)
}

async fn stats_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut snap = state.audit_logger.stats.lock().snapshot();
    snap.as_object_mut().unwrap().insert("result_cache".to_string(), state.compressor.result_cache.stats());
    snap.as_object_mut().unwrap().insert("preserve_anthropic_cache".to_string(), serde_json::json!(state.settings.preserve_anthropic_cache));
    if let Some(ref l1) = state.l1_cache {
        snap.as_object_mut().unwrap().insert("l1_cache".to_string(), l1.stats());
    }
    if state.l2_cache.enabled.load(std::sync::atomic::Ordering::Relaxed) {
        snap.as_object_mut().unwrap().insert("l2_cache".to_string(), state.l2_cache.stats().await);
        if let Some(ref store) = state.l2_cache.vector_store {
            snap.as_object_mut().unwrap().insert("l2_vector_store".to_string(), store.stats().await);
        }
    }

    let recent = state.audit_logger.stats.lock().recent_records(50);
    snap.as_object_mut().unwrap().insert("recent".to_string(), serde_json::json!(recent));

    Json(snap)
}

async fn stats_timeseries_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut stats = state.audit_logger.stats.lock();
    let buckets = stats.timeseries(None);
    Json(serde_json::json!({
        "window_minutes": state.settings.timeseries_minutes,
        "buckets": buckets,
    }))
}

#[derive(serde::Deserialize)]
struct RecentQuery {
    n: Option<usize>,
}

async fn stats_recent_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<RecentQuery>,
) -> impl IntoResponse {
    let n = query.n.unwrap_or(50).max(0).min(500);
    let recent = state.audit_logger.stats.lock().recent_records(n);
    Json(serde_json::json!({
        "count": recent.len(),
        "items": recent,
    }))
}

async fn get_settings_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let rt = state.runtime_settings.read().await;
    Json(effective_runtime_settings(&state, &rt))
}

async fn post_settings_handler(
    State(state): State<Arc<ServerState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !body.is_object() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "body must be a JSON object"}))).into_response();
    }

    let mut rt = state.runtime_settings.write().await;
    let body_obj = body.as_object().unwrap();

    if body_obj.get("l1_cache").and_then(|v| v.as_bool()) == Some(true) && state.l1_cache.is_none() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "l1_cache requested but cache backend is not initialized; enable BRAIN_L1_CACHE_ENABLED before startup"
        }))).into_response();
    }
    if body_obj.get("l2_cache").and_then(|v| v.as_bool()) == Some(true) && !l2_cache_available(&state) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": "l2_cache requested but cache backend is not initialized; configure an embedder and vector store before startup"
        }))).into_response();
    }

    for k in &["input_compression", "output_compression", "jl_dedupe", "auto_insert_wall", "l1_cache", "l2_cache", "adaptive", "rate_limit"] {
        if let Some(v) = body_obj.get(*k) {
            if let Some(b) = v.as_bool() {
                rt[*k] = serde_json::json!(b);
            }
        }
    }

    for engine_key in &["caveman", "rtk", "json_aware", "lsh"] {
        if let Some(incoming) = body_obj.get(*engine_key).and_then(|v| v.as_object()) {
            if let Some(enabled) = incoming.get("enabled").and_then(|e| e.as_bool()) {
                rt[*engine_key]["enabled"] = serde_json::json!(enabled);
            }
            if let Some(level) = incoming.get("level").and_then(|l| l.as_str()) {
                let valid_levels: &[&str] = match *engine_key {
                    "caveman" => &["lite", "standard", "aggressive", "ultra"],
                    "rtk" => &["minimal", "standard", "aggressive"],
                    "json_aware" => &["safe", "standard", "aggressive"],
                    "lsh" => &["conservative", "standard", "aggressive"],
                    _ => &[],
                };
                if !valid_levels.contains(&level) {
                    return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                        "error": format!("{} level must be one of {:?}, got {:?}", engine_key, valid_levels, level)
                    }))).into_response();
                }
                rt[*engine_key]["level"] = serde_json::json!(level);
            }
        }
    }

    if let Some(lingua) = body_obj.get("lingua").and_then(|v| v.as_object()) {
        if let Some(enabled) = lingua.get("enabled").and_then(|e| e.as_bool()) {
            rt["lingua"]["enabled"] = serde_json::json!(enabled);
        }
        if let Some(ratio) = lingua.get("ratio").and_then(|r| r.as_f64()) {
            if ratio < 0.0 || ratio > 1.0 {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                    "error": format!("lingua.ratio must be in [0.0, 1.0], got {}", ratio)
                }))).into_response();
            }
            rt["lingua"]["ratio"] = serde_json::json!(ratio);
        }
    }

    if state.l1_cache.is_none() {
        rt["l1_cache"] = serde_json::json!(false);
    }
    if l2_cache_available(&state) {
        let l2_enabled = rt.get("l2_cache").and_then(|v| v.as_bool()).unwrap_or(false);
        state.l2_cache.enabled.store(l2_enabled, std::sync::atomic::Ordering::Relaxed);
    } else {
        rt["l2_cache"] = serde_json::json!(false);
        state.l2_cache.enabled.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    Json(effective_runtime_settings(&state, &rt)).into_response()
}

async fn dashboard_handler() -> impl IntoResponse {
    Html(crate::server::dashboard::get_dashboard_html())
}

async fn cost_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut snap = state.cost_tracker.snapshot();
    snap.as_object_mut().unwrap().insert("budget".to_string(), state.usage_budget.snapshot());
    Json(snap)
}

async fn cost_reset_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    state.cost_tracker.reset();
    state.usage_budget.reset();
    Json(serde_json::json!({
        "reset": true,
        "total_usd": 0.0,
        "budget_reset": true,
    }))
}

async fn rate_limit_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let rt = state.runtime_settings.read().await;
    let mut snap = state.rate_limiter.stats();
    snap.as_object_mut().unwrap().insert("enabled".to_string(), rt.get("rate_limit").cloned().unwrap_or(serde_json::json!(false)));
    snap.as_object_mut().unwrap().insert("policies_rules".to_string(), serde_json::json!(state.policy_router.rules.len()));
    snap.as_object_mut().unwrap().insert("policies_default_is_vanilla".to_string(), serde_json::json!(state.policy_router.default == crate::server::policies::CompressionPolicy::default()));
    Json(snap)
}

async fn policies_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let rules = &state.policy_router.rules;
    let default = &state.policy_router.default;
    Json(serde_json::json!({
        "rules": rules,
        "default": default,
    }))
}

async fn cache_purge_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut l1_cleared = 0;
    if let Some(ref l1) = state.l1_cache {
        l1_cleared = l1.purge();
    }
    let l2_cleared = state.l2_cache.clear().await;
    Json(serde_json::json!({
        "l1_cleared": l1_cleared,
        "l2_cleared": l2_cleared,
    }))
}

async fn metrics_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let mut snap = state.audit_logger.stats.lock().snapshot();
    snap.as_object_mut().unwrap().insert("result_cache".to_string(), state.compressor.result_cache.stats());
    let body = crate::server::metrics::render_prometheus(&snap, &state.settings);
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

async fn preview_handler(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !payload.is_object() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "body must be a JSON object"}))).into_response();
    }
    let rt = state.runtime_settings.read().await;
    let jl_dedupe = rt.get("jl_dedupe").and_then(|v| v.as_bool()).unwrap_or(state.settings.jl_dedupe_enabled);
    let caveman = rt.get("caveman").cloned();
    let rtk = rt.get("rtk").cloned();
    drop(rt);
    let result = crate::server::preview::preview_compression(&payload, &state.settings, jl_dedupe, caveman, rtk);
    Json(result).into_response()
}

async fn providers_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "adapters": ["anthropic", "openai"],
        "routes": [
            {"provider": "anthropic", "path": "/v1/messages"},
            {"provider": "openai", "path": "/v1/responses"}
        ],
    }))
}

async fn budget_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    Json(state.usage_budget.snapshot())
}

async fn budget_reset_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    state.usage_budget.reset();
    Json(serde_json::json!({
        "reset": true,
        "budget": state.usage_budget.snapshot(),
    }))
}

async fn cache_stats_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let l1_enabled = {
        let rt = state.runtime_settings.read().await;
        rt.get("l1_cache").and_then(|v| v.as_bool()).unwrap_or(false)
    };
    let mut l1_stats = serde_json::json!({"enabled": false});
    if let Some(ref l1) = state.l1_cache {
        if l1_enabled {
            l1_stats = l1.stats();
            l1_stats.as_object_mut().unwrap().insert("enabled".to_string(), serde_json::json!(true));
        }
    }
    let l2_stats = state.l2_cache.stats().await;
    Json(serde_json::json!({
        "l1": l1_stats,
        "l2": l2_stats,
        "l2_misconfigured": false,
    }))
}

// Catch-All Router

async fn proxy_handler(
    State(state): State<Arc<ServerState>>,
    request: Request,
) -> impl IntoResponse {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().unwrap_or("").to_string();

    if path == "" || path == "/" {
        return (StatusCode::OK, Json(serde_json::json!({
            "name": "middleout-claude-proxy",
            "health": "/healthz",
            "stats": "/stats",
            "anthropic_messages": "/v1/messages",
        }))).into_response();
    }

    // Capture headers before consuming `request` with into_body()
    let incoming_headers = request.headers().clone();
    let authorization_header_val = incoming_headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let request_headers: std::collections::HashMap<String, String> = match crate::server::auth::forward_request_headers(&incoming_headers, &state.settings) {
        Ok(h) => h,
        Err(err) => {
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "strict_subscription_auth_error",
                    "message": err.0,
                }
            }))).into_response();
        }
    };

    let mut upstream_url = format!("{}/{}", state.settings.upstream_base_url.trim_end_matches('/'), path.trim_start_matches('/'));
    if !query.is_empty() {
        upstream_url = format!("{}?{}", upstream_url, query);
    }

    let rt = state.runtime_settings.read().await.clone();
    let rate_limit_enabled = rt.get("rate_limit").and_then(|v| v.as_bool()).unwrap_or(false);
    if rate_limit_enabled {
        let client_key = crate::server::auth::rate_limit_key(&request_headers, &authorization_header_val);
        if !state.rate_limiter.check(&client_key) {
            return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "rate_limit_error",
                    "message": "MiddleOut proxy rate limit exceeded for this client.",
                }
            }))).into_response();
        }
    }

    let body_bytes = match axum::body::to_bytes(request.into_body(), 100 * 1024 * 1024).await {
        Ok(b) => b.to_vec(),
        Err(_) => return (StatusCode::BAD_REQUEST, "Failed to read request body").into_response(),
    };

    let started_perf = std::time::Instant::now();
    let bytes_in = body_bytes.len();

    let mut request_audit = crate::compression::CompressionAudit::new(&path);
    let mut outgoing_content = body_bytes.clone();
    let mut cache_lookup_payload: Option<serde_json::Value> = None;
    let mut request_model: Option<String> = None;

    let input_compression = rt.get("input_compression").and_then(|v| v.as_bool()).unwrap_or(false);
    let l1_cache_enabled = rt.get("l1_cache").and_then(|v| v.as_bool()).unwrap_or(false);
    let l2_cache_enabled = rt.get("l2_cache").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut skip_compression = false;
    let mut jl_active = rt.get("jl_dedupe").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut max_text_chars_active = state.settings.max_text_chars;
    let mut caveman_cfg = rt.get("caveman").cloned().unwrap_or(serde_json::json!({"enabled": false, "level": "standard"}));
    let mut rtk_cfg = rt.get("rtk").cloned().unwrap_or(serde_json::json!({"enabled": false, "level": "minimal"}));
    let json_aware_cfg = rt.get("json_aware").cloned().unwrap_or(serde_json::json!({"enabled": false, "level": "safe"}));
    let lsh_cfg = rt.get("lsh").cloned().unwrap_or(serde_json::json!({"enabled": false, "level": "standard"}));
    let auto_insert_cache_wall = rt.get("auto_insert_wall").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut output_compression_policy: Option<bool> = None;

    let mut is_json = false;
    if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
        is_json = true;
        if payload.is_object() {
            if let Some(m) = payload.get("model").and_then(|v| v.as_str()) {
                request_model = Some(m.to_string());
            }
        }

        let rules_active = !state.policy_router.rules.is_empty() || state.policy_router.default != crate::server::policies::CompressionPolicy::default();
        if rules_active {
            let active_policy = state.policy_router.resolve(request_model.as_deref(), &path);
            output_compression_policy = Some(active_policy.output_compression);
            if !active_policy.input_compression {
                skip_compression = true;
            } else {
                jl_active = active_policy.jl_dedupe;
                if let Some(limit) = active_policy.max_text_chars {
                    max_text_chars_active = limit;
                }
                caveman_cfg = serde_json::json!({
                    "enabled": active_policy.caveman_enabled,
                    "level": active_policy.caveman_level,
                });
                rtk_cfg = serde_json::json!({
                    "enabled": active_policy.rtk_enabled,
                    "level": active_policy.rtk_level,
                });
            }
        }

        if input_compression && should_transform_json_request(&path, method.as_str(), &incoming_headers) && !skip_compression {
            let opts = crate::compression::CompressRequestOptions {
                jl_dedupe: Some(jl_active),
                caveman: Some(caveman_cfg),
                rtk: Some(rtk_cfg),
                json_aware: Some(json_aware_cfg),
                lsh: Some(lsh_cfg),
                max_text_chars: Some(max_text_chars_active),
                auto_insert_cache_wall: Some(auto_insert_cache_wall),
            };

            let compressor = state.compressor.clone();
            let payload_for_compress = payload.clone();
            let path_for_compress = path.clone();
            match tokio::task::spawn_blocking(move || {
                compressor.compress_request_payload(&payload_for_compress, &path_for_compress, Some(opts), true)
            }).await.unwrap_or(Err("spawn_blocking panicked".to_string())) {
                Ok((transformed, audit)) => {
                    request_audit = audit;
                    if request_audit.touched() {
                        if let Ok(out) = serde_json::to_vec(&transformed) {
                            outgoing_content = out;
                        }
                    }
                    cache_lookup_payload = Some(transformed);
                }
                Err(_) => {
                    cache_lookup_payload = Some(payload);
                }
            }
        } else {
            cache_lookup_payload = Some(payload);
        }
    }

    let mut l1_status: Option<&str> = None;
    let mut l1_key: Option<String> = None;
    let mut l2_status: Option<&str> = None;
    let mut _l2_similarity: Option<f64> = None;

    let cacheable = cache_lookup_payload.is_some()
        && method == "POST"
        && path.trim_start_matches('/').trim_end_matches('/') == "v1/messages"
        && !cache_lookup_payload.as_ref().unwrap().get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let cache_active = cacheable && l1_cache_enabled && state.l1_cache.is_some();
    let l2_active = cacheable && l2_cache_enabled && state.l2_cache.enabled.load(std::sync::atomic::Ordering::Relaxed);

    let key_headers = {
        let mut map = std::collections::HashMap::new();
        if let Some(v) = request_headers.get("anthropic-version") {
            map.insert("anthropic-version".to_string(), v.clone());
        }
        if let Some(b) = request_headers.get("anthropic-beta") {
            map.insert("anthropic-beta".to_string(), b.clone());
        }
        map
    };

    let key_ctx = crate::server::auth::cache_key_context(&key_headers, &authorization_header_val);

    if cache_active {
        if let Some(ref l1) = state.l1_cache {
            if let Some(ref payload_val) = cache_lookup_payload {
                let key = crate::cache::normalize::cache_key(payload_val, Some(&key_ctx));
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

                    let bh = brain_headers(0, false);
                    for (k, v) in bh {
                        if let Some(name) = k {
                            response_headers.insert(name, v);
                        }
                    }

                    if !response_headers.contains_key(axum::http::header::CONTENT_TYPE) {
                        let media_type = cached.media_type.clone().unwrap_or_else(|| "application/json".to_string());
                        if let Ok(value) = axum::http::HeaderValue::from_str(&media_type) {
                            response_headers.insert(axum::http::header::CONTENT_TYPE, value);
                        }
                    }

                    let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
                    let mapped_req_audit = map_compression_audit(&request_audit);
                    state.audit_logger.record(
                        method.as_str(),
                        &path,
                        Some(cached.status_code),
                        &mapped_req_audit,
                        None,
                        response_headers.get("request-id").and_then(|v| v.to_str().ok()),
                        None,
                        Some(latency_ms),
                        bytes_in,
                        cached.body.len(),
                        request_model.as_deref(),
                        None,
                    );

                    return response_with_headers(cached.status_code, response_headers, cached.body).into_response();
                }
                l1_status = Some("miss");
            }
        }
    }

    if l2_active && (l1_status.is_none() || l1_status == Some("miss")) {
        if let Some(ref payload_val) = cache_lookup_payload {
            let embed_text = crate::cache::normalize::canonical_text(payload_val, Some(&key_ctx));
            if let Some(hit) = state.l2_cache.get_similar(&embed_text, None).await {
                _l2_similarity = Some(hit.similarity);

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

                let bh = brain_headers(0, false);
                for (k, v) in bh {
                    if let Some(name) = k {
                        response_headers.insert(name, v);
                    }
                }

                if !response_headers.contains_key(axum::http::header::CONTENT_TYPE) {
                    let media_type = hit.response.media_type.clone().unwrap_or_else(|| "application/json".to_string());
                    if let Ok(value) = axum::http::HeaderValue::from_str(&media_type) {
                        response_headers.insert(axum::http::header::CONTENT_TYPE, value);
                    }
                }

                let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
                let mapped_req_audit = map_compression_audit(&request_audit);
                state.audit_logger.record(
                    method.as_str(),
                    &path,
                    Some(hit.response.status_code),
                    &mapped_req_audit,
                    None,
                    response_headers.get("request-id").and_then(|v| v.to_str().ok()),
                    None,
                    Some(latency_ms),
                    bytes_in,
                    hit.response.body.len(),
                    request_model.as_deref(),
                    None,
                );

                return response_with_headers(hit.response.status_code, response_headers, hit.response.body).into_response();
            }
            l2_status = Some("miss");
        }
    }

    let is_stream = is_json && is_streaming_messages(&path, cache_lookup_payload.as_ref().unwrap());

    let mut req_builder = state.http_client.request(method.clone(), &upstream_url);
    for (k, v) in &request_headers {
        req_builder = req_builder.header(k, v);
    }
    req_builder = req_builder.body(outgoing_content.clone());

    let upstream_res = match req_builder.send().await {
        Ok(r) => r,
        Err(err) => {
            let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
            let mapped_req_audit = map_compression_audit(&request_audit);
            state.audit_logger.record(
                method.as_str(),
                &path,
                None,
                &mapped_req_audit,
                None,
                None,
                Some(&err.to_string()),
                Some(latency_ms),
                bytes_in,
                0,
                request_model.as_deref(),
                None,
            );
            return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "proxy_upstream_error",
                    "message": format!("MiddleOut proxy could not reach upstream: {}", err),
                }
            }))).into_response();
        }
    };

    if is_stream {
        let sse_acc = Some(crate::cost::SSEUsageAccumulator::new());
        let mapped_req_audit = map_compression_audit(&request_audit);
        return crate::server::streaming::stream_forward(
            upstream_res,
            sse_acc,
            mapped_req_audit,
            path,
            method.to_string(),
            request_model,
            bytes_in,
            started_perf,
            state.audit_logger.clone(),
            state.cost_tracker.clone(),
        ).await.into_response();
    }

    let status = upstream_res.status();
    let upstream_headers = upstream_res.headers().clone();
    let res_bytes = match upstream_res.bytes().await {
        Ok(bytes) => bytes.to_vec(),
        Err(err) => {
            let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
            let mapped_req_audit = map_compression_audit(&request_audit);
            state.audit_logger.record(
                method.as_str(),
                &path,
                Some(status.as_u16()),
                &mapped_req_audit,
                None,
                upstream_headers.get("request-id").and_then(|v| v.to_str().ok()),
                Some(&err.to_string()),
                Some(latency_ms),
                bytes_in,
                0,
                request_model.as_deref(),
                None,
            );
            return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({
                "type": "error",
                "error": {
                    "type": "proxy_upstream_body_error",
                    "message": format!("MiddleOut proxy could not read upstream response body: {}", err),
                }
            }))).into_response();
        }
    };

    let mut response_headers = HeaderMap::new();
    for (name, val) in &upstream_headers {
        let name_str = name.as_str().to_lowercase();
        if !crate::server::auth::HOP_BY_HOP_HEADERS.contains(&name_str.as_str())
            && !crate::server::auth::RESPONSE_STRIPPED_HEADERS.contains(&name_str.as_str())
        {
            response_headers.insert(name.clone(), val.clone());
        }
    }

    let ch = compression_headers(&request_audit, "input");
    for (k, v) in ch {
        if let Some(name) = k {
            response_headers.insert(name, v);
        }
    }

    let bh = brain_headers(0, false);
    for (k, v) in bh {
        if let Some(name) = k {
            response_headers.insert(name, v);
        }
    }

    if let Some(status) = l1_status {
        response_headers.insert("x-brain-l1-cache", status.parse().unwrap());
    }
    if let Some(status) = l2_status {
        response_headers.insert("x-brain-l2-cache", status.parse().unwrap());
    }

    let mut final_res_bytes = res_bytes.clone();
    let mut response_audit: Option<crate::compression::CompressionAudit> = None;
    let output_compression = output_compression_policy
        .unwrap_or_else(|| rt.get("output_compression").and_then(|v| v.as_bool()).unwrap_or(false));

    if output_compression && should_transform_json_response(&path, &response_headers) {
        if let Ok(response_payload) = serde_json::from_slice::<serde_json::Value>(&res_bytes) {
            let (transformed_response, audit) = state.compressor.compress_response_payload(&response_payload, &path, true);
            response_audit = Some(audit);
            if response_audit.as_ref().unwrap().touched() {
                if let Ok(out) = serde_json::to_vec(&transformed_response) {
                    final_res_bytes = out;
                    let och = compression_headers(response_audit.as_ref().unwrap(), "output");
                    for (k, v) in och {
                        if let Some(name) = k {
                            response_headers.insert(name, v);
                        }
                    }
                    response_headers.insert("content-type", "application/json".parse().unwrap());
                }
            }
        }
    }

    if path.trim_start_matches('/').trim_end_matches('/') == "v1/messages" && status.is_success() {
        if let Ok(res_json) = serde_json::from_slice::<serde_json::Value>(&final_res_bytes) {
            let mut model_id = res_json.get("model").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if model_id.is_empty() {
                model_id = request_model.clone().unwrap_or_default();
            }
            let usage_map = crate::cost::extract_usage_from_anthropic(&res_json);
            let cost_record = crate::cost::estimate(
                "anthropic",
                &model_id,
                *usage_map.get("input_tokens").unwrap_or(&0),
                *usage_map.get("output_tokens").unwrap_or(&0),
                *usage_map.get("cache_write_tokens").unwrap_or(&0),
                *usage_map.get("cache_read_tokens").unwrap_or(&0),
            );
            state.cost_tracker.record(&cost_record);
            state.usage_budget.record(
                bytes_in as i64,
                *usage_map.get("input_tokens").unwrap_or(&0) + *usage_map.get("output_tokens").unwrap_or(&0),
            );
            if cost_record.matched {
                response_headers.insert("x-brain-cost-usd", format!("{:.6}", cost_record.usd).parse().unwrap());
            }
        }
    }

    let latency_ms = started_perf.elapsed().as_secs_f64() * 1000.0;
    let mapped_req_audit = map_compression_audit(&request_audit);
    let mapped_res_audit = response_audit.as_ref().map(map_response_audit);

    state.audit_logger.record(
        method.as_str(),
        &path,
        Some(status.as_u16()),
        &mapped_req_audit,
        mapped_res_audit.as_ref(),
        upstream_headers.get("request-id").and_then(|v| v.to_str().ok()),
        None,
        Some(latency_ms),
        bytes_in,
        final_res_bytes.len(),
        request_model.as_deref(),
        None,
    );

    if (cache_active || l2_active) && status.is_success() && cache_lookup_payload.is_some() {
        let cached_resp = crate::cache::l1::CachedResponse {
            status_code: status.as_u16(),
            headers: {
                let mut map = std::collections::HashMap::new();
                for (k, v) in &response_headers {
                    map.insert(k.as_str().to_string(), v.to_str().unwrap_or("").to_string());
                }
                map
            },
            body: final_res_bytes.clone(),
            media_type: upstream_headers.get("content-type").and_then(|v| v.to_str().ok().map(|s| s.to_string())),
            inserted_at: crate::cache::l1::CachedResponse::new(status.as_u16(), std::collections::HashMap::new(), vec![], None).inserted_at,
            last_hit_at: crate::cache::l1::CachedResponse::new(status.as_u16(), std::collections::HashMap::new(), vec![], None).last_hit_at,
            hit_count: 0,
        };

        if cache_active {
            if let Some(ref l1) = state.l1_cache {
                if let Some(ref key) = l1_key {
                    l1.put(key, &cached_resp);
                }
            }
        }

        if l2_active {
            if let Some(ref payload_val) = cache_lookup_payload {
                let point_id = l1_key.clone().unwrap_or_else(|| {
                    crate::cache::normalize::cache_key(payload_val, Some(&key_ctx))
                });
                let embed_text = crate::cache::normalize::canonical_text(payload_val, Some(&key_ctx));
                state.l2_cache.put_similar(&embed_text, &cached_resp, &point_id).await;
            }
        }
    }

    response_with_headers(status.as_u16(), response_headers, final_res_bytes).into_response()
}

pub fn create_router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz_handler))
        .route("/stats", get(stats_handler))
        .route("/stats/timeseries", get(stats_timeseries_handler))
        .route("/stats/recent", get(stats_recent_handler))
        .route("/settings", get(get_settings_handler).post(post_settings_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/cost", get(cost_handler))
        .route("/cost/reset", post(cost_reset_handler))
        .route("/budget", get(budget_handler))
        .route("/budget/reset", post(budget_reset_handler))
        .route("/rate-limit", get(rate_limit_handler))
        .route("/policies", get(policies_handler))
        .route("/providers", get(providers_handler))
        .route("/preview", post(preview_handler))
        .route("/metrics", get(metrics_handler))
        .route("/cache/purge", post(cache_purge_handler))
        .route("/cache/stats", get(cache_stats_handler))
        .route("/v1/responses", post(responses::responses_handler))
        .fallback(proxy_handler)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{header, HeaderValue, Request};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    fn test_state(
        upstream_base_url: String,
        mut settings: Settings,
        policy_router: crate::server::policies::PolicyRouter,
    ) -> Arc<ServerState> {
        settings.upstream_base_url = upstream_base_url;
        settings.audit_enabled = false;
        settings.l2_cache_enabled = false;
        settings.timeout_read_s = 5.0;

        let compressor = PayloadCompressor::new(settings.clone());
        let l1_cache = if settings.l1_cache_enabled {
            Some(
                crate::cache::l1::L1Cache::new(
                    &settings.l1_cache_db_path,
                    settings.l1_cache_max_entries,
                    settings.l1_cache_max_body_bytes,
                )
                .expect("enabled test l1 cache should initialize"),
            )
        } else {
            None
        };
        let l2_cache = crate::cache::l2::L2Cache::new(None, None, 0.97, false, true)
            .expect("disabled l2 cache should initialize");
        let rate_limiter = RequestLimiter::new(
            settings.rate_limit_capacity as i64,
            settings.rate_limit_refill_per_second,
            100,
        );
        let cost_tracker = Arc::new(CostTracker::new());
        let usage_budget = UsageBudget::new(None, None);
        let audit_logger = Arc::new(AuditLogger::new(&settings));
        let runtime_settings = RwLock::new(default_runtime_settings(&settings));
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(settings.timeout_read_s as u64))
            .build()
            .expect("test client should build");

        Arc::new(ServerState {
            settings,
            compressor,
            l1_cache,
            l2_cache,
            rate_limiter,
            cost_tracker,
            usage_budget,
            audit_logger,
            policy_router,
            runtime_settings,
            http_client,
        })
    }

    async fn spawn_json_upstream(payload: serde_json::Value) -> (String, tokio::task::JoinHandle<()>) {
        let payload = Arc::new(payload);
        let app = Router::new().route(
            "/v1/messages",
            post(move || {
                let payload = payload.clone();
                async move {
                    (
                        [(
                            axum::http::HeaderName::from_static("x-upstream-marker"),
                            HeaderValue::from_static("present"),
                        )],
                        Json((*payload).clone()),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test upstream");
        let addr = listener.local_addr().expect("test upstream address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{}", addr), handle)
    }

    async fn spawn_counting_json_upstream(
        payload: serde_json::Value,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let payload = Arc::new(payload);
        let calls = Arc::new(AtomicUsize::new(0));
        let app_calls = calls.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move || {
                let payload = payload.clone();
                let app_calls = app_calls.clone();
                async move {
                    app_calls.fetch_add(1, Ordering::SeqCst);
                    Json((*payload).clone())
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test upstream");
        let addr = listener.local_addr().expect("test upstream address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{}", addr), calls, handle)
    }

    async fn spawn_openai_responses_upstream(
        payload: serde_json::Value,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let payload = Arc::new(payload);
        let app = Router::new().route(
            "/responses",
            post(move || {
                let payload = payload.clone();
                async move {
                    (
                        [(
                            axum::http::HeaderName::from_static("x-request-id"),
                            HeaderValue::from_static("resp_test_request"),
                        )],
                        Json((*payload).clone()),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind OpenAI responses test upstream");
        let addr = listener.local_addr().expect("test upstream address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{}", addr), handle)
    }

    async fn spawn_openai_responses_sse_upstream(
        body: String,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let body = Arc::new(body);
        let app = Router::new().route(
            "/responses",
            post(move || {
                let body = body.clone();
                async move {
                    (
                        [
                            (header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream")),
                            (
                                axum::http::HeaderName::from_static("x-request-id"),
                                HeaderValue::from_static("resp_sse_request"),
                            ),
                        ],
                        (*body).clone(),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind OpenAI responses SSE test upstream");
        let addr = listener.local_addr().expect("test upstream address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{}", addr), handle)
    }

    async fn spawn_capturing_openai_responses_upstream(
        payload: serde_json::Value,
    ) -> (String, Arc<StdMutex<Vec<serde_json::Value>>>, tokio::task::JoinHandle<()>) {
        let payload = Arc::new(payload);
        let captured = Arc::new(StdMutex::new(Vec::new()));
        let app_captured = captured.clone();
        let app = Router::new().route(
            "/responses",
            post(move |body: axum::body::Bytes| {
                let payload = payload.clone();
                let app_captured = app_captured.clone();
                async move {
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) {
                        app_captured.lock().expect("capture lock").push(value);
                    }
                    Json((*payload).clone())
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind capturing OpenAI responses test upstream");
        let addr = listener.local_addr().expect("test upstream address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{}", addr), captured, handle)
    }

    async fn spawn_counting_openai_responses_upstream(
        payload: serde_json::Value,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let payload = Arc::new(payload);
        let calls = Arc::new(AtomicUsize::new(0));
        let app_calls = calls.clone();
        let app = Router::new().route(
            "/responses",
            post(move || {
                let payload = payload.clone();
                let app_calls = app_calls.clone();
                async move {
                    app_calls.fetch_add(1, Ordering::SeqCst);
                    Json((*payload).clone())
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind counting OpenAI responses test upstream");
        let addr = listener.local_addr().expect("test upstream address");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{}", addr), calls, handle)
    }

    async fn spawn_incomplete_body_upstream() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind incomplete upstream");
        let addr = listener.local_addr().expect("incomplete upstream address");
        let handle = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0_u8; 4096];
                let _ = socket.read(&mut buf).await;
                let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"partial\":";
                let _ = socket.write_all(response).await;
            }
        });
        (format!("http://{}", addr), handle)
    }

    fn messages_request(auth: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/messages")
            .header("authorization", auth)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "model": "claude-sonnet-4",
                    "messages": [
                        {
                            "role": "user",
                            "content": [
                                {"type": "text", "text": "hello"}
                            ]
                        }
                    ]
                })
                .to_string(),
            ))
            .expect("build request")
    }

    async fn json_body(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("response body should be json")
    }

    #[tokio::test]
    async fn settings_get_reports_unavailable_caches_disabled() {
        let state = test_state(
            "http://127.0.0.1:9".to_string(),
            Settings::default(),
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );
        {
            let mut rt = state.runtime_settings.write().await;
            rt["l1_cache"] = json!(true);
            rt["l2_cache"] = json!(true);
        }

        let response = create_router(state)
            .oneshot(
                Request::builder()
                    .uri("/settings")
                    .body(Body::empty())
                    .expect("build settings request"),
            )
            .await
            .expect("settings response");
        let body = json_body(response).await;

        assert_eq!(body.get("l1_cache").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(body.get("l2_cache").and_then(|v| v.as_bool()), Some(false));
    }

    #[tokio::test]
    async fn settings_post_rejects_enabling_unavailable_caches() {
        let state = test_state(
            "http://127.0.0.1:9".to_string(),
            Settings::default(),
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );

        let response = create_router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"l1_cache": true, "l2_cache": true}).to_string()))
                    .expect("build settings request"),
            )
            .await
            .expect("settings response");
        let status = response.status();
        let body = json_body(response).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .contains("cache backend is not initialized"),
            "unexpected error body: {body:?}"
        );
    }

    #[tokio::test]
    async fn initialized_l1_cache_serves_second_identical_messages_request() {
        let (upstream, calls, handle) = spawn_counting_json_upstream(json!({
            "model": "claude-sonnet-4",
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))
        .await;
        let settings = Settings {
            l1_cache_enabled: true,
            ..Default::default()
        };
        let state = test_state(
            upstream,
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );
        let router = create_router(state);

        let first = router
            .clone()
            .oneshot(messages_request("Bearer l1-cache-client"))
            .await
            .expect("first response");
        let second = router
            .oneshot(messages_request("Bearer l1-cache-client"))
            .await
            .expect("second response");
        handle.abort();

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            second.headers().get("x-brain-l1-cache"),
            Some(&HeaderValue::from_static("hit"))
        );
    }

    #[tokio::test]
    async fn responses_api_records_openai_usage_and_preserves_body() {
        let upstream_body = json!({
            "id": "resp_test",
            "object": "response",
            "status": "completed",
            "model": "gpt-4o-mini-2024-07-18",
            "usage": {
                "input_tokens": 11,
                "output_tokens": 7,
                "total_tokens": 18,
                "input_tokens_details": {
                    "cached_tokens": 3
                }
            },
            "output": []
        });
        let (upstream, handle) = spawn_openai_responses_upstream(upstream_body.clone()).await;
        let settings = Settings {
            openai_upstream_url: upstream,
            ..Default::default()
        };
        let state = test_state(
            "http://127.0.0.1:9".to_string(),
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );

        let response = create_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"model": "gpt-4o-mini", "input": "hello"}).to_string()))
                    .expect("build responses request"),
            )
            .await
            .expect("responses proxy response");
        handle.abort();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("x-request-id"),
            Some(&HeaderValue::from_static("resp_test_request"))
        );
        let body = json_body(response).await;
        assert_eq!(body, upstream_body);

        let stats = state.audit_logger.stats.lock().snapshot();
        assert_eq!(stats.get("requests_total").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(stats.get("bytes_out_total").and_then(|v| v.as_u64()), Some(body.to_string().len() as u64));

        let recent = state.audit_logger.stats.lock().recent_records(1);
        assert_eq!(recent[0].get("path").and_then(|v| v.as_str()), Some("/v1/responses"));
        assert_eq!(recent[0].get("provider").and_then(|v| v.as_str()), Some("openai"));
        assert_eq!(
            recent[0].get("model").and_then(|v| v.as_str()),
            Some("gpt-4o-mini-2024-07-18")
        );
        assert_eq!(
            recent[0].get("request_id").and_then(|v| v.as_str()),
            Some("resp_test_request")
        );

        let cost = state.cost_tracker.snapshot();
        assert_eq!(cost.get("total_requests").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(
            cost.get("by_provider")
                .and_then(|v| v.get("openai"))
                .and_then(|v| v.get("requests"))
                .and_then(|v| v.as_i64()),
            Some(1)
        );
        let model_row = cost
            .get("by_model")
            .and_then(|v| v.get("openai:gpt-4o-mini-2024-07-18"))
            .expect("OpenAI model cost row should be recorded");
        assert_eq!(model_row.get("input_tokens").and_then(|v| v.as_i64()), Some(11));
        assert_eq!(model_row.get("output_tokens").and_then(|v| v.as_i64()), Some(7));
        assert_eq!(model_row.get("cache_read_tokens").and_then(|v| v.as_i64()), Some(3));
    }

    #[tokio::test]
    async fn responses_api_records_streaming_openai_usage() {
        let completed = json!({
            "type": "response.completed",
            "response": {
                "id": "resp_stream_test",
                "status": "completed",
                "model": "gpt-4o-mini-2024-07-18",
                "usage": {
                    "input_tokens": 13,
                    "output_tokens": 5,
                    "input_tokens_details": {
                        "cached_tokens": 4
                    }
                }
            }
        });
        let sse_body = format!(
            "event: response.created\ndata: {{\"type\":\"response.created\"}}\n\nevent: response.completed\ndata: {}\n\n",
            completed
        );
        let (upstream, handle) = spawn_openai_responses_sse_upstream(sse_body.clone()).await;
        let settings = Settings {
            openai_upstream_url: upstream,
            ..Default::default()
        };
        let state = test_state(
            "http://127.0.0.1:9".to_string(),
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );

        let response = create_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"model": "gpt-4o-mini", "stream": true}).to_string()))
                    .expect("build responses request"),
            )
            .await
            .expect("responses proxy response");
        handle.abort();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/event-stream"))
        );
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read SSE body");
        assert_eq!(String::from_utf8(body.to_vec()).expect("SSE body utf8"), sse_body);

        let recent = state.audit_logger.stats.lock().recent_records(1);
        assert_eq!(
            recent[0].get("model").and_then(|v| v.as_str()),
            Some("gpt-4o-mini-2024-07-18")
        );
        assert_eq!(
            recent[0].get("request_id").and_then(|v| v.as_str()),
            Some("resp_sse_request")
        );

        let cost = state.cost_tracker.snapshot();
        let model_row = cost
            .get("by_model")
            .and_then(|v| v.get("openai:gpt-4o-mini-2024-07-18"))
            .expect("OpenAI streaming model cost row should be recorded");
        assert_eq!(model_row.get("input_tokens").and_then(|v| v.as_i64()), Some(13));
        assert_eq!(model_row.get("output_tokens").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(model_row.get("cache_read_tokens").and_then(|v| v.as_i64()), Some(4));
    }

    #[tokio::test]
    async fn responses_api_input_compression_records_chars_saved_and_forwards_transformed_body() {
        let upstream_body = json!({
            "id": "resp_compressed",
            "status": "completed",
            "model": "gpt-4o-mini-2024-07-18",
            "usage": {"input_tokens": 11, "output_tokens": 3},
            "output": []
        });
        let (upstream, captured, handle) = spawn_capturing_openai_responses_upstream(upstream_body).await;
        let settings = Settings {
            openai_upstream_url: upstream,
            input_compression_enabled: true,
            max_text_chars: 120,
            min_omission_chars: 40,
            ..Default::default()
        };
        let state = test_state(
            "http://127.0.0.1:9".to_string(),
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );
        let long_input = "abcdef ".repeat(300);

        let response = create_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({
                        "model": "gpt-4o-mini",
                        "input": long_input,
                    }).to_string()))
                    .expect("build responses request"),
            )
            .await
            .expect("responses proxy response");
        handle.abort();

        assert_eq!(response.status(), StatusCode::OK);
        let sent = captured.lock().expect("capture lock");
        let forwarded_input = sent[0].get("input").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            forwarded_input.contains("middle-out compressed locally"),
            "upstream should receive compressed input, got: {forwarded_input}"
        );

        let stats = state.audit_logger.stats.lock().snapshot();
        assert!(
            stats.get("chars_saved_in").and_then(|v| v.as_u64()).unwrap_or(0) > 0,
            "Codex input compression should contribute to saved-char stats: {stats:?}"
        );
        assert_eq!(stats.get("compressed_requests").and_then(|v| v.as_u64()), Some(1));
    }

    #[tokio::test]
    async fn responses_api_l1_cache_serves_second_identical_non_streaming_request() {
        let upstream_body = json!({
            "id": "resp_cached",
            "status": "completed",
            "model": "gpt-4o-mini-2024-07-18",
            "usage": {"input_tokens": 2, "output_tokens": 1},
            "output": []
        });
        let (upstream, calls, handle) = spawn_counting_openai_responses_upstream(upstream_body).await;
        let settings = Settings {
            openai_upstream_url: upstream,
            l1_cache_enabled: true,
            ..Default::default()
        };
        let state = test_state(
            "http://127.0.0.1:9".to_string(),
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );
        let router = create_router(state);
        let request_body = json!({"model": "gpt-4o-mini", "input": "cache me"}).to_string();

        let first = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.clone()))
                    .expect("build first responses request"),
            )
            .await
            .expect("first response");
        let second = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body))
                    .expect("build second responses request"),
            )
            .await
            .expect("second response");
        handle.abort();

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            second.headers().get("x-brain-l1-cache"),
            Some(&HeaderValue::from_static("hit"))
        );
    }

    #[tokio::test]
    async fn responses_api_output_compression_records_chars_saved_out() {
        let long_output = "uvwxyz ".repeat(300);
        let upstream_body = json!({
            "id": "resp_output_compressed",
            "status": "completed",
            "model": "gpt-4o-mini-2024-07-18",
            "usage": {"input_tokens": 3, "output_tokens": 9},
            "output": [
                {
                    "type": "message",
                    "content": [
                        {"type": "output_text", "text": long_output}
                    ]
                }
            ]
        });
        let (upstream, handle) = spawn_openai_responses_upstream(upstream_body).await;
        let settings = Settings {
            openai_upstream_url: upstream,
            output_compression_enabled: true,
            output_max_text_chars: 120,
            min_omission_chars: 40,
            ..Default::default()
        };
        let state = test_state(
            "http://127.0.0.1:9".to_string(),
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );

        let response = create_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"model": "gpt-4o-mini", "input": "hello"}).to_string()))
                    .expect("build responses request"),
            )
            .await
            .expect("responses proxy response");
        handle.abort();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        let body_text = String::from_utf8(body.to_vec()).expect("json response utf8");
        assert!(
            body_text.contains("middle-out compressed locally"),
            "Codex output response text should be compressed: {body_text}"
        );

        let stats = state.audit_logger.stats.lock().snapshot();
        assert!(
            stats.get("chars_saved_out").and_then(|v| v.as_u64()).unwrap_or(0) > 0,
            "Codex output compression should contribute to saved-char stats: {stats:?}"
        );
        assert_eq!(stats.get("compressed_requests").and_then(|v| v.as_u64()), Some(1));
    }

    #[tokio::test]
    async fn proxy_forwards_upstream_and_middleout_headers_on_messages() {
        let upstream_payload = json!({
            "model": "claude-sonnet-4",
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        });
        let (upstream, handle) = spawn_json_upstream(upstream_payload).await;
        let settings = Settings::default();
        let state = test_state(
            upstream,
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );

        let response = create_router(state.clone())
            .oneshot(messages_request("Bearer header-forwarding-test"))
            .await
            .expect("proxy response");
        handle.abort();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("x-upstream-marker"),
            Some(&HeaderValue::from_static("present"))
        );
        assert_eq!(
            response.headers().get("x-brain-proxy"),
            Some(&HeaderValue::from_static("middleout-proxy-rs/0.2.0"))
        );
        assert!(
            response.headers().get(header::CONTENT_TYPE).is_some(),
            "proxied JSON response should preserve a content-type header"
        );
        let recent = state.audit_logger.stats.lock().recent_records(1);
        assert_eq!(recent[0].get("provider").and_then(|v| v.as_str()), Some("anthropic"));
        let stats = state.audit_logger.stats.lock().snapshot();
        assert_eq!(
            stats.get("by_provider")
                .and_then(|v| v.get("anthropic"))
                .and_then(|v| v.get("requests"))
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn rate_limit_setting_rejects_second_request_for_same_client() {
        let (upstream, handle) = spawn_json_upstream(json!({
            "model": "claude-sonnet-4",
            "content": [{"type": "text", "text": "ok"}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))
        .await;
        let settings = Settings {
            rate_limit_enabled: true,
            rate_limit_capacity: 1,
            rate_limit_refill_per_second: 0.0,
            ..Default::default()
        };
        let state = test_state(
            upstream,
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );
        let router = create_router(state);

        let first = router
            .clone()
            .oneshot(messages_request("Bearer limited-client"))
            .await
            .expect("first response");
        let second = router
            .oneshot(messages_request("Bearer limited-client"))
            .await
            .expect("second response");
        handle.abort();

        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn upstream_body_read_error_returns_bad_gateway() {
        let (upstream, handle) = spawn_incomplete_body_upstream().await;
        let settings = Settings::default();
        let state = test_state(
            upstream,
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), None),
        );

        let response = create_router(state)
            .oneshot(messages_request("Bearer truncated-upstream"))
            .await
            .expect("proxy response");
        handle.abort();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn policy_output_compression_overrides_runtime_default() {
        let long_text = "abcdef ".repeat(400);
        let (upstream, handle) = spawn_json_upstream(json!({
            "model": "claude-sonnet-4",
            "content": [{"type": "text", "text": long_text}],
            "usage": {"input_tokens": 1, "output_tokens": 1}
        }))
        .await;
        let settings = Settings {
            output_compression_enabled: false,
            output_max_text_chars: 512,
            min_omission_chars: 100,
            head_fraction: 0.5,
            ..Default::default()
        };
        let policy = crate::server::policies::CompressionPolicy {
            output_compression: true,
            ..Default::default()
        };
        let state = test_state(
            upstream,
            settings,
            crate::server::policies::PolicyRouter::new(Vec::new(), Some(policy)),
        );

        let response = create_router(state)
            .oneshot(messages_request("Bearer output-policy"))
            .await
            .expect("proxy response");
        handle.abort();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read response body");
        let body_text = String::from_utf8(body.to_vec()).expect("json body should be utf8");

        assert!(
            body_text.contains("middle-out compressed locally"),
            "policy-enabled output compression should rewrite long response text"
        );
    }
}
