use crate::compression::jl::{tokenize_words, hash64};

const BITS: usize = 64;

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

pub fn simhash64(text: &str, shingle_size: usize, seed: &str) -> u64 {
    let shingles = word_shingles(text, shingle_size);
    if shingles.is_empty() {
        return 0;
    }

    // seed_prefix in python is: seed.encode("utf-8") + b"\x00" + struct.pack("!I", 0)
    // struct.pack("!I", 0) is [0, 0, 0, 0]
    let seed_prefix = format!("{}\x00\x00\x00\x00\x00", seed);
    let mut counts = vec![0; BITS];

    for shingle in shingles {
        let h = hash64(&seed_prefix, &shingle);
        for j in 0..BITS {
            if ((h >> j) & 1) == 1 {
                counts[j] += 1;
            } else {
                counts[j] -= 1;
            }
        }
    }

    let mut out = 0u64;
    for j in 0..BITS {
        if counts[j] > 0 {
            out |= 1 << j;
        }
    }
    out
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub fn simhash_similarity(a: u64, b: u64) -> f64 {
    1.0 - (hamming_distance(a, b) as f64) / (BITS as f64)
}
