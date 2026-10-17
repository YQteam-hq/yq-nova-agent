use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkOptions {
    pub enabled: bool,
    pub split_by: SplitBy,
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    #[serde(default = "default_overlap_chars")]
    pub overlap_chars: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            split_by: SplitBy::Paragraph,
            max_chars: default_max_chars(),
            overlap_chars: default_overlap_chars(),
        }
    }
}

impl ChunkOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_chars < 200 {
            return Err("max_chars must be >= 200".to_string());
        }
        if self.overlap_chars >= self.max_chars {
            return Err("overlap_chars must be < max_chars".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SplitBy {
    Paragraph,
    Char,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    pub uuid: Uuid,
    pub chunk_index: usize,
    pub chunk_total: usize,
}

fn default_max_chars() -> usize {
    1200
}

fn default_overlap_chars() -> usize {
    150
}

pub fn chunk_text(text: &str, opts: &ChunkOptions) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let max = opts.max_chars;
    let overlap = opts.overlap_chars;
    match opts.split_by {
        SplitBy::Paragraph => chunk_by_paragraph(text, max, overlap),
        SplitBy::Char => chunk_by_char(text, max, overlap),
    }
}

fn chunk_by_paragraph(text: &str, max_chars: usize, overlap_chars: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    if paragraphs.is_empty() {
        return Vec::new();
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for p in paragraphs {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            continue;
        }
        let sep = if current.is_empty() { "" } else { "\n\n" };
        let candidate = format!("{}{}{}", current, sep, trimmed);

        if candidate.len() > max_chars && !current.is_empty() {
            chunks.push(current.clone());

            if overlap_chars > 0 {
                // Cut the overlap out of the previous chunk on a char
                // boundary, otherwise a multi-byte character can be split
                // and the slice panics.
                let take = overlap_chars.min(current.len());
                let mut cut = current.len() - take;
                while !current.is_char_boundary(cut) {
                    cut += 1;
                }
                current = current[cut..].to_string();
                current.push_str("\n\n");
                current.push_str(trimmed);
            } else {
                current = trimmed.to_string();
            }
        } else {
            current = candidate;
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

fn chunk_by_char(text: &str, max_chars: usize, overlap_chars: usize) -> Vec<String> {
    let total_chars = text.chars().count();
    if total_chars == 0 {
        return Vec::new();
    }
    if total_chars <= max_chars {
        return vec![text.to_string()];
    }

    let step = max_chars.saturating_sub(overlap_chars);
    let step = if step == 0 { 1 } else { step };

    // Byte offset of every char boundary, computed once so that each chunk
    // slice starts and ends on a boundary and never splits a multi-byte
    // UTF-8 character.
    let mut char_bounds: Vec<usize> = Vec::with_capacity(total_chars + 1);
    char_bounds.extend(text.char_indices().map(|(byte_idx, _)| byte_idx));
    char_bounds.push(text.len());

    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0;
    while start < total_chars {
        let end = (start + max_chars).min(total_chars);
        chunks.push(text[char_bounds[start]..char_bounds[end]].to_string());
        start += step;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_options_validation() {
        let opts = ChunkOptions {
            max_chars: 100,
            ..Default::default()
        };
        assert!(opts.validate().is_err());

        let opts = ChunkOptions {
            max_chars: 200,
            overlap_chars: 200,
            ..Default::default()
        };
        assert!(opts.validate().is_err());

        let opts = ChunkOptions {
            max_chars: 200,
            overlap_chars: 150,
            ..Default::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn chunk_by_paragraph_single_chunk() {
        let text = "This is a short paragraph.";
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Paragraph,
            max_chars: 1200,
            overlap_chars: 150,
        };
        let chunks = chunk_text(text, &opts);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "This is a short paragraph.");
    }

    #[test]
    fn chunk_by_paragraph_multi_chunk() {
        let mut text = String::new();
        for i in 0..20 {
            if i > 0 {
                text.push_str("\n\n");
            }
            text.push_str(&format!(
                "This is paragraph number {} with some filler text to make it reasonably long so \
                 that we can test chunking behavior correctly.",
                i
            ));
        }
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Paragraph,
            max_chars: 200,
            overlap_chars: 30,
        };
        let chunks = chunk_text(&text, &opts);
        assert!(chunks.len() > 1, "should produce multiple chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(!c.is_empty(), "chunk should not be empty");
        }
    }

    #[test]
    fn chunk_by_char_single_chunk() {
        let text = "short";
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 1200,
            overlap_chars: 150,
        };
        let chunks = chunk_text(text, &opts);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short");
    }

    #[test]
    fn chunk_by_char_multi_chunk_no_overlap() {
        let text = "a".repeat(500);
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 200,
            overlap_chars: 0,
        };
        let chunks = chunk_text(&text, &opts);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 200);
        assert_eq!(chunks[1].len(), 200);
        assert_eq!(chunks[2].len(), 100);
    }

    #[test]
    fn chunk_by_char_multi_chunk_with_overlap() {
        let text = "a".repeat(500);
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 200,
            overlap_chars: 50,
        };
        let chunks = chunk_text(&text, &opts);
        assert!(chunks.len() >= 3);
        assert_eq!(chunks[0].len(), 200);
        assert_eq!(chunks[1].len(), 200);
        assert_eq!(chunks[2].len(), 200);
        assert_eq!(chunks[3].len(), 50);
    }

    #[test]
    fn chunk_by_char_overlap_boundary() {
        let text = "abcdefghijklmnopqrstuvwxyz";
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 10,
            overlap_chars: 3,
        };
        let chunks = chunk_text(text, &opts);
        assert!(chunks.len() >= 2);
        if chunks.len() >= 2 {
            assert!(
                chunks[1].starts_with(&chunks[0][chunks[0].len() - 3..]),
                "chunk[1] should start with the last 3 chars of chunk[0]: {:?} vs {:?}",
                &chunks[1][..3.min(chunks[1].len())],
                &chunks[0][chunks[0].len() - 3..]
            );
        }
    }

    #[test]
    fn chunk_by_char_handles_multibyte_characters() {
        // Every CJK character is 3 bytes, so a byte-based window would land
        // inside a character and panic.
        let text = "\u{6f22}".repeat(300);
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 200,
            overlap_chars: 50,
        };
        let chunks = chunk_text(&text, &opts);
        assert!(chunks.len() >= 2, "should produce multiple chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(!c.is_empty(), "chunk should not be empty");
            assert!(
                c.chars().count() <= 200,
                "chunk exceeded max_chars: {} chars",
                c.chars().count()
            );
        }
        assert_eq!(chunks[0].chars().count(), 200);
    }

    #[test]
    fn chunk_by_char_handles_4byte_characters() {
        // Emoji are 4 bytes each and byte offsets land inside them.
        let text = "\u{1f389}".repeat(250);
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 200,
            overlap_chars: 0,
        };
        let chunks = chunk_text(&text, &opts);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), 200);
        assert_eq!(chunks[1].chars().count(), 50);
    }

    #[test]
    fn chunk_by_paragraph_overlap_respects_char_boundaries() {
        // The overlap slice starts at current.len() - overlap_chars, which
        // falls inside a 3-byte character for CJK text.
        let para = "\u{5b57}".repeat(100);
        let mut text = String::new();
        for i in 0..8 {
            if i > 0 {
                text.push_str("\n\n");
            }
            text.push_str(&para);
        }
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Paragraph,
            max_chars: 250,
            overlap_chars: 100,
        };
        let chunks = chunk_text(&text, &opts);
        assert!(chunks.len() > 1, "should produce multiple chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(!c.is_empty(), "chunk should not be empty");
        }
    }

    #[test]
    fn empty_text_returns_empty() {
        let opts = ChunkOptions::default();
        let chunks = chunk_text("", &opts);
        assert!(chunks.is_empty());
    }

    #[test]
    fn paragraph_chunk_with_overlap() {
        let text = "Paragraph A.\n\nParagraph B.\n\nParagraph C.\n\nParagraph D.\n\nParagraph E.";
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Paragraph,
            max_chars: 30,
            overlap_chars: 10,
        };
        let chunks = chunk_text(text, &opts);
        assert!(chunks.len() > 1, "should produce multiple chunks, got {}", chunks.len());
        if chunks.len() >= 2 {
            assert!(
                chunks[1].contains("Paragraph"),
                "overlap should carry over text: {:?}",
                chunks[1]
            );
        }
    }
}
