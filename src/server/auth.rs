use std::collections::HashMap;
use serde_json::{Value, json};
use sha2::{Sha256, Digest};
use axum::http::HeaderMap;
use crate::config::Settings;

pub const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "content-encoding",
];

pub const RESPONSE_STRIPPED_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "anthropic-api-key",
    "proxy-authorization",
    "set-cookie",
];

pub struct StrictAuthError(pub String);

pub fn forward_request_headers(
    headers: &HeaderMap,
    settings: &Settings,
) -> Result<HashMap<String, String>, StrictAuthError> {
    let mut forwarded = HashMap::new();
    let mut saw_api_key_header = false;

    for (name, value) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        let value_str = match value.to_str() {
            Ok(v) => v.to_string(),
            Err(_) => continue,
        };

        if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
            continue;
        }

        // Strict subscription mode: API-key headers are never forwarded upstream.
        if name_str == "x-api-key" || name_str == "anthropic-api-key" {
            saw_api_key_header = true;
            continue;
        }

        if name_str == "accept-encoding" {
            continue;
        }

        forwarded.insert(name_str, value_str);
    }

    if saw_api_key_header {
        return Err(StrictAuthError(
            "This build is subscription/OAuth-only and rejected an X-Api-Key style header. \
             Unset ANTHROPIC_API_KEY, remove apiKeyHelper output, and run Claude Code /status \
             until it shows your Claude subscription login.".to_string()
        ));
    }

    let authorization = forwarded.get("authorization").cloned().unwrap_or_default();
    let authorization_trimmed = authorization.trim();
    if !authorization_trimmed.to_lowercase().starts_with("bearer ") {
        return Err(StrictAuthError(
            "Missing OAuth Authorization: Bearer header from Claude Code. Start Claude Code with \
             ANTHROPIC_BASE_URL=http://127.0.0.1:8787 after logging in with /login, and make sure \
             ANTHROPIC_API_KEY and ANTHROPIC_AUTH_TOKEN are unset in the Claude Code shell.".to_string()
        ));
    }

    if authorization_trimmed.contains(',') {
        return Err(StrictAuthError(
            "Authorization header contains a comma; comma-folded credentials \
             are rejected to prevent smuggling API keys past the Bearer check.".to_string()
        ));
    }

    if !forwarded.contains_key("anthropic-version") {
        forwarded.insert("anthropic-version".to_string(), settings.default_anthropic_version.clone());
    }

    forwarded.insert("x-middleout-proxy".to_string(), "middleout-claude-proxy/0.2.0-strict-subscription".to_string());

    Ok(forwarded)
}

pub fn cache_key_context(
    request_headers: &HashMap<String, String>,
    auth_header: &str,
) -> Value {
    let mut ctx = serde_json::Map::new();

    if let Some(version) = request_headers.get("anthropic-version") {
        ctx.insert("anthropic_version".to_string(), json!(version));
    }

    if let Some(beta) = request_headers.get("anthropic-beta") {
        let mut feats: Vec<String> = beta.split(',')
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect();
        feats.sort();
        if !feats.is_empty() {
            ctx.insert("anthropic_beta".to_string(), json!(feats));
        }
    }

    if !auth_header.is_empty() {
        let mut hasher = Sha256::new();
        hasher.update(auth_header.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        ctx.insert("auth".to_string(), json!(digest[..16].to_string()));
    }

    Value::Object(ctx)
}

pub fn rate_limit_key(
    request_headers: &HashMap<String, String>,
    auth_header: &str,
) -> String {
    let auth_trimmed = auth_header.trim();
    if !auth_trimmed.is_empty() {
        let mut hasher = Sha256::new();
        hasher.update(auth_trimmed.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        return format!("auth:{}", &digest[..16]);
    }

    if let Some(forwarded_for) = request_headers.get("x-forwarded-for") {
        if let Some(first) = forwarded_for.split(',').next() {
            let first = first.trim();
            if !first.is_empty() {
                return format!("ip:{}", first);
            }
        }
    }

    "anonymous".to_string()
}

pub fn forward_response_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let mut forwarded = HashMap::new();
    for (name, value) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        let value_str = match value.to_str() {
            Ok(v) => v.to_string(),
            Err(_) => continue,
        };

        if HOP_BY_HOP_HEADERS.contains(&name_str.as_str()) {
            continue;
        }

        if RESPONSE_STRIPPED_HEADERS.contains(&name_str.as_str()) {
            continue;
        }

        forwarded.insert(name_str, value_str);
    }
    forwarded
}
