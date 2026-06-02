use std::collections::{HashMap, BTreeMap};
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;
use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::Value;

use crate::cache::l1::CachedResponse;
use crate::cache::embedders::EmbeddingClient;

#[derive(Debug, Clone)]
pub struct SemanticHit {
    pub similarity: f64,
    pub response: CachedResponse,
    pub point_id: String,
    pub metadata: Value,
}

pub trait VectorStore: Send + Sync {
    fn upsert<'a>(&'a self, point_id: &'a str, vector: Vec<f64>, payload: Value) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
    fn search<'a>(&'a self, vector: Vec<f64>, top_k: usize) -> Pin<Box<dyn Future<Output = Result<Vec<(String, f64, Value)>, String>> + Send + 'a>>;
    fn delete<'a>(&'a self, point_id: &'a str) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
    fn clear(&self) -> Pin<Box<dyn Future<Output = Result<usize, String>> + Send + '_>>;
    fn stats(&self) -> Pin<Box<dyn Future<Output = Value> + Send + '_>>;
}

pub struct InMemoryVectorStore {
    max_entries: usize,
    points: RwLock<BTreeMap<String, (Vec<f64>, Value, u64)>>,
    access_counter: std::sync::atomic::AtomicU64,
}

impl InMemoryVectorStore {
    pub fn new(max_entries: usize) -> Self {
        InMemoryVectorStore {
            max_entries: if max_entries < 1 { 10000 } else { max_entries },
            points: RwLock::new(BTreeMap::new()),
            access_counter: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut a_norm = 0.0;
    let mut b_norm = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        a_norm += x * x;
        b_norm += y * y;
    }
    if a_norm == 0.0 || b_norm == 0.0 {
        return 0.0;
    }
    dot / (a_norm.sqrt() * b_norm.sqrt())
}

impl VectorStore for InMemoryVectorStore {
    fn upsert<'a>(&'a self, point_id: &'a str, vector: Vec<f64>, payload: Value) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let point_id = point_id.to_string();
        Box::pin(async move {
            let mut points = self.points.write();
            let counter = self.access_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            points.insert(point_id, (vector, payload, counter));

            while points.len() > self.max_entries {
                let to_remove = points.iter()
                    .min_by_key(|(_, (_, _, cnt))| *cnt)
                    .map(|(k, _)| k.clone());
                if let Some(k) = to_remove {
                    points.remove(&k);
                } else {
                    break;
                }
            }
            Ok(())
        })
    }

    fn search<'a>(&'a self, vector: Vec<f64>, top_k: usize) -> Pin<Box<dyn Future<Output = Result<Vec<(String, f64, Value)>, String>> + Send + 'a>> {
        Box::pin(async move {
            let points = self.points.read();
            let mut scored = Vec::new();
            for (pid, (v, payload, _)) in points.iter() {
                let sim = cosine_similarity(&vector, v);
                scored.push((pid.clone(), sim, payload.clone()));
            }

            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let results: Vec<(String, f64, Value)> = scored.into_iter().take(top_k).collect();

            if !results.is_empty() {
                drop(points);
                let mut points_write = self.points.write();
                let top_pid = &results[0].0;
                if let Some((_, _, cnt)) = points_write.get_mut(top_pid) {
                    *cnt = self.access_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            Ok(results)
        })
    }

    fn delete<'a>(&'a self, point_id: &'a str) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let point_id = point_id.to_string();
        Box::pin(async move {
            let mut points = self.points.write();
            points.remove(&point_id);
            Ok(())
        })
    }

    fn clear(&self) -> Pin<Box<dyn Future<Output = Result<usize, String>> + Send + '_>> {
        Box::pin(async move {
            let mut points = self.points.write();
            let count = points.len();
            points.clear();
            Ok(count)
        })
    }

    fn stats(&self) -> Pin<Box<dyn Future<Output = Value> + Send + '_>> {
        Box::pin(async move {
            let points = self.points.read();
            serde_json::json!({
                "backend": "in_memory",
                "entries": points.len(),
                "max_entries": self.max_entries,
            })
        })
    }
}

pub struct QdrantVectorStore {
    client: reqwest::Client,
    url: String,
    collection: String,
    dim: usize,
    api_key: Option<String>,
}

