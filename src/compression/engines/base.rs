#[derive(Debug, Clone)]
pub struct EngineResult {
    pub text: String,
    pub note: String,
    pub original_chars: usize,
    pub compressed_chars: usize,
}

impl EngineResult {
    pub fn chars_saved(&self) -> usize {
        if self.original_chars > self.compressed_chars {
            self.original_chars - self.compressed_chars
        } else {
            0
        }
    }

    pub fn changed(&self) -> bool {
        !self.text.is_empty() && self.original_chars != self.compressed_chars
    }
}

pub fn make_result(original: &str, compressed: &str, note: String) -> EngineResult {
    EngineResult {
        text: compressed.to_string(),
        note,
        original_chars: original.chars().count(),
        compressed_chars: compressed.chars().count(),
    }
}

pub fn identity_result(text: &str) -> EngineResult {
    let char_count = text.chars().count();
    EngineResult {
        text: text.to_string(),
        note: "".to_string(),
        original_chars: char_count,
        compressed_chars: char_count,
    }
}
