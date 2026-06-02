use blake2::Blake2bVar;
use blake2::digest::{Update, VariableOutput};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct MinHashLSH {
    pub num_perms: usize,
    pub bands: usize,
    pub rows: usize,
    // List of bands, each band has a map of: HashKey (16 bytes) -> Vec of DocIDs
    buckets: Vec<HashMap<Vec<u8>, Vec<String>>>,
    // Mapping of: DocID -> List of BandKeys (each key is 16 bytes)
    band_keys_by_doc: HashMap<String, Vec<Vec<u8>>>,
}

impl MinHashLSH {
    pub fn new(num_perms: usize, bands: usize) -> Self {
        assert!(num_perms > 0, "num_perms must be positive");
        assert!(bands > 0, "bands must be positive");
        assert_eq!(num_perms % bands, 0, "bands must divide num_perms evenly");

        let rows = num_perms / bands;
        let buckets = vec![HashMap::new(); bands];
        
        MinHashLSH {
            num_perms,
            bands,
            rows,
            buckets,
            band_keys_by_doc: HashMap::new(),
        }
    }

    pub fn add(&mut self, doc_id: String, signature: &[u64]) {
        self.validate_sig(signature);
        
        // Idempotent re-add: remove old one if it exists
        if self.band_keys_by_doc.contains_key(&doc_id) {
            self.remove(&doc_id);
        }

        let keys = self.band_keys(signature);
        for (band_idx, key) in keys.iter().enumerate() {
            let bucket = self.buckets[band_idx].entry(key.clone()).or_insert_with(Vec::new);
            bucket.push(doc_id.clone());
        }
        self.band_keys_by_doc.insert(doc_id, keys);
    }

    pub fn candidates(&self, signature: &[u64]) -> HashSet<String> {
        self.validate_sig(signature);
        
        let keys = self.band_keys(signature);
        let mut out = HashSet::new();
        for (band_idx, key) in keys.iter().enumerate() {
            if let Some(bucket) = self.buckets[band_idx].get(key) {
                for doc_id in bucket {
                    out.insert(doc_id.clone());
                }
            }
        }
        out
    }

    pub fn remove(&mut self, doc_id: &str) {
        let keys = self.band_keys_by_doc.remove(doc_id);
        if let Some(keys) = keys {
            for (band_idx, key) in keys.iter().enumerate() {
                if let Some(bucket) = self.buckets[band_idx].get_mut(key) {
                    if let Some(pos) = bucket.iter().position(|x| x == doc_id) {
                        bucket.swap_remove(pos);
                    }
                    if bucket.is_empty() {
                        self.buckets[band_idx].remove(key);
                    }
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.band_keys_by_doc.len()
    }

    pub fn is_empty(&self) -> bool {
        self.band_keys_by_doc.is_empty()
    }

    pub fn contains(&self, doc_id: &str) -> bool {
        self.band_keys_by_doc.contains_key(doc_id)
    }

    // ----- internals -----

    fn validate_sig(&self, signature: &[u64]) {
        assert_eq!(
            signature.len(),
            self.num_perms,
            "Signature length must match num_perms"
        );
    }

    fn band_keys(&self, signature: &[u64]) -> Vec<Vec<u8>> {
        let mut keys = Vec::with_capacity(self.bands);
        for band_idx in 0..self.bands {
            let start = band_idx * self.rows;
            
            // Pack slab: Q * rows
            let mut slab = Vec::with_capacity(self.rows * 8);
            for i in 0..self.rows {
                slab.extend_from_slice(&signature[start + i].to_be_bytes());
            }

            // Blake2b-128
            let mut hasher = Blake2bVar::new(16).expect("Failed to initialize Blake2bVar");
            hasher.update(&slab);
            let mut buf = vec![0u8; 16];
            hasher.finalize_variable(&mut buf).expect("Failed to finalize Blake2bVar");
            keys.push(buf);
        }
        keys
    }
}
