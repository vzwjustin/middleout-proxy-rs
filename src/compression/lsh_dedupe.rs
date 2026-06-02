use blake2::Blake2bVar;
use blake2::digest::{Update, VariableOutput};
use std::collections::{HashMap, HashSet};
use serde_json::Value;

const HYBRID_CHAR_LEN_MIN: usize = 120;

fn hash_token(seed_idx: usize, token: &str) -> u64 {
    let mut hasher = Blake2bVar::new(8).expect("Failed to initialize Blake2bVar");
    let payload = format!("mh{:04}:{}", seed_idx, token);
    hasher.update(payload.as_bytes());
    let mut buf = [0u8; 8];
    hasher.finalize_variable(&mut buf).expect("Failed to finalize Blake2bVar");
    u64::from_be_bytes(buf)
}

fn char_shingles(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    if chars.len() <= width {
        return vec![text.to_string()];
    }
    let mut out = Vec::with_capacity(chars.len() - width + 1);
    for i in 0..=(chars.len() - width) {
        let s: String = chars[i..(i + width)].iter().collect();
        out.push(s);
    }
    out
}

fn collect_shingles(text: &str) -> Vec<String> {
    if text.chars().count() >= HYBRID_CHAR_LEN_MIN {
        let tokens = crate::compression::jl::tokenize(text);
        let word_shingles = crate::compression::jl::shingles(&tokens, 5);
        if !word_shingles.is_empty() {
            return word_shingles;
        }
    }
    char_shingles(text, 8)
        .into_iter()
        .map(|s| format!("\x01{}", s))
        .collect()
}

fn minhash_signature(text: &str) -> Vec<u64> {
    let mut sigs = vec![u64::MAX; 128];
    let shingles_list = collect_shingles(text);
    if shingles_list.is_empty() {
        return vec![0; 128];
    }
    for shingle in shingles_list {
        for i in 0..128 {
            let h = hash_token(i, &shingle);
            if h < sigs[i] {
                sigs[i] = h;
            }
        }
    }
    sigs
}

fn band_keys(signature: &[u64], bands: usize, rows: usize) -> Vec<Vec<u8>> {
    let mut keys = Vec::with_capacity(bands);
    for b in 0..bands {
        let chunk = &signature[b * rows..(b + 1) * rows];
        let mut packed = Vec::with_capacity(rows * 8);
        for &v in chunk {
            packed.extend_from_slice(&v.to_be_bytes());
        }
        let mut hasher = Blake2bVar::new(8).expect("Failed to initialize Blake2bVar");
        hasher.update(&packed);
        let mut buf = vec![0u8; 8];
        hasher.finalize_variable(&mut buf).expect("Failed to finalize Blake2bVar");
        keys.push(buf);
    }
    keys
}

fn jaccard_estimate(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let same = a.iter().zip(b.iter()).filter(|(&x, &y)| x == y).count();
    (same as f64) / (a.len() as f64)
}

pub struct LSHDedupeIndex {
    pub threshold: f64,
    pub bands: usize,
    pub rows: usize,
    bands_buckets: Vec<HashMap<Vec<u8>, Vec<usize>>>,
    signatures: HashMap<usize, Vec<u64>>,
}

impl LSHDedupeIndex {
    pub fn new(level: &str) -> Result<Self, String> {
        let (threshold, bands, rows) = match level {
            "conservative" => (0.95, 8, 16),
            "standard" => (0.88, 16, 8),
            "aggressive" => (0.80, 32, 4),
            _ => return Err(format!("lsh level must be conservative, standard, or aggressive, got {:?}", level)),
        };

        Ok(LSHDedupeIndex {
            threshold,
            bands,
            rows,
            bands_buckets: vec![HashMap::new(); bands],
            signatures: HashMap::new(),
        })
    }

    pub fn add(&mut self, block_id: usize, text: &str) {
        let sig = minhash_signature(text);
        let keys = band_keys(&sig, self.bands, self.rows);
        for (b, key) in keys.into_iter().enumerate() {
            self.bands_buckets[b].entry(key).or_insert_with(Vec::new).push(block_id);
        }
        self.signatures.insert(block_id, sig);
    }

    pub fn find_near_duplicate(&self, text: &str) -> Option<(usize, f64)> {
        if self.signatures.is_empty() {
            return None;
        }
        let sig = minhash_signature(text);
        let keys = band_keys(&sig, self.bands, self.rows);
        let mut candidates = HashSet::new();
        for (b, key) in keys.into_iter().enumerate() {
            if let Some(bucket) = self.bands_buckets[b].get(&key) {
                for &cand in bucket {
                    candidates.insert(cand);
                }
            }
        }

        let mut best: Option<(usize, f64)> = None;
        for cand in candidates {
            if let Some(other) = self.signatures.get(&cand) {
                let score = jaccard_estimate(&sig, other);
                if score >= self.threshold {
                    if best.is_none() || score > best.unwrap().1 {
                        best = Some((cand, score));
                    }
                }
            }
        }
        best
    }
}

pub fn block_text(block: &Value) -> Option<String> {
    if !block.is_object() {
        return None;
    }
    let btype = block.get("type")?.as_str()?;
    if btype == "text" {
        return block.get("text")?.as_str().map(|s| s.to_string());
    }
    if btype == "tool_result" {
        let content = block.get("content")?;
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }
        if let Some(arr) = content.as_array() {
            let mut parts = Vec::new();
            for it in arr {
                if it.is_object() {
                    if let Some(type_val) = it.get("type").and_then(|t| t.as_str()) {
                        if type_val == "text" {
                            if let Some(text_val) = it.get("text").and_then(|t| t.as_str()) {
                                parts.push(text_val.to_string());
                            }
                        }
                    }
                }
            }
            if !parts.is_empty() {
                return Some(parts.join("\n"));
            }
        }
    }
    None
}

pub fn set_block_text(block: &mut Value, new_text: &str) {
    if let Some(obj) = block.as_object_mut() {
        let btype = obj.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if btype == "text" {
            obj.insert("text".to_string(), serde_json::json!(new_text));
        } else if btype == "tool_result" {
            obj.insert("content".to_string(), serde_json::json!(new_text));
        }
    }
}

pub fn dedupe_blocks(
    blocks: &[Value],
    level: &str,
    protected: &HashSet<usize>,
) -> Result<(Vec<Value>, serde_json::Value), String> {
    let mut new_blocks = blocks.to_vec();
    let mut index = LSHDedupeIndex::new(level)?;
    let mut replaced = 0;

    for (i, block) in new_blocks.iter_mut().enumerate() {
        let text = match block_text(block) {
            Some(t) => t,
            None => continue,
        };
        if text.is_empty() {
            continue;
        }

        if protected.contains(&i) {
            index.add(i, &text);
            continue;
        }

        if let Some((other_id, score)) = index.find_near_duplicate(&text) {
            let marker = format!(
                "[duplicate of earlier block at {}, ~{} chars, similarity {:.2}]",
                other_id, text.chars().count(), score
            );
            set_block_text(block, &marker);
            replaced += 1;
        } else {
            index.add(i, &text);
        }
    }

    let stats = serde_json::json!({
        "replaced": replaced,
        "level": level,
        "threshold": index.threshold,
        "protected": protected.len(),
    });

    Ok((new_blocks, stats))
}
