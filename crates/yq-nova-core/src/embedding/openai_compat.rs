use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use super::{
    EmbeddingMeta, EmbeddingProvider,
    retry::{RetryAction, RetryConfig, classify_http_status, with_retry},
    truncate_utf8,
};
use crate::error::NovaResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiCompatConfig {
    pub base_url: String,

    #[serde(default)]
    pub api_key: String,

    pub model: String,

    pub dims: usize,

    pub batch_size: usize,

    #[serde(with = "crate::config::duration_seconds")]
    pub request_timeout: Duration,

    #[serde(flatten)]
    pub retry: RetryConfig,
}

impl Default for OpenAiCompatConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "text-embedding-3-small".to_string(),
            dims: 1536,
            batch_size: 128,
            request_timeout: Duration::from_secs(15),
            retry: RetryConfig::default(),
        }
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingReqBody<'a> {
    model: &'a str,
    input: &'a [&'a str],
    encoding_format: &'static str,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct EmbeddingRespDataItem {
    #[serde_as(as = "serde_with::VecSkipError<_>")]
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResp {
    data: Vec<EmbeddingRespDataItem>,
}

pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    meta: EmbeddingMeta,
    config: OpenAiCompatConfig,
    endpoint: String,
}

impl std::fmt::Debug for OpenAiCompatProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatProvider")
            .field("meta", &self.meta)
            .field("base_url", &self.config.base_url)
            .field("model", &self.config.model)
            .field("batch_size", &self.config.batch_size)
            .finish_non_exhaustive()
    }
}

impl OpenAiCompatProvider {
    pub fn new(config: OpenAiCompatConfig) -> NovaResult<Self> {
        if config.model.trim().is_empty() {
            return Err(crate::error::NovaError::validation(
                "openai_compat: model must not be empty",
            ));
        }
        if config.dims == 0 {
            return Err(crate::error::NovaError::validation("openai_compat: dims must be > 0"));
        }
        if config.batch_size == 0 {
            return Err(crate::error::NovaError::validation(
                "openai_compat: batch_size must be > 0",
            ));
        }

        let base = config.base_url.trim_end_matches('/').to_string();
        let endpoint = format!("{base}/embeddings");

        let client = reqwest::Client::builder()
            .user_agent(concat!("yq-nova-agent/", env!("CARGO_PKG_VERSION")))
            .timeout(config.request_timeout)
            .build()
            .map_err(|e| crate::error::NovaError::embedding_msg(format!("build client: {e}")))?;

        let meta = EmbeddingMeta {
            provider: "openai_compat".into(),
            model: config.model.clone(),
            dims: config.dims,
        };
        Ok(Self {
            client,
            meta,
            config,
            endpoint,
        })
    }

