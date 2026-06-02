use std::time::{Duration};
use std::future::Future;
use std::pin::Pin;
use blake2::Blake2bVar;
use blake2::digest::{Update, VariableOutput};
use serde::{Deserialize, Serialize};

pub trait EmbeddingClient: Send + Sync {
    fn embed<'a>(&'a self, text: &'a str) -> Pin<Box<dyn Future<Output = Result<Vec<f64>, String>> + Send + 'a>>;
    fn dimension(&self) -> usize;
}

pub const DEFAULT_HASH_DIM: usize = 3072;

pub struct HashEmbedder {
    dim: usize,
    shingle_chars: usize,
}

impl HashEmbedder {
    pub fn new(dim: usize, shingle_chars: usize) -> Self {
        HashEmbedder {
            dim: if dim < 16 { DEFAULT_HASH_DIM } else { dim },
            shingle_chars: if shingle_chars < 1 { 4 } else { shingle_chars },
        }
    }
}

impl EmbeddingClient for HashEmbedder {
    fn embed<'a>(&'a self, text: &'a str) -> Pin<Box<dyn Future<Output = Result<Vec<f64>, String>> + Send + 'a>> {
        let text_owned = text.to_string();
        Box::pin(async move {
            let char_count = text_owned.chars().count();
            let mut padded = text_owned;
            if char_count < self.shingle_chars {
                for _ in 0..(self.shingle_chars - char_count) {
                    padded.push('\0');
                }
            }

            let mut vec = vec![0.0; self.dim];
            let chars: Vec<char> = padded.chars().collect();

            for i in 0..=(chars.len() - self.shingle_chars) {
                let shingle: String = chars[i..i + self.shingle_chars].iter().collect();
                let mut hasher = Blake2bVar::new(8)
                    .map_err(|e| format!("Failed to initialize Blake2bVar: {:?}", e))?;
                hasher.update(shingle.as_bytes());
                let mut digest = [0u8; 8];
                hasher.finalize_variable(&mut digest)
                    .map_err(|e| format!("Failed to finalize Blake2bVar: {:?}", e))?;

                let bucket = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) as usize % self.dim;
                let sign = if (digest[4] & 1) == 1 { 1.0 } else { -1.0 };
                vec[bucket] += sign;
            }

            let sum_sq: f64 = vec.iter().map(|v| v * v).sum();
            let norm = sum_sq.sqrt();
            if norm == 0.0 {
                return Ok(vec);
            }
            let inv = 1.0 / norm;
            for v in vec.iter_mut() {
                *v *= inv;
            }
            Ok(vec)
        })
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

#[derive(Serialize)]
struct OpenAIEmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
    dimensions: usize,
}

#[derive(Deserialize)]
struct OpenAIEmbedData {
    embedding: Vec<f64>,
}

#[derive(Deserialize)]
struct OpenAIEmbedResponse {
    data: Vec<OpenAIEmbedData>,
}

pub struct OpenAIEmbeddingClient {
    client: reqwest::Client,
    model: String,
    dim: usize,
    api_key: String,
}

impl OpenAIEmbeddingClient {
    pub fn new(model: String, dim: usize, api_key: String, timeout_s: u64, _base_url: Option<String>) -> Self {
        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_s));
        
        let client = builder.build().unwrap_or_default();

        OpenAIEmbeddingClient {
            client,
            model,
            dim,
            api_key,
        }
    }
}

impl EmbeddingClient for OpenAIEmbeddingClient {
    fn embed<'a>(&'a self, text: &'a str) -> Pin<Box<dyn Future<Output = Result<Vec<f64>, String>> + Send + 'a>> {
        let text_owned = text.to_string();
        Box::pin(async move {
            let req_payload = OpenAIEmbedRequest {
                model: &self.model,
                input: &text_owned,
                dimensions: self.dim,
            };

            let response = self.client.post("https://api.openai.com/v1/embeddings")
                .bearer_auth(&self.api_key)
                .json(&req_payload)
                .send()
                .await
                .map_err(|e| format!("Failed to send embedding request: {:?}", e))?;

            if !response.status().is_success() {
                let status = response.status();
                let err_text = response.text().await.unwrap_or_default();
                return Err(format!("OpenAI embedding error {}: {}", status, err_text));
            }

            let resp_payload: OpenAIEmbedResponse = response.json()
                .await
                .map_err(|e| format!("Failed to parse OpenAI embedding response: {:?}", e))?;

            if resp_payload.data.is_empty() {
                return Err("OpenAI embedding response returned empty data".to_string());
            }

            let embedding = resp_payload.data[0].embedding.clone();
            if embedding.len() != self.dim {
                return Err(format!(
                    "OpenAI returned {}-dim embedding; expected {}.",
                    embedding.len(),
                    self.dim
                ));
            }

            Ok(embedding)
        })
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}
