//! Agentic Chunking Algorithm (Plan 04 §4).
//!
//! Splits text preserving logical context boundaries (code blocks, markdown sections),
//! forming a Parent-Child hierarchy for retrieval:
//! - Search on Child chunks (smaller, precise)
//! - Return Parent context (larger, complete)

/// Approximate token count (character/4 heuristic, good enough for chunking decisions).
pub fn token_count(text: &str) -> usize {
    (text.len() + 3) / 4
}

/// A text chunk with parent-child relationship for hierarchical retrieval.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub id: String,
    pub content: String,
    pub is_parent: bool,
    pub parent_id: Option<String>,
}

/// Splits text into a hierarchical Parent-Child chunk structure.
///
/// - Parents: ~1024 tokens (full context window)
/// - Children: ~200 tokens (search precision)
pub fn agentic_chunking(text: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();

    // Step 1: Structural splitting by blank lines or markdown headers
    let initial_splits: Vec<&str> = split_structurally(text);

    if initial_splits.is_empty() {
        return chunks;
    }

    let parent_token_limit = 1024;
    let child_token_limit = 200;

    let mut current_parent = String::new();
    let mut child_texts: Vec<String> = Vec::new();
    let mut current_child = String::new();

    let flush = |parent: &str, children: &[String], chunks: &mut Vec<Chunk>| {
        if parent.is_empty() {
            return;
        }
        let parent_id = uuid::Uuid::new_v4().to_string();
        chunks.push(Chunk {
            id: parent_id.clone(),
            content: parent.trim().to_string(),
            is_parent: true,
            parent_id: None,
        });
        for child in children {
            if !child.trim().is_empty() {
                chunks.push(Chunk {
                    id: uuid::Uuid::new_v4().to_string(),
                    content: child.trim().to_string(),
                    is_parent: false,
                    parent_id: Some(parent_id.clone()),
                });
            }
        }
    };

    for split in &initial_splits {
        let split_tokens = token_count(split);

        if token_count(&current_parent) + split_tokens <= parent_token_limit {
            // Accumulate into current parent
            if !current_parent.is_empty() {
                current_parent.push_str("\n\n");
            }
            current_parent.push_str(split);

            // Also accumulate into current child
            if token_count(&current_child) + split_tokens <= child_token_limit {
                if !current_child.is_empty() {
                    current_child.push('\n');
                }
                current_child.push_str(split);
            } else {
                // Flush current child and start new one
                if !current_child.is_empty() {
                    child_texts.push(current_child.clone());
                }
                current_child = split.to_string();
            }
        } else {
            // Parent limit reached — flush current parent + children
            if !current_child.is_empty() {
                child_texts.push(current_child.clone());
                current_child = String::new();
            }
            flush(&current_parent, &child_texts, &mut chunks);

            // Start new parent
            current_parent = split.to_string();
            child_texts.clear();
            current_child = split.to_string();
        }
    }

    // Flush the last batch
    if !current_child.is_empty() {
        child_texts.push(current_child);
    }
    flush(&current_parent, &child_texts, &mut chunks);

    chunks
}

/// Splits text at blank lines and markdown headers (##, ###, #).
fn split_structurally(text: &str) -> Vec<&str> {
    // Split by double newlines (paragraph boundary)
    let mut result = Vec::new();
    let mut start = 0;

    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Match \n\n or \r\n\r\n
        if bytes[i] == b'\n' && i + 1 < len && bytes[i + 1] == b'\n' {
            let segment = text[start..i].trim();
            if !segment.is_empty() {
                result.push(segment);
            }
            i += 2;
            start = i;
            continue;
        }
        // Also split on markdown headers at line start
        if (i == 0 || bytes[i - 1] == b'\n') && bytes[i] == b'#' {
            if i > start {
                let segment = text[start..i].trim();
                if !segment.is_empty() {
                    result.push(segment);
                }
                start = i;
            }
        }
        i += 1;
    }

    // Remaining text
    let last = text[start..].trim();
    if !last.is_empty() {
        result.push(last);
    }

    result
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_count_basic() {
        assert_eq!(token_count(""), 0);
        assert_eq!(token_count("test"), 1);
        assert_eq!(token_count("hello world"), 3); // 11 chars → (11+3)/4 = 3
    }

    #[test]
    fn test_agentic_chunking_empty() {
        let chunks = agentic_chunking("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_agentic_chunking_short_text_creates_parent_child() {
        let text = "First paragraph here.\n\nSecond paragraph here.";
        let chunks = agentic_chunking(text);

        // Should have at least 1 parent and 1+ children
        assert!(!chunks.is_empty());
        let parents: Vec<_> = chunks.iter().filter(|c| c.is_parent).collect();
        let children: Vec<_> = chunks.iter().filter(|c| !c.is_parent).collect();

        assert!(!parents.is_empty(), "Should have parent chunks");
        // All children should reference a valid parent
        for child in &children {
            assert!(child.parent_id.is_some(), "Child must have a parent_id");
            let pid = child.parent_id.as_ref().unwrap();
            assert!(
                parents.iter().any(|p| &p.id == pid),
                "Child's parent_id must match an existing parent"
            );
        }
    }

    #[test]
    fn test_agentic_chunking_large_text_splits_parents() {
        // Generate text that exceeds parent_token_limit (1024 tokens ≈ 4096 chars)
        let paragraph = "Lorem ipsum dolor sit amet consectetur adipiscing elit. ".repeat(20);
        let text = (0..20)
            .map(|_| paragraph.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let chunks = agentic_chunking(&text);
        let parents: Vec<_> = chunks.iter().filter(|c| c.is_parent).collect();

        assert!(
            parents.len() >= 2,
            "Large text should create multiple parent chunks, got {}",
            parents.len()
        );
    }

    #[test]
    fn test_agentic_chunking_markdown_headers_split() {
        let text = "# Section 1\nContent of section 1.\n\n## Section 2\nContent of section 2.\n\n### Section 3\nContent of section 3.";
        let chunks = agentic_chunking(text);

        assert!(!chunks.is_empty());
        // Every parent chunk should have non-empty content
        for chunk in chunks.iter().filter(|c| c.is_parent) {
            assert!(!chunk.content.is_empty());
        }
    }

    #[test]
    fn test_chunk_ids_are_unique() {
        let text = "Para 1.\n\nPara 2.\n\nPara 3.\n\nPara 4.\n\nPara 5.";
        let chunks = agentic_chunking(text);

        let ids: std::collections::HashSet<_> = chunks.iter().map(|c| &c.id).collect();
        assert_eq!(ids.len(), chunks.len(), "All chunk IDs must be unique");
    }
}
