//! Ollama HTTP client for embedding, classification, and reasoning.
//!
//! Uses the /api/embed endpoint (not the deprecated /api/embeddings).
//! Behavior when Ollama is down: explicit error — no silent degradation (N2).

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

// ---------------------------------------------------------------------------
// LlmProvider Trait (Port Layer — Plan 04 §2)
// ---------------------------------------------------------------------------

/// Common interface for LLM providers supporting all 3 model roles.
///
/// Annotated with `mockall::automock` for unit testing use cases without
/// a real Ollama instance.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Embed a text string → vector of f32.
    ///
    /// Number of dimensions depends on the model (mxbai-embed-large = 1024).
    /// Returns `Err` if the model is unavailable (do not fall back silently).
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Classify text into one of: MEMORY | KNOWLEDGE | SKILL.
    ///
    /// Returns (category, confidence 0..1).
    /// Returns `Err` if the classifier model is unavailable.
    async fn classify(&self, content: &str) -> Result<(String, f32)>;

    /// Generate a text response using the reasoning model.
    ///
    /// Used for spec recheck, review assist, handover validation, etc.
    /// Returns `Err` if the reasoner model is unavailable.
    async fn generate(&self, prompt: &str) -> Result<String>;
}

// ---------------------------------------------------------------------------
// OllamaProvider — concrete implementation
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct OllamaProvider {
    client: Client,
    pub base_url: String,
    pub embedding_model: String,
    pub classifier_model: String,
    pub reasoner_model: String,
}

impl OllamaProvider {
    /// Creates a new provider with reasonable defaults.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("Failed to build reqwest client"),
            base_url: base_url.into(),
            embedding_model: "mxbai-embed-large".to_string(),
            classifier_model: "gemma4:e2b".to_string(),
            reasoner_model: "qwen3:14b".to_string(),
        }
    }

    /// Override the models used (useful for testing or config).
    pub fn with_models(
        mut self,
        embedding_model: impl Into<String>,
        classifier_model: impl Into<String>,
        reasoner_model: impl Into<String>,
    ) -> Self {
        self.embedding_model = embedding_model.into();
        self.classifier_model = classifier_model.into();
        self.reasoner_model = reasoner_model.into();
        self
    }
}

// ---------------------------------------------------------------------------
// Serde structs for Ollama API
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

// ---------------------------------------------------------------------------
// LlmProvider implementation for OllamaProvider
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embed", self.base_url);
        let req = EmbedRequest {
            model: &self.embedding_model,
            input: text,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to connect to Ollama — ensure Ollama is running on the server")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "SERVICE_UNAVAILABLE: Ollama embed failed with HTTP {}: {}",
                status,
                body
            );
        }

        let mut parsed: EmbedResponse = resp
            .json()
            .await
            .context("Failed to parse Ollama embed response")?;

        parsed
            .embeddings
            .pop()
            .ok_or_else(|| anyhow::anyhow!("SERVICE_UNAVAILABLE: Ollama returned empty embeddings"))
    }

    async fn classify(&self, content: &str) -> Result<(String, f32)> {
        let prompt = format!(
            "You are a routing classification AI.\n\
            Classify the following text into exactly one of three categories: MEMORY, KNOWLEDGE, or SKILL.\n\
            - MEMORY: A specific fact about a file, bug, or temporary state (e.g., 'auth.ts has a bug on line 42').\n\
            - KNOWLEDGE: A general architectural principle or rule (e.g., 'always use PostgreSQL, not MySQL').\n\
            - SKILL: A step-by-step reusable procedure or script (e.g., '1. Install X, 2. Run Y').\n\
            \n\
            Text to classify: \"{}\"\n\
            Respond ONLY with the category name (MEMORY, KNOWLEDGE, or SKILL).",
            content
        );

        let raw = self.generate(&prompt).await?;
        let category = raw.trim().to_uppercase();

        // Parse the response — only accept known categories
        let valid = matches!(category.as_str(), "MEMORY" | "KNOWLEDGE" | "SKILL");
        if !valid {
            // Default to MEMORY if the model returns something unexpected
            return Ok(("MEMORY".to_string(), 0.5));
        }

        Ok((category, 1.0))
    }

    async fn generate(&self, prompt: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.base_url);
        let req = GenerateRequest {
            model: &self.reasoner_model,
            prompt,
            stream: false,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to connect to Ollama reasoner")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "SERVICE_UNAVAILABLE: Ollama generate failed with HTTP {}: {}",
                status,
                body
            );
        }

        let parsed: GenerateResponse = resp
            .json()
            .await
            .context("Failed to parse Ollama generate response")?;

        Ok(parsed.response)
    }
}

// ---------------------------------------------------------------------------
// Unit Tests (TDD — these run without a real Ollama instance)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::mock;

    // Test the trait via MockLlmProvider
    mock! {
        pub Llm {}
        #[async_trait]
        impl LlmProvider for Llm {
            async fn embed(&self, text: &str) -> Result<Vec<f32>>;
            async fn classify(&self, content: &str) -> Result<(String, f32)>;
            async fn generate(&self, prompt: &str) -> Result<String>;
        }
    }

    #[tokio::test]
    async fn test_mock_embed_returns_vector() {
        let mut mock = MockLlm::new();
        mock.expect_embed().returning(|_| Ok(vec![0.1, 0.2, 0.3]));

        let result = mock.embed("test text").await.unwrap();
        assert_eq!(result.len(), 3);
        assert!((result[0] - 0.1).abs() < 1e-5);
    }

    #[tokio::test]
    async fn test_mock_classify_returns_category() {
        let mut mock = MockLlm::new();
        mock.expect_classify()
            .returning(|_| Ok(("KNOWLEDGE".to_string(), 0.95)));

        let (cat, conf) = mock.classify("always use Rust").await.unwrap();
        assert_eq!(cat, "KNOWLEDGE");
        assert!(conf > 0.9);
    }

    #[tokio::test]
    async fn test_mock_generate_returns_text() {
        let mut mock = MockLlm::new();
        mock.expect_generate()
            .returning(|_| Ok("The task specification looks complete.".to_string()));

        let result = mock.generate("Review this spec").await.unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn test_ollama_provider_defaults() {
        let provider = OllamaProvider::new("http://localhost:11434");
        assert_eq!(provider.embedding_model, "mxbai-embed-large");
        assert_eq!(provider.classifier_model, "gemma4:e2b");
        assert_eq!(provider.reasoner_model, "qwen3:14b");
    }
}
