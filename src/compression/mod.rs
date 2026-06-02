pub mod jl;
pub mod engines;
pub mod cache_wall;
pub mod lsh_dedupe;

use sha2::{Sha256, Digest};
use parking_lot::Mutex;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressionEvent {
    pub path: String,
    pub mode: String,
    pub original_chars: usize,
    pub compressed_chars: usize,
    pub sha256: String,
    pub note: String,
    pub sample_before: Option<String>,
    pub sample_after: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auto_insert_cache_wall_adds_cache_control_to_static_prefix() {
        let settings = crate::config::Settings {
            auto_insert_cache_wall: true,
            preserve_anthropic_cache: true,
            input_compression_enabled: true,
            ..Default::default()
        };

        let compressor = PayloadCompressor::new(settings);
        let payload = json!({
            "system": "stable reusable system prompt",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "hello"}
                    ]
                }
            ]
        });

        let (compressed, _) = compressor
            .compress_request_payload(&payload, "/v1/messages", None, true)
            .expect("compression should succeed");

        let system = compressed
            .get("system")
            .and_then(|value| value.as_array())
            .expect("string system prompt should be converted to block array");
        assert!(
            system
                .first()
                .and_then(|block| block.get("cache_control"))
                .is_some(),
            "auto-inserted cache wall should add cache_control to the reusable prefix"
        );
    }
}