    async fn run_once(&self, texts: &[&str]) -> NovaResult<Vec<Vec<f32>>> {
        let body = EmbeddingReqBody {
            model: &self.config.model,
            input: texts,
            encoding_format: "float",
        };

        let resp = with_retry(&self.config.retry, |_attempt| async {
            let mut req = self.client.post(self.endpoint.as_str()).json(&body);
            if !self.config.api_key.is_empty() {
                req = req.bearer_auth(&self.config.api_key);
            }
            let res = req.send().await.map_err(|e| {
                let action = if e.is_timeout() || e.is_connect() || e.is_request() {
                    RetryAction::Retry
                } else {
                    RetryAction::Fail
                };
                (action, anyhow::anyhow!("{e}"))
            })?;

            let status = res.status();
            if !status.is_success() {
                let text = res.text().await.unwrap_or_default();
                let snippet = truncate_utf8(&text, 400);
                return Err((
                    classify_http_status(status),
                    anyhow::anyhow!("HTTP {}: {}", status.as_u16(), snippet),
                ));
            }

            let parsed: EmbeddingResp = res
                .json()
                .await
                .map_err(|e| (RetryAction::Fail, anyhow::anyhow!("parse response: {e}")))?;
            Ok(parsed)
        })
        .await?;

        if resp.data.len() != texts.len() {
            return Err(crate::error::NovaError::embedding_msg(format!(
                "upstream returned {} embeddings for {} inputs",
                resp.data.len(),
                texts.len()
            )));
        }

        let expected_dims = self.config.dims;
        let mut out = Vec::with_capacity(resp.data.len());
        for item in resp.data {
            if item.embedding.len() != expected_dims {
                return Err(crate::error::NovaError::embedding_msg(format!(
                    "upstream returned dims={} expected {expected_dims}",
                    item.embedding.len()
                )));
            }
            out.push(item.embedding);
        }
        Ok(out)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiCompatProvider {
    fn meta(&self) -> &EmbeddingMeta {
        &self.meta
    }

    async fn embed_batch(&self, texts: &[&str]) -> NovaResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if texts.len() <= self.config.batch_size {
            return self.run_once(texts).await;
        }

        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(self.config.batch_size) {
            let refs: Vec<&str> = chunk.to_vec();
            let sub = self.run_once(&refs).await?;
            out.extend(sub);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn spawn_mock_server(body: String, status: u16) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 8192];
            let mut read = 0usize;
            loop {
                let n = socket.read(&mut buffer[read..]).await.unwrap();
                if n == 0 {
                    break;
                }
                read += n;
                if buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: \
                 {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn config_defaults_sane() {
        let c = OpenAiCompatConfig::default();
        assert_eq!(c.model, "text-embedding-3-small");
        assert_eq!(c.dims, 1536);
        assert_eq!(c.batch_size, 128);
        assert!(c.api_key.is_empty());
    }

    #[test]
    fn new_validates_required_fields() {
        let bad = OpenAiCompatConfig {
            model: "  ".into(),
            ..OpenAiCompatConfig::default()
        };
        assert!(matches!(
            OpenAiCompatProvider::new(bad).unwrap_err().code(),
            crate::error::ErrorCode::Validation
        ));

        let zero_dim = OpenAiCompatConfig {
            dims: 0,
            ..OpenAiCompatConfig::default()
        };
        assert!(matches!(
            OpenAiCompatProvider::new(zero_dim).unwrap_err().code(),
            crate::error::ErrorCode::Validation
        ));

        let zero_batch = OpenAiCompatConfig {
            batch_size: 0,
            ..OpenAiCompatConfig::default()
        };
        assert!(matches!(
            OpenAiCompatProvider::new(zero_batch).unwrap_err().code(),
            crate::error::ErrorCode::Validation
        ));
    }

    #[test]
    fn endpoint_strips_trailing_slash() {
        let cfg = OpenAiCompatConfig {
            base_url: "https://example.com/v1///".into(),
            model: "m".into(),
            dims: 8,
            batch_size: 4,
            ..OpenAiCompatConfig::default()
        };
        let p = OpenAiCompatProvider::new(cfg).unwrap();
        assert_eq!(p.endpoint, "https://example.com/v1/embeddings");
        assert_eq!(p.meta().provider, "openai_compat");
    }

    #[tokio::test]
    async fn multibyte_error_body_does_not_panic() {
        // 200 three-byte characters = 600 bytes; byte 400 lands inside a
        // character, so a naive `&body[..400]` would panic while an error
        // message is being built.
        let body = "中".repeat(200);
        assert!(body.len() > 400);
        assert!(!body.is_char_boundary(400));

        let (base, server) = spawn_mock_server(body, 400).await;
        let cfg = OpenAiCompatConfig {
            base_url: base,
            model: "m".into(),
            dims: 4,
            batch_size: 1,
            request_timeout: Duration::from_secs(5),
            retry: RetryConfig {
                max_attempts: 1,
                ..RetryConfig::default()
            },
            ..OpenAiCompatConfig::default()
        };
        let provider = OpenAiCompatProvider::new(cfg).unwrap();

        let err = provider.embed_batch(&["hello"]).await.unwrap_err();
        let message = err.to_string();
        assert!(message.contains("HTTP 400"), "unexpected error message: {message}");
        server.abort();
    }
}
