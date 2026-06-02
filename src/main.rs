pub mod config;
pub mod compression;
pub mod sim;
pub mod cache;
pub mod cost;
pub mod audit;
pub mod rate_limit;
pub mod server;

use std::sync::Arc;

#[tokio::main]
async fn main() {
    // Initialize tracing subscriber
    tracing_subscriber::fmt::init();

    // Load config settings
    let settings = match crate::config::load_settings() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Configuration Load Error: {}", err);
            std::process::exit(1);
        }
    };

    // Initialize L1 Exact-Match SQLite cache
    let l1_cache = if settings.l1_cache_enabled {
        match crate::cache::l1::L1Cache::new(
            &settings.l1_cache_db_path,
            settings.l1_cache_max_entries,
            settings.l1_cache_max_body_bytes,
        ) {
            Ok(c) => Some(c),
            Err(err) => {
                eprintln!("L1 Cache Setup Error: {}", err);
                None
            }
        }
    } else {
        None
    };

    // Initialize L2 Semantic Vector Cache Embedder
    let embedder: Option<Arc<dyn crate::cache::embedders::EmbeddingClient>> = if settings.l2_cache_enabled {
        match settings.l2_embedder.as_str() {
            "openai" => {
                if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
                    Some(Arc::new(crate::cache::embedders::OpenAIEmbeddingClient::new(
                        settings.l2_openai_model.clone(),
                        settings.l2_embedding_dim,
                        api_key,
                        settings.timeout_read_s as u64,
                        None,
                    )))
                } else {
                    eprintln!("L2 OpenAI embedder configured but OPENAI_API_KEY is not set!");
                    None
                }
            }
            _ => {
                Some(Arc::new(crate::cache::embedders::HashEmbedder::new(settings.l2_embedding_dim, 4)))
            }
        }
    } else {
        None
    };

    // Initialize L2 Vector Store
    let vector_store: Option<Arc<dyn crate::cache::l2::VectorStore>> = if settings.l2_cache_enabled {
        match settings.l2_backend.as_str() {
            "qdrant" => {
                let api_key = std::env::var("QDRANT_API_KEY").ok();
                Some(Arc::new(crate::cache::l2::QdrantVectorStore::new(
                    settings.l2_qdrant_url.clone(),
                    settings.l2_qdrant_collection.clone(),
                    settings.l2_embedding_dim,
                    api_key,
                    settings.timeout_read_s as u64,
                )))
            }
            _ => {
                Some(Arc::new(crate::cache::l2::InMemoryVectorStore::new(settings.l2_max_entries)))
            }
        }
    } else {
        None
    };

    // Initialize L2 Cache wrapper with exact-match verification enabled
    let l2_cache = crate::cache::l2::L2Cache::new(
        embedder,
        vector_store,
        settings.l2_similarity_threshold,
        settings.l2_cache_enabled,
        true,
    ).expect("Failed to initialize L2 Semantic Vector Cache");

    // Initialize server state components
    let compressor = crate::compression::PayloadCompressor::new(settings.clone());
    let rate_limiter = crate::rate_limit::RequestLimiter::new(
        settings.rate_limit_capacity as i64,
        settings.rate_limit_refill_per_second,
        settings.l2_max_entries, // using a reasonable max clients bound
    );
    let cost_tracker = Arc::new(crate::cost::CostTracker::new());
    let usage_budget = crate::cost::UsageBudget::new(None, None);
    let audit_logger = Arc::new(crate::audit::AuditLogger::new(&settings));
    let policy_router = crate::server::policies::PolicyRouter::from_env();
    let runtime_settings = tokio::sync::RwLock::new(crate::server::default_runtime_settings(&settings));
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(settings.timeout_read_s as u64))
        .build()
        .expect("Failed to build reqwest HTTP client");

    let shared_state = Arc::new(crate::server::ServerState {
        settings: settings.clone(),
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
    });

    // Create Axum router and listener
    let router = crate::server::create_router(shared_state);
    let addr = format!("{}:{}", settings.host, settings.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(err) => {
            eprintln!("Failed to bind tcp socket listener on {}: {}", addr, err);
            std::process::exit(1);
        }
    };

    println!("MiddleOut Claude Proxy running on http://{}", addr);
    if let Err(err) = axum::serve(listener, router).await {
        eprintln!("Fatal server execution error: {}", err);
    }
}
