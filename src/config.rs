use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

pub const BLOCKED_AUTH_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BEARER_TOKEN",
    "PROXY_ANTHROPIC_API_KEY",
    "PROXY_AUTH_MODE",
    "PROXY_FORCE_API_KEY",
    "CLAUDE_CODE_API_KEY_HELPER",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "AWS_BEDROCK_API_KEY",
    "AWS_BEARER_TOKEN_BEDROCK",
    "VERTEX_API_KEY",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // Local listener
    pub host: String,
    pub port: u16,
    pub reload: bool,

    // Upstream
    pub upstream_base_url: String,
    pub auth_mode: String,

    // Input Compaction
    pub input_compression_enabled: bool,
    pub max_text_chars: usize,
    pub min_omission_chars: usize,
    pub head_fraction: f64,
    pub compress_system: bool,
    pub compress_tool_results: bool,

    // JL Deduplication
    pub jl_dedupe_enabled: bool,
    pub jl_dims: usize,
    pub jl_shingle_tokens: usize,
    pub jl_similarity_threshold: f64,
    pub jl_min_chars: usize,

    // Output Compaction
    pub output_compression_enabled: bool,
    pub output_max_text_chars: usize,

    // Caveman Terse-Text Engine
    pub caveman_enabled: bool,
    pub caveman_level: String,

    // RTK Phrase Abbreviation
    pub rtk_enabled: bool,
    pub rtk_level: String,

    // JSON Aware Engine
    pub json_aware_enabled: bool,
    pub json_aware_level: String,

    // LSH Deduplication Engine
    pub lsh_enabled: bool,
    pub lsh_level: String,

    // Adaptive Policy
    pub adaptive_enabled: bool,

    // Cache Protection
    pub preserve_anthropic_cache: bool,
    pub auto_insert_cache_wall: bool,

    // L1 Cache
    pub l1_cache_enabled: bool,
    pub l1_cache_db_path: String,
    pub l1_cache_max_entries: usize,
    pub l1_cache_max_body_bytes: usize,

    // L2 Semantic Cache
    pub l2_cache_enabled: bool,
    pub l2_similarity_threshold: f64,
    pub l2_backend: String,
    pub l2_max_entries: usize,
    pub l2_qdrant_url: String,
    pub l2_qdrant_collection: String,
    pub l2_qdrant_api_key: String,
    pub l2_embedder: String,
    pub l2_embedding_dim: usize,
    pub l2_openai_model: String,

    // Local Compression Cache
    pub compression_cache_enabled: bool,
    pub compression_cache_size: usize,

    // Observability & Auditing
    pub audit_enabled: bool,
    pub audit_log_dir: PathBuf,
    pub log_text_samples: bool,
    pub log_json: bool,
    pub timeseries_minutes: u64,
    pub recent_max: usize,

    // Rate Limiter
    pub rate_limit_enabled: bool,
    pub rate_limit_capacity: usize,
    pub rate_limit_refill_per_second: f64,

    // Timeouts
    pub timeout_connect_s: f64,
    pub timeout_read_s: f64,
    pub default_anthropic_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            host: "127.0.0.1".to_string(),
            port: 8787,
            reload: false,
            upstream_base_url: "https://api.anthropic.com".to_string(),
            auth_mode: "subscription_oauth_passthrough_only".to_string(),
            input_compression_enabled: true,
            max_text_chars: 12000,
            min_omission_chars: 1500,
            head_fraction: 0.55,
            compress_system: false,
            compress_tool_results: true,
            jl_dedupe_enabled: true,
            jl_dims: 512,
            jl_shingle_tokens: 5,
            jl_similarity_threshold: 0.985,
            jl_min_chars: 4000,
            output_compression_enabled: false,
            output_max_text_chars: 20000,
            caveman_enabled: false,
            caveman_level: "standard".to_string(),
            rtk_enabled: false,
            rtk_level: "minimal".to_string(),
            json_aware_enabled: false,
            json_aware_level: "safe".to_string(),
            lsh_enabled: false,
            lsh_level: "standard".to_string(),
            adaptive_enabled: false,
            preserve_anthropic_cache: true,
            auto_insert_cache_wall: true,
            l1_cache_enabled: false,
            l1_cache_db_path: ":memory:".to_string(),
            l1_cache_max_entries: 10000,
            l1_cache_max_body_bytes: 5 * 1024 * 1024,
            l2_cache_enabled: false,
            l2_similarity_threshold: 0.97,
            l2_backend: "in_memory".to_string(),
            l2_max_entries: 10000,
            l2_qdrant_url: "".to_string(),
            l2_qdrant_collection: "brain_proxy_l2".to_string(),
            l2_qdrant_api_key: "".to_string(),
            l2_embedder: "hash".to_string(),
            l2_embedding_dim: 3072,
            l2_openai_model: "text-embedding-3-large".to_string(),
            compression_cache_enabled: true,
            compression_cache_size: 256,
            audit_enabled: true,
            audit_log_dir: PathBuf::from(".middleout-logs"),
            log_text_samples: false,
            log_json: false,
            timeseries_minutes: 60,
            recent_max: 200,
            rate_limit_enabled: false,
            rate_limit_capacity: 120,
            rate_limit_refill_per_second: 2.0,
            timeout_connect_s: 30.0,
            timeout_read_s: 600.0,
            default_anthropic_version: "2023-06-01".to_string(),
        }
    }
}

