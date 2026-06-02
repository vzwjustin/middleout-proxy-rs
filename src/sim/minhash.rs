use crate::compression::jl::{tokenize_words};
use blake2::Blake2bVar;
use blake2::digest::{Update, VariableOutput};

const UINT64_MAX: u64 = u64::MAX;

fn word_shingles(text: &str, shingle_size: usize) -> Vec<String> {
    let tokens = tokenize_words(text);
    if tokens.is_empty() {
        return Vec::new();
    }
    let size = shingle_size.max(1);
    if tokens.len() <= size {
        return vec![tokens.join(" ")];
    }
    let mut out = Vec::with_capacity(tokens.len() - size + 1);
    for i in 0..=(tokens.len() - size) {
        out.push(tokens[i..(i + size)].join(" "));
    }
    out
}

pub fn minhash_signature(
    text: &str,
    num_perms: usize,
    shingle_size: usize,
    seed: &str,
) -> Vec<u64> {
    assert!(num_perms > 0, "num_perms must be positive");

    let shingles = word_shingles(text, shingle_size);
    if shingles.is_empty() {
        return vec![UINT64_MAX; num_perms];
    }

    let mut mins = vec![UINT64_MAX; num_perms];

    // Seed prefix as bytes: seed + \x00
    let seed_bytes = seed.as_bytes();
    
    for i in 0..num_perms {
        let mut local_min = UINT64_MAX;
        
        // Construct prefix: seed_bytes + b"\x00" + struct.pack("!I", i)
        let mut prefix = Vec::with_capacity(seed_bytes.len() + 1 + 4);
        prefix.extend_from_slice(seed_bytes);
        prefix.push(0u8);
        prefix.extend_from_slice(&(i as u32).to_be_bytes());

        for shingle in &shingles {
            let mut hasher = Blake2bVar::new(8).expect("Failed to initialize Blake2bVar");
            hasher.update(&prefix);
            hasher.update(shingle.as_bytes());
            
            let mut buf = [0u8; 8];
            hasher.finalize_variable(&mut buf).expect("Failed to finalize Blake2bVar");
            let value = u64::from_be_bytes(buf);
            
            if value < local_min {
                local_min = value;
            }
        }
        mins[i] = local_min;
    }

    mins
}

pub fn jaccard_estimate(sig_a: &[u64], sig_b: &[u64]) -> f64 {
    assert_eq!(sig_a.len(), sig_b.len(), "Signature length mismatch");
    if sig_a.is_empty() {
        return 0.0;
    }
    let matches = sig_a.iter().zip(sig_b.iter()).filter(|(x, y)| x == y).count();
    (matches as f64) / (sig_a.len() as f64)
}