impl CompressionEvent {
    pub fn chars_saved(&self) -> usize {
        if self.original_chars > self.compressed_chars {
            self.original_chars - self.compressed_chars
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressionAudit {
    pub endpoint: String,
    pub events: Vec<CompressionEvent>,
    pub cache_hits: usize,
    pub cache_misses: usize,
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

pub fn sha256_short(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)[..16].to_string()
}

pub fn middle_out_text(
    text: &str,
    max_chars: usize,
    min_omission_chars: usize,
    head_fraction: f64,
) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }

    let digest = sha256_short(text);
    let build_marker = |omitted: usize, original: usize, digest: &str| -> String {
        format!(
            "\n\n[... middle-out compressed locally: omitted {} chars; original_chars={}; sha256={}; not reversible by the model ...]\n\n",
            omitted, original, digest
        )
    };

    let mut marker = build_marker(char_count - max_chars, char_count, &digest);
    let marker_char_count = marker.chars().count();

    let mut budget = max_chars.saturating_sub(marker_char_count).max(128);
    let mut head_chars = ((budget as f64) * head_fraction) as usize;
    head_chars = head_chars.max(64);
    head_chars = head_chars.min(budget.saturating_sub(64).max(64));
    let mut tail_chars = budget.saturating_sub(head_chars).max(64);

    if head_chars + tail_chars >= char_count.saturating_sub(min_omission_chars) {
        return text.to_string();
    }

    for _ in 0..3 {
        let omitted = char_count.saturating_sub(head_chars + tail_chars);
        let new_marker = build_marker(omitted, char_count, &digest);
        let new_marker_len = new_marker.chars().count();
        let old_marker_len = marker.chars().count();
        if new_marker_len == old_marker_len {
            marker = new_marker;
            break;
        }
        marker = new_marker;
        budget = max_chars.saturating_sub(new_marker_len).max(128);
        head_chars = ((budget as f64) * head_fraction) as usize;
        head_chars = head_chars.max(64);
        head_chars = head_chars.min(budget.saturating_sub(64).max(64));
        tail_chars = budget.saturating_sub(head_chars).max(64);
    }

    let text_chars: Vec<char> = text.chars().collect();
    let head: String = text_chars[..head_chars].iter().collect();
    let tail: String = text_chars[text_chars.len() - tail_chars..].iter().collect();

    format!("{}{}{}", head.trim_end(), marker, tail.trim_start())
}

pub fn duplicate_marker(text: &str, record_path: &str, similarity: f64) -> String {
    let digest = sha256_short(text);
    format!(
        "[Near-duplicate content omitted locally by JL-style request sketch. Similar to earlier block at {}; similarity={:.3}; original_chars={}; sha256={}.]",
        record_path, similarity, text.chars().count(), digest
    )
}

#[derive(Debug)]
struct LruCacheData {
    map: HashMap<String, String>,
    order: Vec<String>,
    hits: usize,
    misses: usize,
}

#[derive(Debug)]
pub struct CompressionResultCache {
    max_entries: usize,
    data: Mutex<LruCacheData>,
}

impl CompressionResultCache {
    pub fn new(max_entries: usize) -> Self {
        CompressionResultCache {
            max_entries,
            data: Mutex::new(LruCacheData {
                map: HashMap::new(),
                order: Vec::new(),
                hits: 0,
                misses: 0,
            }),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        if self.max_entries == 0 {
            return None;
        }
        let mut guard = self.data.lock();
        if let Some(val) = guard.map.get(key).cloned() {
            guard.hits += 1;
            if let Some(pos) = guard.order.iter().position(|k| k == key) {
                guard.order.remove(pos);
            }
            guard.order.push(key.to_string());
            Some(val)
        } else {
            guard.misses += 1;
            None
        }
    }

    pub fn put(&self, key: &str, value: &str) {
        if self.max_entries == 0 {
            return;
        }
        let mut guard = self.data.lock();
        if guard.map.contains_key(key) {
            guard.map.insert(key.to_string(), value.to_string());
            if let Some(pos) = guard.order.iter().position(|k| k == key) {
                guard.order.remove(pos);
            }
            guard.order.push(key.to_string());
        } else {
            guard.map.insert(key.to_string(), value.to_string());
            guard.order.push(key.to_string());
            while guard.order.len() > self.max_entries {
                let oldest = guard.order.remove(0);
                guard.map.remove(&oldest);
            }
        }
    }

    pub fn stats(&self) -> serde_json::Value {
        let guard = self.data.lock();
        serde_json::json!({
            "size": guard.map.len(),
            "max_entries": self.max_entries,
            "hits": guard.hits,
            "misses": guard.misses,
        })
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CompressRequestOptions {
    pub jl_dedupe: Option<bool>,
    pub caveman: Option<serde_json::Value>,
    pub rtk: Option<serde_json::Value>,
    pub json_aware: Option<serde_json::Value>,
    pub lsh: Option<serde_json::Value>,
    pub max_text_chars: Option<usize>,
    pub auto_insert_cache_wall: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PayloadCompressor {
    pub settings: crate::config::Settings,
    pub result_cache: std::sync::Arc<CompressionResultCache>,
}

impl PayloadCompressor {
    pub fn new(settings: crate::config::Settings) -> Self {
        let cache_size = if settings.compression_cache_enabled {
            settings.compression_cache_size
        } else {
            0
        };
        PayloadCompressor {
            settings,
            result_cache: std::sync::Arc::new(CompressionResultCache::new(cache_size)),
        }
    }

    pub fn compress_request_payload(
        &self,
        payload: &serde_json::Value,
        endpoint: &str,
        opts: Option<CompressRequestOptions>,
        force_enabled: bool,
    ) -> Result<(serde_json::Value, CompressionAudit), String> {
        let mut audit = CompressionAudit::new(endpoint);
        if !force_enabled && !self.settings.input_compression_enabled {
            return Ok((payload.clone(), audit));
        }

        let opts = opts.unwrap_or(CompressRequestOptions {
            jl_dedupe: None,
            caveman: None,
            rtk: None,
            json_aware: None,
            lsh: None,
            max_text_chars: None,
            auto_insert_cache_wall: None,
        });

        let jl_active = opts.jl_dedupe.unwrap_or(self.settings.jl_dedupe_enabled);
        let max_text_chars_active = opts.max_text_chars.unwrap_or(self.settings.max_text_chars);
        let auto_insert_cache_wall = opts
            .auto_insert_cache_wall
            .unwrap_or(self.settings.auto_insert_cache_wall);

        let caveman_enabled = opts.caveman.as_ref()
            .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
            .unwrap_or(self.settings.caveman_enabled);
        let caveman_level = opts.caveman.as_ref()
            .and_then(|v| v.get("level").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| self.settings.caveman_level.clone());

        let rtk_enabled = opts.rtk.as_ref()
            .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
            .unwrap_or(self.settings.rtk_enabled);
        let rtk_level = opts.rtk.as_ref()
            .and_then(|v| v.get("level").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| self.settings.rtk_level.clone());

        let json_aware_enabled = opts.json_aware.as_ref()
            .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
            .unwrap_or(self.settings.json_aware_enabled);
        let json_aware_level = opts.json_aware.as_ref()
            .and_then(|v| v.get("level").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| self.settings.json_aware_level.clone());

        let lsh_enabled = opts.lsh.as_ref()
            .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
            .unwrap_or(self.settings.lsh_enabled);
        let lsh_level = opts.lsh.as_ref()
            .and_then(|v| v.get("level").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| self.settings.lsh_level.clone());

        let mut working = payload.clone();
        let mut sketch_index = crate::compression::jl::RequestSketchIndex::new(
            self.settings.jl_dims,
            self.settings.jl_shingle_tokens,
        );

        let wall = if self.settings.preserve_anthropic_cache {
            Some(crate::compression::cache_wall::compute_wall(
                &mut working,
                auto_insert_cache_wall,
            ))
        } else {
            None
        };

        if self.settings.compress_system {
            if let Some(system) = working.get_mut("system") {
                let is_protected = if let Some(ref w) = wall {
                    w.is_protected(crate::compression::cache_wall::BlockKind::System, None, 0)
                } else {
                    false
                };

                if is_protected {
                    audit.protected_blocks += 1;
                } else {
                    *system = self.compress_content_value(
                        system,
                        "system",
                        &mut audit,
                        &mut sketch_index,
                        false,
                        crate::compression::cache_wall::BlockKind::System,
                        None,
                        false, // JL/LSH never useful on static system prompt
                        max_text_chars_active,
                        caveman_enabled,
                        &caveman_level,
                        rtk_enabled,
                        &rtk_level,
                        json_aware_enabled,
                        &json_aware_level,
                        false,
                        &lsh_level,
                        wall.as_ref(),
                    )?;
                }
            }
        }

        if let Some(messages) = working.get_mut("messages").and_then(|m| m.as_array_mut()) {
            for (i, message) in messages.iter_mut().enumerate() {
                if !message.is_object() || message.get("content").is_none() {
                    continue;
                }
                let role = message.get("role").and_then(|r| r.as_str()).unwrap_or("message").to_string();
                let content_value = message.get_mut("content").unwrap();

                if content_value.is_string() {
                    let is_protected = if let Some(ref w) = wall {
                        w.is_protected(crate::compression::cache_wall::BlockKind::Message, Some(i), 0)
                    } else {
                        false
                    };

                    if is_protected {
                        audit.protected_blocks += 1;
                        continue;
                    }
                }

                // Only run JL/LSH on conversational turns — tool results are structured
                // and change every request, so dedup never fires and only burns CPU.
                let is_conversational = role == "user" || role == "assistant";
                *content_value = self.compress_content_value(
                    content_value,
                    &format!("messages[{}].{}.content", i, role),
                    &mut audit,
                    &mut sketch_index,
                    self.settings.compress_tool_results,
                    crate::compression::cache_wall::BlockKind::Message,
                    Some(i),
                    jl_active && is_conversational,
                    max_text_chars_active,
                    caveman_enabled,
                    &caveman_level,
                    rtk_enabled,
                    &rtk_level,
                    json_aware_enabled,
                    &json_aware_level,
                    lsh_enabled && is_conversational,
                    &lsh_level,
                    wall.as_ref(),
                )?;
            }
        }

        Ok((working, audit))
    }

    pub fn compress_openai_responses_request_payload(
        &self,
        payload: &serde_json::Value,
        endpoint: &str,
        opts: Option<CompressRequestOptions>,
        force_enabled: bool,
    ) -> Result<(serde_json::Value, CompressionAudit), String> {
        let mut audit = CompressionAudit::new(endpoint);
        if !force_enabled && !self.settings.input_compression_enabled {
            return Ok((payload.clone(), audit));
        }

        let opts = opts.unwrap_or(CompressRequestOptions {
            jl_dedupe: None,
            caveman: None,
            rtk: None,
            json_aware: None,
            lsh: None,
            max_text_chars: None,
            auto_insert_cache_wall: None,
        });

        let jl_active = opts.jl_dedupe.unwrap_or(self.settings.jl_dedupe_enabled);
        let max_text_chars_active = opts.max_text_chars.unwrap_or(self.settings.max_text_chars);
        let caveman_enabled = opts.caveman.as_ref()
            .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
            .unwrap_or(self.settings.caveman_enabled);
        let caveman_level = opts.caveman.as_ref()
            .and_then(|v| v.get("level").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| self.settings.caveman_level.clone());
        let rtk_enabled = opts.rtk.as_ref()
            .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
            .unwrap_or(self.settings.rtk_enabled);
        let rtk_level = opts.rtk.as_ref()
            .and_then(|v| v.get("level").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| self.settings.rtk_level.clone());
        let json_aware_enabled = opts.json_aware.as_ref()
            .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
            .unwrap_or(self.settings.json_aware_enabled);
        let json_aware_level = opts.json_aware.as_ref()
            .and_then(|v| v.get("level").and_then(|s| s.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| self.settings.json_aware_level.clone());

        let mut working = payload.clone();
        let mut sketch_index = crate::compression::jl::RequestSketchIndex::new(
            self.settings.jl_dims,
            self.settings.jl_shingle_tokens,
        );

        if let Some(instructions) = working.get_mut("instructions") {
            *instructions = self.compress_openai_input_value(
                instructions,
                "instructions",
                &mut audit,
                &mut sketch_index,
                false,
                max_text_chars_active,
                caveman_enabled,
                &caveman_level,
                rtk_enabled,
                &rtk_level,
                json_aware_enabled,
                &json_aware_level,
            )?;
        }

        if let Some(input) = working.get_mut("input") {
            *input = self.compress_openai_input_value(
                input,
                "input",
                &mut audit,
                &mut sketch_index,
                jl_active,
                max_text_chars_active,
                caveman_enabled,
                &caveman_level,
                rtk_enabled,
                &rtk_level,
                json_aware_enabled,
                &json_aware_level,
            )?;
        }

        Ok((working, audit))
    }

    #[allow(clippy::too_many_arguments)]
    fn compress_openai_input_value(
        &self,
        value: &serde_json::Value,
        path: &str,
        audit: &mut CompressionAudit,
        sketch_index: &mut crate::compression::jl::RequestSketchIndex,
        jl_active: bool,
        max_text_chars_active: usize,
        caveman_enabled: bool,
        caveman_level: &str,
        rtk_enabled: bool,
        rtk_level: &str,
        json_aware_enabled: bool,
        json_aware_level: &str,
    ) -> Result<serde_json::Value, String> {
        if let Some(text) = value.as_str() {
            let compressed = self.compress_text_with_dedupe(
                text,
                path,
                audit,
                sketch_index,
                jl_active,
                max_text_chars_active,
                caveman_enabled,
                caveman_level,
                rtk_enabled,
                rtk_level,
                json_aware_enabled,
                json_aware_level,
            )?;
            return Ok(serde_json::Value::String(compressed));
        }

        if let Some(arr) = value.as_array() {
            let mut out = arr.clone();
            for (i, item) in out.iter_mut().enumerate() {
                *item = self.compress_openai_input_value(
                    item,
                    &format!("{}[{}]", path, i),
                    audit,
                    sketch_index,
                    jl_active,
                    max_text_chars_active,
                    caveman_enabled,
                    caveman_level,
                    rtk_enabled,
                    rtk_level,
                    json_aware_enabled,
                    json_aware_level,
                )?;
            }
            return Ok(serde_json::Value::Array(out));
        }

        let Some(obj) = value.as_object() else {
            return Ok(value.clone());
        };
        let mut out = obj.clone();

        let text_type = out.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let is_text_item = text_type.is_empty() || matches!(text_type, "input_text" | "text");
        if is_text_item {
            if let Some(text) = out.get("text").and_then(|v| v.as_str()) {
                let compressed = self.compress_text_with_dedupe(
                    text,
                    &format!("{}.text", path),
                    audit,
                    sketch_index,
                    jl_active,
                    max_text_chars_active,
                    caveman_enabled,
                    caveman_level,
                    rtk_enabled,
                    rtk_level,
                    json_aware_enabled,
                    json_aware_level,
                )?;
                out.insert("text".to_string(), serde_json::Value::String(compressed));
            }
        }

        if let Some(content) = out.get("content").cloned() {
            out.insert(
                "content".to_string(),
                self.compress_openai_input_value(
                    &content,
                    &format!("{}.content", path),
                    audit,
                    sketch_index,
                    jl_active,
                    max_text_chars_active,
                    caveman_enabled,
                    caveman_level,
                    rtk_enabled,
                    rtk_level,
                    json_aware_enabled,
                    json_aware_level,
                )?,
            );
        }

        Ok(serde_json::Value::Object(out))
    }

    fn compress_content_value(
        &self,
        value: &serde_json::Value,
        path: &str,
        audit: &mut CompressionAudit,
        sketch_index: &mut crate::compression::jl::RequestSketchIndex,
        allow_tool_result: bool,
        kind: crate::compression::cache_wall::BlockKind,
        msg_idx: Option<usize>,
        jl_active: bool,
        max_text_chars_active: usize,
        caveman_enabled: bool,
        caveman_level: &str,
        rtk_enabled: bool,
        rtk_level: &str,
        json_aware_enabled: bool,
        json_aware_level: &str,
        lsh_enabled: bool,
        lsh_level: &str,
        wall: Option<&crate::compression::cache_wall::CacheWall>,
    ) -> Result<serde_json::Value, String> {
        if let Some(s) = value.as_str() {
            let compressed = self.compress_text_with_dedupe(
                s,
                path,
                audit,
                sketch_index,
                jl_active,
                max_text_chars_active,
                caveman_enabled,
                caveman_level,
                rtk_enabled,
                rtk_level,
                json_aware_enabled,
                json_aware_level,
            )?;
            return Ok(serde_json::Value::String(compressed));
        }

        let arr = match value.as_array() {
            Some(a) => a,
            None => return Ok(value.clone()),
        };

        let mut working_blocks = arr.clone();
        if lsh_enabled && !working_blocks.is_empty() {
            let mut protected_idx = std::collections::HashSet::new();
            if let Some(w) = wall {
                for i in 0..working_blocks.len() {
                    if w.is_protected(kind, msg_idx, i) {
                        protected_idx.insert(i);
                    }
                }
            }

            if let Ok((deduped, stats)) = crate::compression::lsh_dedupe::dedupe_blocks(&working_blocks, lsh_level, &protected_idx) {
                let replaced = stats.get("replaced").and_then(|r| r.as_u64()).unwrap_or(0);
                if replaced > 0 {
                    for (i, (old, new)) in arr.iter().zip(deduped.iter()).enumerate() {
                        if old != new {
                            let old_text = crate::compression::lsh_dedupe::block_text(old).unwrap_or_default();
                            let new_text = crate::compression::lsh_dedupe::block_text(new).unwrap_or_default();
                            let digest = sha256_short(&old_text);
                            audit.events.push(self.make_event(
                                &format!("{}[{}]", path, i),
                                "lsh-near-duplicate",
                                &old_text,
                                &new_text,
                                &digest,
                                &format!("level={}", lsh_level),
                            ));
                        }
                    }
                    working_blocks = deduped;
                }
            }
        }

        for (i, block) in working_blocks.iter_mut().enumerate() {
            let block_path = format!("{}[{}]", path, i);
            let is_protected = if let Some(w) = wall {
                w.is_protected(kind, msg_idx, i)
            } else {
                false
            };

            if is_protected {
                audit.protected_blocks += 1;
                continue;
            }

            if !block.is_object() {
                continue;
            }

            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if block_type == "text" {
                if let Some(text_str) = block.get("text").and_then(|t| t.as_str()) {
                    let compressed = self.compress_text_with_dedupe(
                        text_str,
                        &format!("{}.text", block_path),
                        audit,
                        sketch_index,
                        jl_active,
                        max_text_chars_active,
                        caveman_enabled,
                        caveman_level,
                        rtk_enabled,
                        rtk_level,
                        json_aware_enabled,
                        json_aware_level,
                    )?;
                    block.as_object_mut().unwrap().insert("text".to_string(), serde_json::Value::String(compressed));
                }
            } else if block_type == "tool_result" && allow_tool_result {
                if let Some(content) = block.get_mut("content") {
                    *content = self.compress_tool_result_content(
                        content,
                        &format!("{}.tool_result.content", block_path),
                        audit,
                        sketch_index,
                        jl_active,
                        max_text_chars_active,
                        caveman_enabled,
                        caveman_level,
                        rtk_enabled,
                        rtk_level,
                        json_aware_enabled,
                        json_aware_level,
                    )?;
                }
            }
        }

        Ok(serde_json::Value::Array(working_blocks))
    }

    fn compress_tool_result_content(
        &self,
        content: &serde_json::Value,
        path: &str,
        audit: &mut CompressionAudit,
        sketch_index: &mut crate::compression::jl::RequestSketchIndex,
        jl_active: bool,
        max_text_chars_active: usize,
        caveman_enabled: bool,
        caveman_level: &str,
        rtk_enabled: bool,
        rtk_level: &str,
        json_aware_enabled: bool,
        json_aware_level: &str,
    ) -> Result<serde_json::Value, String> {
        if let Some(s) = content.as_str() {
            let compressed = self.compress_text_with_dedupe(
                s,
                path,
                audit,
                sketch_index,
                jl_active,
                max_text_chars_active,
                caveman_enabled,
                caveman_level,
                rtk_enabled,
                rtk_level,
                json_aware_enabled,
                json_aware_level,
            )?;
            return Ok(serde_json::Value::String(compressed));
        }

        let mut arr = match content.as_array() {
            Some(a) => a.clone(),
            None => return Ok(content.clone()),
        };

        for (i, item) in arr.iter_mut().enumerate() {
            if item.is_object() && item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text_str) = item.get("text").and_then(|t| t.as_str()) {
                    let compressed = self.compress_text_with_dedupe(
                        text_str,
                        &format!("{}[{}].text", path, i),
                        audit,
                        sketch_index,
                        jl_active,
                        max_text_chars_active,
                        caveman_enabled,
                        caveman_level,
                        rtk_enabled,
                        rtk_level,
                        json_aware_enabled,
                        json_aware_level,
                    )?;
                    item.as_object_mut().unwrap().insert("text".to_string(), serde_json::Value::String(compressed));
                }
            }
        }
        Ok(serde_json::Value::Array(arr))
    }

    fn compress_text_with_dedupe(
        &self,
        text: &str,
        path: &str,
        audit: &mut CompressionAudit,
        sketch_index: &mut crate::compression::jl::RequestSketchIndex,
        jl_active: bool,
        max_text_chars_active: usize,
        caveman_enabled: bool,
        caveman_level: &str,
        rtk_enabled: bool,
        rtk_level: &str,
        json_aware_enabled: bool,
        json_aware_level: &str,
    ) -> Result<String, String> {
        let original_digest = sha256_short(text);

        if jl_active && text.chars().count() >= self.settings.jl_min_chars {
            let (best_record, best_score) = sketch_index.find_best(text);
            if let Some(record) = best_record {
                if best_score >= self.settings.jl_similarity_threshold {
                    let replacement = duplicate_marker(text, &record.path, best_score);
                    audit.events.push(self.make_event(
                        path,
                        "jl-near-duplicate",
                        text,
                        &replacement,
                        &original_digest,
                        &format!("matched {} ({})", record.path, record.digest),
                    ));
                    return Ok(replacement);
                }
            }
            sketch_index.add(text, path.to_string(), original_digest.clone());
        }

        let cache_key = self.build_cache_key(
            text,
            max_text_chars_active,
            caveman_enabled,
            caveman_level,
            rtk_enabled,
            rtk_level,
            json_aware_enabled,
            json_aware_level,
        );

        if let Some(cached) = self.result_cache.get(&cache_key) {
            audit.cache_hits += 1;
            if cached != text {
                audit.events.push(self.make_event(
                    path,
                    "cache-hit",
                    text,
                    &cached,
                    &original_digest,
                    "local-lru",
                ));
            }
            return Ok(cached);
        }
        audit.cache_misses += 1;

        let mut compressed = self.compress_text_middle_out(text, path, audit, max_text_chars_active, &original_digest);

        if json_aware_enabled {
            if let Ok((out, _)) = crate::compression::engines::compress_json_aware(&compressed, json_aware_level) {
                if out != compressed {
                    let digest = sha256_short(&compressed);
                    audit.events.push(self.make_event(
                        path,
                        "json-aware",
                        &compressed,
                        &out,
                        &digest,
                        &format!("level={}", json_aware_level),
                    ));
                    compressed = out;
                }
            }
        }

        if caveman_enabled {
            if let Ok(res) = crate::compression::engines::compress_caveman(&compressed, caveman_level) {
                if res != compressed {
                    let digest = sha256_short(&compressed);
                    audit.events.push(self.make_event(
                        path,
                        "caveman",
                        &compressed,
                        &res,
                        &digest,
                        &format!("level={}", caveman_level),
                    ));
                    compressed = res;
                }
            }
        }

        if rtk_enabled {
            if let Ok(res) = crate::compression::engines::compress_rtk(&compressed, rtk_level) {
                if res != compressed {
                    let digest = sha256_short(&compressed);
                    audit.events.push(self.make_event(
                        path,
                        "rtk",
                        &compressed,
                        &res,
                        &digest,
                        &format!("level={}", rtk_level),
                    ));
                    compressed = res;
                }
            }
        }

        self.result_cache.put(&cache_key, &compressed);
        Ok(compressed)
    }

    fn build_cache_key(
        &self,
        text: &str,
        max_text_chars_active: usize,
        caveman_enabled: bool,
        caveman_level: &str,
        rtk_enabled: bool,
        rtk_level: &str,
        json_aware_enabled: bool,
        json_aware_level: &str,
    ) -> String {
        let parts = vec![
            sha256_short(text),
            text.chars().count().to_string(),
            max_text_chars_active.to_string(),
            self.settings.min_omission_chars.to_string(),
            format!("{:.4}", self.settings.head_fraction),
            if caveman_enabled { "cav1" } else { "cav0" }.to_string(),
            caveman_level.to_string(),
            if rtk_enabled { "rtk1" } else { "rtk0" }.to_string(),
            rtk_level.to_string(),
            if json_aware_enabled { "ja1" } else { "ja0" }.to_string(),
            json_aware_level.to_string(),
        ];
        parts.join("|")
    }

    fn compress_text_middle_out(
        &self,
        text: &str,
        path: &str,
        audit: &mut CompressionAudit,
        max_chars: usize,
        digest: &str,
    ) -> String {
        let compressed = middle_out_text(
            text,
            max_chars,
            self.settings.min_omission_chars,
            self.settings.head_fraction,
        );
        if compressed != text {
            audit.events.push(self.make_event(
                path,
                "middle-out",
                text,
                &compressed,
                digest,
                "",
            ));
        }
        compressed
    }

    pub fn compress_response_payload(
        &self,
        payload: &serde_json::Value,
        endpoint: &str,
        force_enabled: bool,
    ) -> (serde_json::Value, CompressionAudit) {
        let mut audit = CompressionAudit::new(endpoint);
        if !force_enabled && !self.settings.output_compression_enabled {
            return (payload.clone(), audit);
        }

        let mut working = payload.clone();
        if let Some(content) = working.get_mut("content").and_then(|c| c.as_array_mut()) {
            for (i, block) in content.iter_mut().enumerate() {
                if block.is_object() && block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(text_str) = block.get("text").and_then(|t| t.as_str()) {
                        let compressed = middle_out_text(
                            text_str,
                            self.settings.output_max_text_chars,
                            self.settings.min_omission_chars,
                            self.settings.head_fraction,
                        );
                        if compressed != text_str {
                            let digest = sha256_short(text_str);
                            audit.events.push(self.make_event(
                                &format!("response.content[{}].text", i),
                                "middle-out-response",
                                text_str,
                                &compressed,
                                &digest,
                                "",
                            ));
                            block.as_object_mut().unwrap().insert("text".to_string(), serde_json::Value::String(compressed));
                        }
                    }
                }
            }
        }
        (working, audit)
    }

    pub fn compress_openai_responses_response_payload(
        &self,
        payload: &serde_json::Value,
        endpoint: &str,
        force_enabled: bool,
    ) -> (serde_json::Value, CompressionAudit) {
        let mut audit = CompressionAudit::new(endpoint);
        if !force_enabled && !self.settings.output_compression_enabled {
            return (payload.clone(), audit);
        }

        let working = self.compress_openai_output_value(payload, "response", &mut audit);
        (working, audit)
    }

    fn compress_openai_output_value(
        &self,
        value: &serde_json::Value,
        path: &str,
        audit: &mut CompressionAudit,
    ) -> serde_json::Value {
        if let Some(arr) = value.as_array() {
            return serde_json::Value::Array(
                arr.iter()
                    .enumerate()
                    .map(|(i, item)| self.compress_openai_output_value(item, &format!("{}[{}]", path, i), audit))
                    .collect(),
            );
        }

        let Some(obj) = value.as_object() else {
            return value.clone();
        };
        let mut out = obj.clone();

        let text_type = out.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let is_text_item = matches!(text_type, "output_text" | "text");
        if is_text_item {
            if let Some(text) = out.get("text").and_then(|v| v.as_str()) {
                let compressed = middle_out_text(
                    text,
                    self.settings.output_max_text_chars,
                    self.settings.min_omission_chars,
                    self.settings.head_fraction,
                );
                if compressed != text {
                    let digest = sha256_short(text);
                    audit.events.push(self.make_event(
                        &format!("{}.text", path),
                        "middle-out-response",
                        text,
                        &compressed,
                        &digest,
                        "",
                    ));
                    out.insert("text".to_string(), serde_json::Value::String(compressed));
                }
            }
        }

        for key in ["output", "content"] {
            if let Some(child) = out.get(key).cloned() {
                out.insert(
                    key.to_string(),
                    self.compress_openai_output_value(&child, &format!("{}.{}", path, key), audit),
                );
            }
        }

        serde_json::Value::Object(out)
    }

    fn make_event(
        &self,
        path: &str,
        mode: &str,
        original: &str,
        compressed: &str,
        digest: &str,
        note: &str,
    ) -> CompressionEvent {
        let (sample_before, sample_after) = if self.settings.log_text_samples {
            let sb: String = original.chars().take(500).collect();
            let sa: String = compressed.chars().take(500).collect();
            (Some(sb), Some(sa))
        } else {
            (None, None)
        };

        CompressionEvent {
            path: path.to_string(),
            mode: mode.to_string(),
            original_chars: original.chars().count(),
            compressed_chars: compressed.chars().count(),
            sha256: digest.to_string(),
            note: note.to_string(),
            sample_before,
            sample_after,
        }
    }
}