impl QdrantVectorStore {
    pub fn new(url: String, collection: String, dim: usize, api_key: Option<String>, timeout_s: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_s))
            .build()
            .unwrap_or_default();
        
        QdrantVectorStore {
            client,
            url,
            collection,
            dim,
            api_key,
        }
    }
}

fn coerce_qdrant_id(point_id: &str) -> String {
    if point_id.len() >= 32 {
        let p = &point_id[..32];
        format!(
            "{}-{}-{}-{}-{}",
            &p[0..8],
            &p[8..12],
            &p[12..16],
            &p[16..20],
            &p[20..32]
        )
    } else {
        point_id.to_string()
    }
}

impl VectorStore for QdrantVectorStore {
    fn upsert<'a>(&'a self, point_id: &'a str, vector: Vec<f64>, payload: Value) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let point_id = point_id.to_string();
        Box::pin(async move {
            let url = format!("{}/collections/{}/points", self.url.trim_end_matches('/'), self.collection);
            let qid = coerce_qdrant_id(&point_id);

            let req_body = serde_json::json!({
                "points": [
                    {
                        "id": qid,
                        "vector": vector,
                        "payload": payload
                    }
                ]
            });

            let mut req = self.client.post(&url).json(&req_body);
            if let Some(ref key) = self.api_key {
                req = req.header("api-key", key);
            }

            let resp = req.send().await.map_err(|e| format!("Qdrant upsert HTTP error: {:?}", e))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Qdrant upsert failed ({}): {}", status, body));
            }
            Ok(())
        })
    }

    fn search<'a>(&'a self, vector: Vec<f64>, top_k: usize) -> Pin<Box<dyn Future<Output = Result<Vec<(String, f64, Value)>, String>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/collections/{}/points/search", self.url.trim_end_matches('/'), self.collection);

            let req_body = serde_json::json!({
                "vector": vector,
                "limit": top_k,
                "with_payload": true
            });

            let mut req = self.client.post(&url).json(&req_body);
            if let Some(ref key) = self.api_key {
                req = req.header("api-key", key);
            }

            let resp = req.send().await.map_err(|e| format!("Qdrant search HTTP error: {:?}", e))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Qdrant search failed ({}): {}", status, body));
            }

            #[derive(Deserialize)]
            struct QdrantPoint {
                id: Value,
                score: f64,
                payload: Option<Value>,
            }

            #[derive(Deserialize)]
            struct QdrantSearchResponse {
                result: Vec<QdrantPoint>,
            }

            let res_payload: QdrantSearchResponse = resp.json().await
                .map_err(|e| format!("Qdrant search parse error: {:?}", e))?;

            let mut out = Vec::new();
            for r in res_payload.result {
                let id_str = match r.id {
                    Value::String(s) => s,
                    Value::Number(num) => num.to_string(),
                    other => format!("{}", other),
                };
                let payload = r.payload.unwrap_or_else(|| serde_json::json!({}));
                out.push((id_str, r.score, payload));
            }
            Ok(out)
        })
    }

    fn delete<'a>(&'a self, point_id: &'a str) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let point_id = point_id.to_string();
        Box::pin(async move {
            let url = format!("{}/collections/{}/points/delete", self.url.trim_end_matches('/'), self.collection);
            let qid = coerce_qdrant_id(&point_id);

            let req_body = serde_json::json!({
                "points": [qid]
            });

            let mut req = self.client.post(&url).json(&req_body);
            if let Some(ref key) = self.api_key {
                req = req.header("api-key", key);
            }

            let resp = req.send().await.map_err(|e| format!("Qdrant delete HTTP error: {:?}", e))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Qdrant delete failed ({}): {}", status, body));
            }
            Ok(())
        })
    }

    fn clear(&self) -> Pin<Box<dyn Future<Output = Result<usize, String>> + Send + '_>> {
        Box::pin(async move {
            let url = format!("{}/collections/{}/points/delete", self.url.trim_end_matches('/'), self.collection);
            let req_body = serde_json::json!({
                "filter": {}
            });

            let mut req = self.client.post(&url).json(&req_body);
            if let Some(ref key) = self.api_key {
                req = req.header("api-key", key);
            }

            let resp = req.send().await.map_err(|e| format!("Qdrant clear HTTP error: {:?}", e))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Qdrant clear failed ({}): {}", status, body));
            }
            Ok(0)
        })
    }

    fn stats(&self) -> Pin<Box<dyn Future<Output = Value> + Send + '_>> {
        Box::pin(async move {
            let url = format!("{}/collections/{}", self.url.trim_end_matches('/'), self.collection);
            let mut req = self.client.get(&url);
            if let Some(ref key) = self.api_key {
                req = req.header("api-key", key);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    return serde_json::json!({
                        "backend": "qdrant",
                        "collection": self.collection,
                        "error": format!("HTTP error: {:?}", e),
                    });
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                return serde_json::json!({
                    "backend": "qdrant",
                    "collection": self.collection,
                    "error": format!("Qdrant status error: {}", status),
                });
            }

            #[derive(Deserialize)]
            struct QdrantCollectionInfo {
                points_count: Option<u64>,
            }

            #[derive(Deserialize)]
            struct QdrantCollectionResponse {
                result: QdrantCollectionInfo,
            }

            match resp.json::<QdrantCollectionResponse>().await {
                Ok(r) => {
                    serde_json::json!({
                        "backend": "qdrant",
                        "collection": self.collection,
                        "entries": r.result.points_count.unwrap_or(0),
                        "dim": self.dim,
                    })
                }
                Err(e) => {
                    serde_json::json!({
                        "backend": "qdrant",
                        "collection": self.collection,
                        "error": format!("Parse error: {:?}", e),
                    })
                }
            }
        })
    }
}

