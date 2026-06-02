use crate::compression::jl::{SketchRecord, cosine, signed_jl_projection};
use crate::sim::lsh::MinHashLSH;
use crate::sim::minhash::minhash_signature;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct HybridSketchIndex {
    pub jl_dims: usize,
    pub jl_shingle_tokens: usize,
    pub mh_num_perms: usize,
    pub mh_bands: usize,
    pub mh_shingle_size: usize,
    pub small_corpus_cutoff: usize,
    records: Vec<SketchRecord>,
    record_by_id: HashMap<String, SketchRecord>,
    lsh: MinHashLSH,
}

impl HybridSketchIndex {
    pub fn new(
        jl_dims: usize,
        jl_shingle_tokens: usize,
        mh_num_perms: usize,
        mh_bands: usize,
        mh_shingle_size: usize,
        small_corpus_cutoff: usize,
    ) -> Self {
        let lsh = MinHashLSH::new(mh_num_perms, mh_bands);
        HybridSketchIndex {
            jl_dims,
            jl_shingle_tokens,
            mh_num_perms,
            mh_bands,
            mh_shingle_size,
            small_corpus_cutoff,
            records: Vec::new(),
            record_by_id: HashMap::new(),
            lsh,
        }
    }

    pub fn add(&mut self, text: &str, path: String, digest: String) {
        let sketch = signed_jl_projection(text, self.jl_dims, self.jl_shingle_tokens, "middleout-jl-v1");
        let record = SketchRecord {
            path: path.clone(),
            digest: digest.clone(),
            chars: text.len(),
            sketch,
        };
        
        let doc_id = self.make_doc_id(&path, &digest, self.records.len());
        self.records.push(record.clone());
        self.record_by_id.insert(doc_id.clone(), record);

        let sig = minhash_signature(text, self.mh_num_perms, self.mh_shingle_size, "middleout-mh-v1");
        self.lsh.add(doc_id, &sig);
    }

    pub fn find_best(&self, text: &str) -> (Option<&SketchRecord>, f64) {
        if self.records.is_empty() {
            return (None, -1.0);
        }

        let sketch = signed_jl_projection(text, self.jl_dims, self.jl_shingle_tokens, "middleout-jl-v1");

        // Brute-force JL scan for tiny corpora
        if self.records.len() < self.small_corpus_cutoff {
            return self.scan(&sketch, &self.records);
        }

        let sig = minhash_signature(text, self.mh_num_perms, self.mh_shingle_size, "middleout-mh-v1");
        let candidate_ids = self.lsh.candidates(&sig);
        if candidate_ids.is_empty() {
            return (None, -1.0);
        }

        let mut candidates = Vec::new();
        for cid in &candidate_ids {
            if let Some(record) = self.record_by_id.get(cid) {
                candidates.push(record);
            }
        }
        if candidates.is_empty() {
            return (None, -1.0);
        }
        
        self.scan_refs(&sketch, &candidates)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    fn make_doc_id(&self, path: &str, digest: &str, index: usize) -> String {
        format!("{}#{}#{}", path, digest, index)
    }

    fn scan<'a>(&self, sketch: &[f64], records: &'a [SketchRecord]) -> (Option<&'a SketchRecord>, f64) {
        let mut best_record: Option<&'a SketchRecord> = None;
        let mut best_score = -1.0;
        for record in records {
            let score = cosine(sketch, &record.sketch);
            if score > best_score {
                best_score = score;
                best_record = Some(record);
            }
        }
        (best_record, best_score)
    }

    fn scan_refs<'a>(&self, sketch: &[f64], records: &[&'a SketchRecord]) -> (Option<&'a SketchRecord>, f64) {
        let mut best_record: Option<&'a SketchRecord> = None;
        let mut best_score = -1.0;
        for &record in records {
            let score = cosine(sketch, &record.sketch);
            if score > best_score {
                best_score = score;
                best_record = Some(record);
            }
        }
        (best_record, best_score)
    }
}
