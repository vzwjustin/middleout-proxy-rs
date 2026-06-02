use blake2::Blake2bVar;
use blake2::digest::{Update, VariableOutput};
use regex::Regex;
use std::sync::OnceLock;

static TOKEN_RE: OnceLock<Regex> = OnceLock::new();
static WORD_RE: OnceLock<Regex> = OnceLock::new();

fn get_token_re() -> &'static Regex {
    TOKEN_RE.get_or_init(|| {
        Regex::new(r"[A-Za-z_][A-Za-z0-9_]*|\d+(?:\.\d+)?|[^\s]").unwrap()
    })
}

fn get_word_re() -> &'static Regex {
    WORD_RE.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9]+").unwrap()
    })
}

pub fn tokenize(text: &str) -> Vec<String> {
    let re = get_token_re();
    re.find_iter(text)
        .map(|m| m.as_str().to_lowercase())
        .collect()
}

pub fn tokenize_words(text: &str) -> Vec<String> {
    let re = get_word_re();
    re.find_iter(text)
        .map(|m| m.as_str().to_lowercase())
        .collect()
}

pub fn shingles(tokens: &[String], width: usize) -> Vec<String> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let width = width.max(1);
    if tokens.len() <= width {
        return vec![tokens.join(" ")];
    }
    let mut out = Vec::with_capacity(tokens.len() - width + 1);
    for i in 0..=(tokens.len() - width) {
        out.push(tokens[i..(i + width)].join(" "));
    }
    out
}

pub fn hash64(seed: &str, value: &str) -> u64 {
    let mut hasher = Blake2bVar::new(8).expect("Failed to initialize Blake2bVar");
    let payload = format!("{}\0{}", seed, value);
    hasher.update(payload.as_bytes());
    let mut buf = [0u8; 8];
    hasher.finalize_variable(&mut buf).expect("Failed to finalize Blake2bVar");
    u64::from_be_bytes(buf)
}

pub fn signed_jl_projection(
    text: &str,
    dims: usize,
    shingle_tokens: usize,
    seed: &str,
) -> Vec<f64> {
    let mut vec = vec![0.0; dims];
    let toks = tokenize(text);
    for shingle in shingles(&toks, shingle_tokens) {
        let h = hash64(seed, &shingle);
        let idx = (h as usize) % dims;
        let sign = if ((h >> 32) & 1) == 1 { 1.0 } else { -1.0 };
        vec[idx] += sign;
    }

    let sum_sq: f64 = vec.iter().map(|v| v * v).sum();
    let norm = sum_sq.sqrt();
    if norm == 0.0 {
        return vec;
    }
    for v in vec.iter_mut() {
        *v /= norm;
    }
    vec
}

pub fn cosine(a: &[f64], b: &[f64]) -> f64 {
    assert_eq!(a.len(), b.len(), "Vectors must have the same dimensionality");
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[derive(Debug, Clone)]
pub struct SketchRecord {
    pub path: String,
    pub digest: String,
    pub chars: usize,
    pub sketch: Vec<f64>,
}

#[derive(Debug)]
pub struct RequestSketchIndex {
    pub dims: usize,
    pub shingle_tokens: usize,
    records: Vec<SketchRecord>,
}

impl RequestSketchIndex {
    pub fn new(dims: usize, shingle_tokens: usize) -> Self {
        RequestSketchIndex {
            dims,
            shingle_tokens,
            records: Vec::new(),
        }
    }

    pub fn find_best(&self, text: &str) -> (Option<&SketchRecord>, f64) {
        let sketch = signed_jl_projection(text, self.dims, self.shingle_tokens, "middleout-jl-v1");
        let mut best_record: Option<&SketchRecord> = None;
        let mut best_score = -1.0;
        for record in &self.records {
            let score = cosine(&sketch, &record.sketch);
            if score > best_score {
                best_score = score;
                best_record = Some(record);
            }
        }
        (best_record, best_score)
    }

    pub fn add(&mut self, text: &str, path: String, digest: String) {
        let sketch = signed_jl_projection(text, self.dims, self.shingle_tokens, "middleout-jl-v1");
        self.records.push(SketchRecord {
            path,
            digest,
            chars: text.len(),
            sketch,
        });
    }

    pub fn records(&self) -> &[SketchRecord] {
        &self.records
    }
}