pub struct L2Cache {
    pub embedding_client: Option<Arc<dyn EmbeddingClient>>,
    pub vector_store: Option<Arc<dyn VectorStore>>,
    similarity_threshold: f64,
    pub enabled: std::sync::atomic::AtomicBool,
    verify_exact: bool,
    lookups: std::sync::atomic::AtomicUsize,
    hits: std::sync::atomic::AtomicUsize,
}

impl L2Cache {
    pub fn new(
        embedding_client: Option<Arc<dyn EmbeddingClient>>,
        vector_store: Option<Arc<dyn VectorStore>>,
        similarity_threshold: f64,
        enabled: bool,
        verify_exact: bool,
    ) -> Result<Self, String> {
        if enabled && (embedding_client.is_none() || vector_store.is_none()) {
            return Err(
                "L2 semantic cache is enabled but no embedding_client or vector_store was provided. \
                 Either disable L2 or wire both.".to_string()
            );
        }
        Ok(L2Cache {
            embedding_client,
            vector_store,
            similarity_threshold,
            enabled: std::sync::atomic::AtomicBool::new(enabled),
            verify_exact,
            lookups: std::sync::atomic::AtomicUsize::new(0),
            hits: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    pub async fn get_similar(
        &self,
        normalized_payload_text: &str,
        threshold: Option<f64>,
    ) -> Option<SemanticHit> {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let client = self.embedding_client.as_ref()?;
        let store = self.vector_store.as_ref()?;

        self.lookups.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let vec = match client.embed(normalized_payload_text).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("L2 embedding lookup failed: {}", e);
                return None;
            }
        };

        let results = match store.search(vec, 1).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("L2 search failed: {}", e);
                return None;
            }
        };

        if results.is_empty() {
            return None;
        }

        let (point_id, similarity, metadata) = &results[0];
        let eff_threshold = threshold.unwrap_or(self.similarity_threshold);
        if *similarity < eff_threshold {
            return None;
        }

        if self.verify_exact {
            let stored_text = metadata.get("__norm_text__").and_then(|v| v.as_str());
            if stored_text != Some(normalized_payload_text) {
                return None;
            }
        }