// Flat structure representing the raw TOML deserialized layout
#[derive(Debug, Deserialize)]
struct RawTomlConfig {
    server: Option<RawServerSection>,
    compression: Option<RawCompressionSection>,
    jl: Option<RawJlSection>,
    caveman: Option<RawCavemanSection>,
    rtk: Option<RawRtkSection>,
    json_aware: Option<RawJsonAwareSection>,
    lsh: Option<RawLshSection>,
    adaptive: Option<RawAdaptiveSection>,
    l1_cache: Option<RawL1CacheSection>,
    l2_cache: Option<RawL2CacheSection>,
    audit: Option<RawAuditSection>,
}

#[derive(Debug, Deserialize)]
struct RawServerSection {
    host: Option<String>,
    port: Option<u16>,
    reload: Option<bool>,
    upstream_base_url: Option<String>,
    connect_timeout_s: Option<f64>,
    read_timeout_s: Option<f64>,
    anthropic_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCompressionSection {
    input_enabled: Option<bool>,
    output_enabled: Option<bool>,
    max_text_chars: Option<usize>,
    output_max_text_chars: Option<usize>,
    min_omission_chars: Option<usize>,
    head_fraction: Option<f64>,
    compress_system: Option<bool>,
    compress_tool_results: Option<bool>,
    preserve_anthropic_cache: Option<bool>,
    cache_enabled: Option<bool>,
    cache_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawJlSection {
    enabled: Option<bool>,
    dims: Option<usize>,
    shingle_tokens: Option<usize>,
    similarity_threshold: Option<f64>,
    min_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawCavemanSection {
    enabled: Option<bool>,
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRtkSection {
    enabled: Option<bool>,
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawJsonAwareSection {
    enabled: Option<bool>,
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawLshSection {
    enabled: Option<bool>,
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAdaptiveSection {
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawL1CacheSection {
    enabled: Option<bool>,
    db_path: Option<String>,
    max_entries: Option<usize>,
    max_body_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawL2CacheSection {
    enabled: Option<bool>,
    similarity_threshold: Option<f64>,
    backend: Option<String>,
    max_entries: Option<usize>,
    qdrant_url: Option<String>,
    qdrant_collection: Option<String>,
    qdrant_api_key: Option<String>,
    embedder: Option<String>,
    embedding_dim: Option<usize>,
    openai_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAuditSection {
    enabled: Option<bool>,
    log_dir: Option<String>,
    log_text_samples: Option<bool>,
    log_json: Option<bool>,
    timeseries_minutes: Option<u64>,
    recent_max: Option<usize>,
}

fn load_toml_config() -> HashMap<String, String> {
    let mut flat_toml = HashMap::new();
    
    // Find the config file
    let explicit = env::var("MIDDLEOUT_CONFIG").ok();
    let mut candidates = Vec::new();
    if let Some(ref path_str) = explicit {
        candidates.push(PathBuf::from(path_str));
    }
    candidates.push(PathBuf::from("middleout.toml"));
    if let Some(home) = env::var("HOME").ok().map(PathBuf::from) {
        candidates.push(home.join(".config/middleout/middleout.toml"));
    }

    for path in candidates {
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(parsed) = toml::from_str::<RawTomlConfig>(&content) {
                    flatten_toml_config(parsed, &mut flat_toml);
                    break;
                }
            }
        }
    }
    flat_toml
}

fn flatten_toml_config(raw: RawTomlConfig, out: &mut HashMap<String, String>) {
    if let Some(sec) = raw.server {
        if let Some(v) = sec.host { out.insert("host".to_string(), v); }
        if let Some(v) = sec.port { out.insert("port".to_string(), v.to_string()); }
        if let Some(v) = sec.reload { out.insert("reload".to_string(), v.to_string()); }
        if let Some(v) = sec.upstream_base_url { out.insert("upstream_base_url".to_string(), v); }
        if let Some(v) = sec.connect_timeout_s { out.insert("timeout_connect_s".to_string(), v.to_string()); }
        if let Some(v) = sec.read_timeout_s { out.insert("timeout_read_s".to_string(), v.to_string()); }
        if let Some(v) = sec.anthropic_version { out.insert("default_anthropic_version".to_string(), v); }
    }
    if let Some(sec) = raw.compression {
        if let Some(v) = sec.input_enabled { out.insert("input_compression_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.output_enabled { out.insert("output_compression_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.max_text_chars { out.insert("max_text_chars".to_string(), v.to_string()); }
        if let Some(v) = sec.output_max_text_chars { out.insert("output_max_text_chars".to_string(), v.to_string()); }
        if let Some(v) = sec.min_omission_chars { out.insert("min_omission_chars".to_string(), v.to_string()); }
        if let Some(v) = sec.head_fraction { out.insert("head_fraction".to_string(), v.to_string()); }
        if let Some(v) = sec.compress_system { out.insert("compress_system".to_string(), v.to_string()); }
        if let Some(v) = sec.compress_tool_results { out.insert("compress_tool_results".to_string(), v.to_string()); }
        if let Some(v) = sec.preserve_anthropic_cache { out.insert("preserve_anthropic_cache".to_string(), v.to_string()); }
        if let Some(v) = sec.cache_enabled { out.insert("compression_cache_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.cache_size { out.insert("compression_cache_size".to_string(), v.to_string()); }
    }
    if let Some(sec) = raw.jl {
        if let Some(v) = sec.enabled { out.insert("jl_dedupe_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.dims { out.insert("jl_dims".to_string(), v.to_string()); }
        if let Some(v) = sec.shingle_tokens { out.insert("jl_shingle_tokens".to_string(), v.to_string()); }
        if let Some(v) = sec.similarity_threshold { out.insert("jl_similarity_threshold".to_string(), v.to_string()); }
        if let Some(v) = sec.min_chars { out.insert("jl_min_chars".to_string(), v.to_string()); }
    }
    if let Some(sec) = raw.caveman {
        if let Some(v) = sec.enabled { out.insert("caveman_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.level { out.insert("caveman_level".to_string(), v); }
    }
    if let Some(sec) = raw.rtk {
        if let Some(v) = sec.enabled { out.insert("rtk_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.level { out.insert("rtk_level".to_string(), v); }
    }
    if let Some(sec) = raw.json_aware {
        if let Some(v) = sec.enabled { out.insert("json_aware_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.level { out.insert("json_aware_level".to_string(), v); }
    }
    if let Some(sec) = raw.lsh {
        if let Some(v) = sec.enabled { out.insert("lsh_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.level { out.insert("lsh_level".to_string(), v); }
    }
    if let Some(sec) = raw.adaptive {
        if let Some(v) = sec.enabled { out.insert("adaptive_enabled".to_string(), v.to_string()); }
    }
    if let Some(sec) = raw.l1_cache {
        if let Some(v) = sec.enabled { out.insert("l1_cache_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.db_path { out.insert("l1_cache_db_path".to_string(), v); }
        if let Some(v) = sec.max_entries { out.insert("l1_cache_max_entries".to_string(), v.to_string()); }
        if let Some(v) = sec.max_body_bytes { out.insert("l1_cache_max_body_bytes".to_string(), v.to_string()); }
    }
    if let Some(sec) = raw.l2_cache {
        if let Some(v) = sec.enabled { out.insert("l2_cache_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.similarity_threshold { out.insert("l2_similarity_threshold".to_string(), v.to_string()); }
        if let Some(v) = sec.backend { out.insert("l2_backend".to_string(), v); }
        if let Some(v) = sec.max_entries { out.insert("l2_max_entries".to_string(), v.to_string()); }
        if let Some(v) = sec.qdrant_url { out.insert("l2_qdrant_url".to_string(), v); }
        if let Some(v) = sec.qdrant_collection { out.insert("l2_qdrant_collection".to_string(), v); }
        if let Some(v) = sec.qdrant_api_key { out.insert("l2_qdrant_api_key".to_string(), v); }
        if let Some(v) = sec.embedder { out.insert("l2_embedder".to_string(), v); }
        if let Some(v) = sec.embedding_dim { out.insert("l2_embedding_dim".to_string(), v.to_string()); }
        if let Some(v) = sec.openai_model { out.insert("l2_openai_model".to_string(), v); }
    }
    if let Some(sec) = raw.audit {
        if let Some(v) = sec.enabled { out.insert("audit_enabled".to_string(), v.to_string()); }
        if let Some(v) = sec.log_dir { out.insert("audit_log_dir".to_string(), v); }
        if let Some(v) = sec.log_text_samples { out.insert("log_text_samples".to_string(), v.to_string()); }
        if let Some(v) = sec.log_json { out.insert("log_json".to_string(), v.to_string()); }
        if let Some(v) = sec.timeseries_minutes { out.insert("timeseries_minutes".to_string(), v.to_string()); }
        if let Some(v) = sec.recent_max { out.insert("recent_max".to_string(), v.to_string()); }
    }
}

fn get_env_or_toml_or_default<T>(
    env_name: &str,
    toml_name: &str,
    toml_map: &HashMap<String, String>,
    default: T,
    parser: fn(&str) -> Option<T>,
) -> T {
    if let Some(env_val) = env::var(env_name).ok() {
        if let Some(v) = parser(&env_val) {
            return v;
        }
    }
    if let Some(toml_val) = toml_map.get(toml_name) {
        if let Some(v) = parser(toml_val) {
            return v;
        }
    }
    default
}

pub fn load_settings() -> Result<Settings, String> {
    // Check for blocked environment variables first (Fortress strict check)
    let mut blocked = Vec::new();
    for &env_var in BLOCKED_AUTH_ENV_VARS {
        if env::var(env_var).is_ok() {
            blocked.push(env_var);
        }
    }
    if !blocked.is_empty() {
        return Err(format!(
            "Strict subscription-only mode refuses proxy-side auth environment variables. \
             Unset these before starting middleout-proxy: {}",
            blocked.join(", ")
        ));
    }

    let toml_map = load_toml_config();
    let def = Settings::default();

    let parse_str = |s: &str| Some(s.to_string());
    let parse_bool = |s: &str| match s.trim().to_lowercase().as_str() {
        "1" | "true" | "t" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "f" | "no" | "n" | "off" | "" => Some(false),
        _ => None,
    };
    let parse_u16 = |s: &str| s.parse::<u16>().ok();
    let parse_usize = |s: &str| s.parse::<usize>().ok();
    let parse_u64 = |s: &str| s.parse::<u64>().ok();
    let parse_f64 = |s: &str| s.parse::<f64>().ok();
    let parse_path = |s: &str| Some(PathBuf::from(s));

    let host = get_env_or_toml_or_default("MIDDLEOUT_HOST", "host", &toml_map, def.host, parse_str);
    let port = get_env_or_toml_or_default("MIDDLEOUT_PORT", "port", &toml_map, def.port, parse_u16);
    let reload = get_env_or_toml_or_default("MIDDLEOUT_RELOAD", "reload", &toml_map, def.reload, parse_bool);
    let upstream_base_url = get_env_or_toml_or_default("PROXY_UPSTREAM_BASE_URL", "upstream_base_url", &toml_map, def.upstream_base_url, parse_str);
    let input_compression_enabled = get_env_or_toml_or_default("MIDDLEOUT_INPUT_COMPRESSION", "input_compression_enabled", &toml_map, def.input_compression_enabled, parse_bool);
    let max_text_chars = get_env_or_toml_or_default("MIDDLEOUT_MAX_TEXT_CHARS", "max_text_chars", &toml_map, def.max_text_chars, parse_usize);
    let min_omission_chars = get_env_or_toml_or_default("MIDDLEOUT_MIN_OMISSION_CHARS", "min_omission_chars", &toml_map, def.min_omission_chars, parse_usize);
    let head_fraction = get_env_or_toml_or_default("MIDDLEOUT_HEAD_FRACTION", "head_fraction", &toml_map, def.head_fraction, parse_f64);
    let compress_system = get_env_or_toml_or_default("MIDDLEOUT_COMPRESS_SYSTEM", "compress_system", &toml_map, def.compress_system, parse_bool);
    let compress_tool_results = get_env_or_toml_or_default("MIDDLEOUT_COMPRESS_TOOL_RESULTS", "compress_tool_results", &toml_map, def.compress_tool_results, parse_bool);
    let jl_dedupe_enabled = get_env_or_toml_or_default("MIDDLEOUT_JL_DEDUPE", "jl_dedupe_enabled", &toml_map, def.jl_dedupe_enabled, parse_bool);
    let jl_dims = get_env_or_toml_or_default("MIDDLEOUT_JL_DIMS", "jl_dims", &toml_map, def.jl_dims, parse_usize);
    let jl_shingle_tokens = get_env_or_toml_or_default("MIDDLEOUT_JL_SHINGLE_TOKENS", "jl_shingle_tokens", &toml_map, def.jl_shingle_tokens, parse_usize);
    let jl_similarity_threshold = get_env_or_toml_or_default("MIDDLEOUT_JL_SIMILARITY", "jl_similarity_threshold", &toml_map, def.jl_similarity_threshold, parse_f64);
    let jl_min_chars = get_env_or_toml_or_default("MIDDLEOUT_JL_MIN_CHARS", "jl_min_chars", &toml_map, def.jl_min_chars, parse_usize);
    let output_compression_enabled = get_env_or_toml_or_default("MIDDLEOUT_OUTPUT_COMPRESSION", "output_compression_enabled", &toml_map, def.output_compression_enabled, parse_bool);
    let output_max_text_chars = get_env_or_toml_or_default("MIDDLEOUT_OUTPUT_MAX_TEXT_CHARS", "output_max_text_chars", &toml_map, def.output_max_text_chars, parse_usize);
    let caveman_enabled = get_env_or_toml_or_default("MIDDLEOUT_CAVEMAN", "caveman_enabled", &toml_map, def.caveman_enabled, parse_bool);
    let caveman_level = get_env_or_toml_or_default("MIDDLEOUT_CAVEMAN_LEVEL", "caveman_level", &toml_map, def.caveman_level, parse_str);
    let rtk_enabled = get_env_or_toml_or_default("MIDDLEOUT_RTK", "rtk_enabled", &toml_map, def.rtk_enabled, parse_bool);
    let rtk_level = get_env_or_toml_or_default("MIDDLEOUT_RTK_LEVEL", "rtk_level", &toml_map, def.rtk_level, parse_str);
    let json_aware_enabled = get_env_or_toml_or_default("MIDDLEOUT_JSON_AWARE", "json_aware_enabled", &toml_map, def.json_aware_enabled, parse_bool);
    let json_aware_level = get_env_or_toml_or_default("MIDDLEOUT_JSON_AWARE_LEVEL", "json_aware_level", &toml_map, def.json_aware_level, parse_str);
    let lsh_enabled = get_env_or_toml_or_default("MIDDLEOUT_LSH", "lsh_enabled", &toml_map, def.lsh_enabled, parse_bool);
    let lsh_level = get_env_or_toml_or_default("MIDDLEOUT_LSH_LEVEL", "lsh_level", &toml_map, def.lsh_level, parse_str);
    let adaptive_enabled = get_env_or_toml_or_default("MIDDLEOUT_ADAPTIVE", "adaptive_enabled", &toml_map, def.adaptive_enabled, parse_bool);
    let preserve_anthropic_cache = get_env_or_toml_or_default("MIDDLEOUT_PRESERVE_ANTHROPIC_CACHE", "preserve_anthropic_cache", &toml_map, def.preserve_anthropic_cache, parse_bool);
    let auto_insert_cache_wall = get_env_or_toml_or_default("BRAIN_AUTO_INSERT_WALL", "auto_insert_cache_wall", &toml_map, def.auto_insert_cache_wall, parse_bool);
    let l1_cache_enabled = get_env_or_toml_or_default("BRAIN_L1_CACHE_ENABLED", "l1_cache_enabled", &toml_map, def.l1_cache_enabled, parse_bool);
    let l1_cache_db_path = get_env_or_toml_or_default("BRAIN_L1_CACHE_DB", "l1_cache_db_path", &toml_map, def.l1_cache_db_path, parse_str);
    let l1_cache_max_entries = get_env_or_toml_or_default("BRAIN_L1_CACHE_MAX_ENTRIES", "l1_cache_max_entries", &toml_map, def.l1_cache_max_entries, parse_usize);
    let l1_cache_max_body_bytes = get_env_or_toml_or_default("BRAIN_L1_CACHE_MAX_BODY_BYTES", "l1_cache_max_body_bytes", &toml_map, def.l1_cache_max_body_bytes, parse_usize);
    let l2_cache_enabled = get_env_or_toml_or_default("BRAIN_L2_CACHE_ENABLED", "l2_cache_enabled", &toml_map, def.l2_cache_enabled, parse_bool);
    let l2_similarity_threshold = get_env_or_toml_or_default("BRAIN_L2_SIMILARITY", "l2_similarity_threshold", &toml_map, def.l2_similarity_threshold, parse_f64);
    let l2_backend = get_env_or_toml_or_default("BRAIN_L2_BACKEND", "l2_backend", &toml_map, def.l2_backend, parse_str);
    let l2_max_entries = get_env_or_toml_or_default("BRAIN_L2_MAX_ENTRIES", "l2_max_entries", &toml_map, def.l2_max_entries, parse_usize);
    let l2_qdrant_url = get_env_or_toml_or_default("BRAIN_L2_QDRANT_URL", "l2_qdrant_url", &toml_map, def.l2_qdrant_url, parse_str);
    let l2_qdrant_collection = get_env_or_toml_or_default("BRAIN_L2_QDRANT_COLLECTION", "l2_qdrant_collection", &toml_map, def.l2_qdrant_collection, parse_str);
    let l2_qdrant_api_key = get_env_or_toml_or_default("BRAIN_L2_QDRANT_API_KEY", "l2_qdrant_api_key", &toml_map, def.l2_qdrant_api_key, parse_str);
    let l2_embedder = get_env_or_toml_or_default("BRAIN_L2_EMBEDDER", "l2_embedder", &toml_map, def.l2_embedder, parse_str);
    let l2_embedding_dim = get_env_or_toml_or_default("BRAIN_L2_EMBEDDING_DIM", "l2_embedding_dim", &toml_map, def.l2_embedding_dim, parse_usize);
    let l2_openai_model = get_env_or_toml_or_default("BRAIN_L2_OPENAI_MODEL", "l2_openai_model", &toml_map, def.l2_openai_model, parse_str);
    let compression_cache_enabled = get_env_or_toml_or_default("MIDDLEOUT_COMPRESSION_CACHE", "compression_cache_enabled", &toml_map, def.compression_cache_enabled, parse_bool);
    let compression_cache_size = get_env_or_toml_or_default("MIDDLEOUT_COMPRESSION_CACHE_SIZE", "compression_cache_size", &toml_map, def.compression_cache_size, parse_usize);
    let audit_enabled = get_env_or_toml_or_default("MIDDLEOUT_AUDIT", "audit_enabled", &toml_map, def.audit_enabled, parse_bool);
    let raw_audit_dir = get_env_or_toml_or_default("MIDDLEOUT_AUDIT_DIR", "audit_log_dir", &toml_map, def.audit_log_dir, parse_path);
    
    // Resolve absolute path for audit log directory
    let audit_log_dir = if raw_audit_dir.is_absolute() {
        raw_audit_dir
    } else {
        Path::new(&raw_audit_dir).to_path_buf()
    };

    let log_text_samples = get_env_or_toml_or_default("MIDDLEOUT_LOG_TEXT_SAMPLES", "log_text_samples", &toml_map, def.log_text_samples, parse_bool);
    let log_json = get_env_or_toml_or_default("MIDDLEOUT_LOG_JSON", "log_json", &toml_map, def.log_json, parse_bool);
    let timeseries_minutes = get_env_or_toml_or_default("MIDDLEOUT_TIMESERIES_MINUTES", "timeseries_minutes", &toml_map, def.timeseries_minutes, parse_u64);
    let recent_max = get_env_or_toml_or_default("MIDDLEOUT_RECENT_MAX", "recent_max", &toml_map, def.recent_max, parse_usize);
    let rate_limit_enabled = get_env_or_toml_or_default("BRAIN_RATE_LIMIT_ENABLED", "rate_limit_enabled", &toml_map, def.rate_limit_enabled, parse_bool);
    let rate_limit_capacity = get_env_or_toml_or_default("BRAIN_RATE_LIMIT_CAPACITY", "rate_limit_capacity", &toml_map, def.rate_limit_capacity, parse_usize);
    let rate_limit_refill_per_second = get_env_or_toml_or_default("BRAIN_RATE_LIMIT_REFILL_PER_SECOND", "rate_limit_refill_per_second", &toml_map, def.rate_limit_refill_per_second, parse_f64);
    let timeout_connect_s = get_env_or_toml_or_default("MIDDLEOUT_CONNECT_TIMEOUT_S", "timeout_connect_s", &toml_map, def.timeout_connect_s, parse_f64);
    let timeout_read_s = get_env_or_toml_or_default("MIDDLEOUT_READ_TIMEOUT_S", "timeout_read_s", &toml_map, def.timeout_read_s, parse_f64);
    let default_anthropic_version = get_env_or_toml_or_default("MIDDLEOUT_ANTHROPIC_VERSION", "anthropic_version", &toml_map, def.default_anthropic_version, parse_str);

    // Validation checks
    if max_text_chars < 512 {
        return Err("MIDDLEOUT_MAX_TEXT_CHARS must be at least 512".to_string());
    }
    if !(0.05..=0.95).contains(&head_fraction) {
        return Err("MIDDLEOUT_HEAD_FRACTION must be between 0.05 and 0.95".to_string());
    }
    if jl_dims < 16 {
        return Err("MIDDLEOUT_JL_DIMS must be at least 16".to_string());
    }
    match caveman_level.as_str() {
        "lite" | "standard" | "aggressive" | "ultra" => {}
        _ => return Err(format!("MIDDLEOUT_CAVEMAN_LEVEL must be one of lite/standard/aggressive/ultra, got {:?}", caveman_level)),
    }
    match rtk_level.as_str() {
        "minimal" | "standard" | "aggressive" => {}
        _ => return Err(format!("MIDDLEOUT_RTK_LEVEL must be one of minimal/standard/aggressive, got {:?}", rtk_level)),
    }
    match json_aware_level.as_str() {
        "safe" | "standard" | "aggressive" => {}
        _ => return Err(format!("MIDDLEOUT_JSON_AWARE_LEVEL must be one of safe/standard/aggressive, got {:?}", json_aware_level)),
    }
    match lsh_level.as_str() {
        "conservative" | "standard" | "aggressive" => {}
        _ => return Err(format!("MIDDLEOUT_LSH_LEVEL must be one of conservative/standard/aggressive, got {:?}", lsh_level)),
    }
    match l2_backend.as_str() {
        "in_memory" | "qdrant" => {}
        _ => return Err(format!("BRAIN_L2_BACKEND must be one of in_memory/qdrant, got {:?}", l2_backend)),
    }
    match l2_embedder.as_str() {
        "hash" | "openai" => {}
        _ => return Err(format!("BRAIN_L2_EMBEDDER must be one of hash/openai, got {:?}", l2_embedder)),
    }
    if l2_embedding_dim < 16 {
        return Err("BRAIN_L2_EMBEDDING_DIM must be >= 16".to_string());
    }

    Ok(Settings {
        host,
        port,
        reload,
        upstream_base_url,
        auth_mode: def.auth_mode,
        input_compression_enabled,
        max_text_chars,
        min_omission_chars,
        head_fraction,
        compress_system,
        compress_tool_results,
        jl_dedupe_enabled,
        jl_dims,
        jl_shingle_tokens,
        jl_similarity_threshold,
        jl_min_chars,
        output_compression_enabled,
        output_max_text_chars,
        caveman_enabled,
        caveman_level,
        rtk_enabled,
        rtk_level,
        json_aware_enabled,
        json_aware_level,
        lsh_enabled,
        lsh_level,
        adaptive_enabled,
        preserve_anthropic_cache,
        auto_insert_cache_wall,
        l1_cache_enabled,
        l1_cache_db_path,
        l1_cache_max_entries,
        l1_cache_max_body_bytes,
        l2_cache_enabled,
        l2_similarity_threshold,
        l2_backend,
        l2_max_entries,
        l2_qdrant_url,
        l2_qdrant_collection,
        l2_qdrant_api_key,
        l2_embedder,
        l2_embedding_dim,
        l2_openai_model,
        compression_cache_enabled,
        compression_cache_size,
        audit_enabled,
        audit_log_dir,
        log_text_samples,
        log_json,
        timeseries_minutes,
        recent_max,
        rate_limit_enabled,
        rate_limit_capacity,
        rate_limit_refill_per_second,
        timeout_connect_s,
        timeout_read_s,
        default_anthropic_version,
    })
}
