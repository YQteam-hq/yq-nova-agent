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
                let take = overlap_chars.min(current.len());
                // `current` may contain multi-byte UTF-8 sequences, so move the
                // cut point forward to the next char boundary instead of
                // slicing at an arbitrary byte offset.
                let tail_start = ceil_char_boundary(&current, current.len() - take);
                current = current[tail_start..].to_string();
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
    let len = text.len();
    if len == 0 {
        return Vec::new();
    }
    if len <= max_chars {
        return vec![text.to_string()];
    }

    let step = max_chars.saturating_sub(overlap_chars);
    let step = if step == 0 { 1 } else { step };

    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0;
    while start < len {
        // Floor the cut point to a char boundary so a multi-byte UTF-8
        // sequence is never split across two chunks.
        let end = floor_char_boundary(text, (start + max_chars).min(len));
        if end <= start {
            // The character at `start` is wider than `max_chars`; skip past it
            // so the loop always makes progress.
            start = ceil_char_boundary(text, start + 1);
            continue;
        }
        chunks.push(text[start..end].to_string());
        // `step` is a byte count, so round the next start up to the following
        // char boundary instead of slicing from a mid-character offset.
        start = ceil_char_boundary(text, start + step);
    }

    chunks
}

/// Largest char boundary in `s` that is `<= i` (or `s.len()` when `i` is past
/// the end).
fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut i = i;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary in `s` that is `>= i` (or `s.len()` when `i` is past
/// the end).
fn ceil_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut i = i;
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
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
    fn empty_text_returns_empty() {
        let opts = ChunkOptions::default();
        let chunks = chunk_text("", &opts);
        assert!(chunks.is_empty());
    }

    #[test]
    fn char_chunks_never_split_multibyte_utf8() {
        // The arrow character is three bytes wide, so a byte-oriented cut at
        // max_chars lands inside a character and used to panic.
        let text: String = "\u{2192}".repeat(600);
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 200,
            overlap_chars: 30,
        };
        let chunks = chunk_text(&text, &opts);
        assert!(chunks.len() > 1, "expected multiple chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(!c.is_empty());
            assert!(c.len() <= 200, "chunk exceeds max_chars: {} bytes", c.len());
            assert_eq!(c.len() % 3, 0, "chunk must hold whole 3-byte characters");
        }
    }

    #[test]
    fn char_chunks_cover_ascii_boundary_exactly() {
        let text = "a".repeat(1000);
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 250,
            overlap_chars: 0,
        };
        let chunks = chunk_text(&text, &opts);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), 1000);
    }

    #[test]
    fn paragraph_overlap_never_splits_multibyte_utf8() {
        let mut text = String::new();
        for i in 0..20 {
            if i > 0 {
                text.push_str("\n\n");
            }
            text.push_str(&format!(
                "Paragraph {} contains arrow characters \u{2192}\u{2192}\u{2192} to exercise \
                 overlap cuts on multi-byte text.",
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
        assert!(chunks.len() > 1, "expected multiple chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(!c.is_empty());
            assert!(c.len() <= 200 + 30, "chunk exceeds max_chars + overlap: {} bytes", c.len());
        }
    }

    #[test]
    fn emoji_chunks_never_split_multibyte_utf8() {
        // Emoji are four bytes and frequently appear in agent memories.
        let text: String = "\u{1f389}".repeat(300);
        let opts = ChunkOptions {
            enabled: true,
            split_by: SplitBy::Char,
            max_chars: 200,
            overlap_chars: 0,
        };
        let chunks = chunk_text(&text, &opts);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert_eq!(c.chars().count() * 4, c.len(), "every chunk must hold whole emoji");
        }
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