        let response = metadata_to_response(metadata)?;
        self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Some(SemanticHit {
            similarity: *similarity,
            response,
            point_id: point_id.clone(),
            metadata: metadata.clone(),
        })
    }

    pub async fn put_similar(&self, normalized_payload_text: &str, response: &CachedResponse, point_id: &str) {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let client = match self.embedding_client.as_ref() {
            Some(c) => c,
            None => return,
        };
        let store = match self.vector_store.as_ref() {
            Some(s) => s,
            None => return,
        };

        let vec = match client.embed(normalized_payload_text).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("L2 put embedding failed: {}", e);
                return;
            }
        };

        let mut metadata = response_to_metadata(response);
        metadata.insert("__norm_text__".to_string(), Value::String(normalized_payload_text.to_string()));

        let _ = store.upsert(point_id, vec, Value::Object(metadata)).await;
    }

    pub async fn stats(&self) -> Value {
        serde_json::json!({
            "enabled": self.enabled.load(std::sync::atomic::Ordering::Relaxed),
            "lookups": self.lookups.load(std::sync::atomic::Ordering::Relaxed),
            "hits": self.hits.load(std::sync::atomic::Ordering::Relaxed),
            "threshold": self.similarity_threshold,
            "embedding_dim": self.embedding_client.as_ref().map(|c| c.dimension()),
        })
    }

    pub async fn clear(&self) -> usize {
        let mut cleared = 0;
        if let Some(store) = &self.vector_store {
            if let Ok(c) = store.clear().await {
                cleared = c;
            }
        }
        self.lookups.store(0, std::sync::atomic::Ordering::Relaxed);
        self.hits.store(0, std::sync::atomic::Ordering::Relaxed);
        cleared
    }
}

fn response_to_metadata(response: &CachedResponse) -> serde_json::Map<String, Value> {
    let body_b64 = base64_encode(&response.body);
    let mut m = serde_json::Map::new();
    m.insert("status_code".to_string(), serde_json::json!(response.status_code));
    let headers_val = serde_json::to_value(&response.headers).unwrap_or_default();
    m.insert("headers".to_string(), headers_val);
    m.insert("body_b64".to_string(), Value::String(body_b64));
    m.insert("media_type".to_string(), serde_json::json!(response.media_type));
    m.insert("inserted_at".to_string(), serde_json::json!(response.inserted_at));
    m.insert("hit_count".to_string(), serde_json::json!(response.hit_count));
    m
}

fn metadata_to_response(metadata: &Value) -> Option<CachedResponse> {
    let obj = metadata.as_object()?;
    let status_code = obj.get("status_code")?.as_u64()? as u16;
    
    let headers_val = obj.get("headers")?;
    let headers: HashMap<String, String> = serde_json::from_value(headers_val.clone()).ok()?;
    
    let body_b64 = obj.get("body_b64")?.as_str()?;
    let body = base64_decode(body_b64)?;
    
    let media_type = obj.get("media_type").and_then(|v| v.as_str().map(|s| s.to_string()));
    let inserted_at = obj.get("inserted_at").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let hit_count = obj.get("hit_count").and_then(|v| v.as_i64()).unwrap_or(0);

    Some(CachedResponse {
        status_code,
        headers,
        body,
        media_type,
        inserted_at,
        last_hit_at: inserted_at,
        hit_count,
    })
}

fn base64_encode(data: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as usize;
        let b1 = if i + 1 < data.len() { data[i + 1] as usize } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as usize } else { 0 };

        let c0 = b0 >> 2;
        let c1 = ((b0 & 3) << 4) | (b1 >> 4);
        let c2 = ((b1 & 15) << 2) | (b2 >> 6);
        let c3 = b2 & 63;

        result.push(CHARSET[c0] as char);
        result.push(CHARSET[c1] as char);
        if i + 1 < data.len() {
            result.push(CHARSET[c2] as char);
        } else {
            result.push('=');
        }
        if i + 2 < data.len() {
            result.push(CHARSET[c3] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut map = [0u8; 256];
    for (idx, &c) in CHARSET.iter().enumerate() {
        map[c as usize] = idx as u8;
    }

    let input = input.trim_end_matches('=');
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 >= bytes.len() {
            return None;
        }
        let c0 = map[bytes[i] as usize] as usize;
        let c1 = map[bytes[i + 1] as usize] as usize;
        result.push(((c0 << 2) | (c1 >> 4)) as u8);

        if i + 2 < bytes.len() {
            let c2 = map[bytes[i + 2] as usize] as usize;
            result.push(((c1 << 4) | (c2 >> 2)) as u8);

            if i + 3 < bytes.len() {
                let c3 = map[bytes[i + 3] as usize] as usize;
                result.push(((c2 << 6) | c3) as u8);
                i += 4;
            } else {
                i += 3;
            }
        } else {
            i += 2;
        }
    }
    Some(result)
}
