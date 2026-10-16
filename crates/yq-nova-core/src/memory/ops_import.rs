use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::MemoryService;
use crate::{
    error::{NovaError, NovaResult},
    storage::{MemorySource, memory::sha256_hex, vector::VectorStore},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    #[default]
    Skip,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportItem {
    pub content: String,
    pub uuid: Option<Uuid>,
    pub metadata: Option<Value>,
    pub importance: Option<f32>,
    pub source: Option<String>,
    pub tags: Option<Vec<String>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportError {
    pub index: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOutput {
    pub received: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub errors: Vec<ImportError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInput {
    #[serde(default = "default_namespace")]
    pub namespace_id: i64,
    pub items: Vec<ImportItem>,
    #[serde(default = "default_embed")]
    pub embed: bool,
    #[serde(default)]
    pub on_conflict: ConflictStrategy,
}

const fn default_namespace() -> i64 {
    crate::storage::namespace::DEFAULT_NAMESPACE_ID
}

const fn default_embed() -> bool {
    true
}

pub async fn import_memories(svc: &MemoryService, input: ImportInput) -> NovaResult<ImportOutput> {
    let ns = input.namespace_id;
    let received = input.items.len();
    let mut imported = 0usize;
    let mut duplicates = 0usize;
    let mut errors = Vec::new();

    for (i, item) in input.items.into_iter().enumerate() {
        let content = item.content.trim().to_string();
        if content.is_empty() {
            errors.push(ImportError {
                index: i,
                message: "content must not be empty".into(),
            });
            continue;
        }

        let importance = item.importance.unwrap_or(0.5);
        if !(0.0..=1.0).contains(&importance) || !importance.is_finite() {
            errors.push(ImportError {
                index: i,
                message: format!("importance must be in [0.0, 1.0], got {importance}"),
            });
            continue;
        }

        let source = match item.source.as_deref().unwrap_or("agent") {
            "agent" => MemorySource::Agent,
            "user" => MemorySource::User,
            "system" => MemorySource::System,
            "tool" => MemorySource::Tool,
            other => {
                errors.push(ImportError {
                    index: i,
                    message: format!("unknown source: {other}"),
                });
                continue;
            },
        };

        let metadata = item.metadata.unwrap_or(serde_json::json!({}));
        let tags = item.tags.unwrap_or_default();
        let expires_at = item.expires_at;
        let provided_uuid = item.uuid;

        let content_hash = sha256_hex(&content);

        let existing: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, uuid FROM memory_items WHERE content_hash = ?1 AND namespace_id = ?2 AND \
             status != 'deleted'",
        )
        .bind(&content_hash)
        .bind(ns)
        .fetch_optional(&svc.database.pool)
        .await
        .map_err(NovaError::storage)?;

        if existing.is_some() {
            duplicates += 1;
            continue;
        }

        if let Some(u) = provided_uuid {
            let uuid_exists: bool = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM memory_items WHERE uuid = ?1 AND namespace_id = ?2",
            )
            .bind(u.to_string())
            .bind(ns)
            .fetch_one(&svc.database.pool)
            .await
            .map_err(NovaError::storage)?
                > 0;

            if uuid_exists {
                duplicates += 1;
                continue;
            }
        }

        let insert_uuid = provided_uuid.unwrap_or_else(Uuid::new_v4);
        let now = Utc::now().timestamp();
        let expires_ts = expires_at.map(|t| t.timestamp());

        let result = sqlx::query(
            "INSERT INTO memory_items (uuid, namespace_id, content, content_hash, metadata_json, \
             source, importance, access_count, last_accessed, created_at, expires_at, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )
        .bind(insert_uuid.to_string())
        .bind(ns)
        .bind(&content)
        .bind(&content_hash)
        .bind(metadata.to_string())
        .bind(source.as_str())
        .bind(importance as f64)
        .bind(0_i64)
        .bind::<Option<i64>>(None)
        .bind(now)
        .bind(expires_ts)
        .bind("active")
        .execute(&svc.database.pool)
        .await;

        match result {
            Ok(_) => {
                if !tags.is_empty() {
                    if let Err(e) = crate::storage::memory::attach_tags(
                        &svc.database.pool,
                        ns,
                        insert_uuid,
                        &tags,
                    )
                    .await
                    {
                        // The row is stored, but without its tags. Report the
                        // partial failure instead of dropping it: callers that
                        // filter by tag would otherwise never learn why their
                        // imported memories are invisible.
                        errors.push(ImportError {
                            index: i,
                            message: format!("inserted but tagging failed: {e}"),
                        });
                    }
                }
                if input.embed {
                    let meta = svc.embedding.meta();
                    match svc.embedding.embed_one(&content).await {
                        Ok(vec) => {
                            if vec.len() != meta.dims {
                                errors.push(ImportError {
                                    index: i,
                                    message: format!(
                                        "inserted but not embedded: provider {} returned {} dims, \
                                         expected {}",
                                        meta.provider,
                                        vec.len(),
                                        meta.dims
                                    ),
                                });
                            } else if let Err(e) = svc
                                .vector_store
                                .insert_vector(ns, insert_uuid, &meta.provider, &meta.model, &vec)
                                .await
                            {
                                errors.push(ImportError {
                                    index: i,
                                    message: format!("inserted but vector storage failed: {e}"),
                                });
                            }
                        },
                        Err(e) => {
                            errors.push(ImportError {
                                index: i,
                                message: format!("inserted but embedding failed: {e}"),
                            });
                        },
                    }
                }
                imported += 1;
            },
            Err(e) => {
                errors.push(ImportError {
                    index: i,
                    message: format!("insert failed: {e}"),
                });
            },
        }
    }

    Ok(ImportOutput {
        received,
        imported,
        duplicates,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Uuid,
        config::StorageConfig,
        memory::ops_remember::{RememberInput, service_for_tests},
        storage::Database,
    };

    async fn temp_svc() -> crate::memory::MemoryService {
        let dir = std::env::temp_dir().join(format!("yq-nova-import-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = StorageConfig {
            db_path: dir.join("test.db"),
            pool_max_connections: 2,
            pool_min_connections: 0,
            ..StorageConfig::default()
        };
        let db = Database::open(cfg).await.unwrap();
        service_for_tests(db, 8, None)
    }

    #[tokio::test]
    async fn import_basic_items() {
        let svc = temp_svc().await;
        let items = vec![
            ImportItem {
                content: "first memory".into(),
                importance: Some(0.7),
                source: Some("user".into()),
                tags: Some(vec!["a".into()]),
                ..Default::default()
            },
            ImportItem {
                content: "second memory".into(),
                importance: Some(0.3),
                source: Some("agent".into()),
                ..Default::default()
            },
        ];
        let input = ImportInput {
            namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
            items,
            embed: false,
            on_conflict: ConflictStrategy::Skip,
        };
        let out = import_memories(&svc, input).await.unwrap();
        assert_eq!(out.received, 2);
        assert_eq!(out.imported, 2);
        assert_eq!(out.duplicates, 0);
        assert!(out.errors.is_empty());
    }

    #[tokio::test]
    async fn import_preserves_provided_uuid() {
        let svc = temp_svc().await;
        let custom_uuid = Uuid::new_v4();
        let items = vec![ImportItem {
            content: "memory with custom uuid".into(),
            uuid: Some(custom_uuid),
            importance: Some(0.5),
            ..Default::default()
        }];
        let input = ImportInput {
            namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
            items,
            embed: false,
            on_conflict: ConflictStrategy::Skip,
        };
        let out = import_memories(&svc, input).await.unwrap();
        assert_eq!(out.imported, 1);

        let mem = svc.get_memory(custom_uuid).await.unwrap();
        assert_eq!(mem.content, "memory with custom uuid");
    }

    #[tokio::test]
    async fn import_skips_duplicate_content() {
        let svc = temp_svc().await;
        svc.remember(RememberInput {
            content: "unique content",
            importance: 0.5,
            ..Default::default()
        })
        .await
        .unwrap();

        let items = vec![ImportItem {
            content: "unique content".into(),
            importance: Some(0.5),
            ..Default::default()
        }];
        let input = ImportInput {
            namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
            items,
            embed: false,
            on_conflict: ConflictStrategy::Skip,
        };
        let out = import_memories(&svc, input).await.unwrap();
        assert_eq!(out.imported, 0);
        assert_eq!(out.duplicates, 1);
    }

    #[tokio::test]
    async fn import_skips_existing_uuid() {
        let svc = temp_svc().await;
        let existing = svc
            .remember(RememberInput {
                content: "existing",
                importance: 0.5,
                ..Default::default()
            })
            .await
            .unwrap();

        let items = vec![ImportItem {
            content: "different content but same uuid".into(),
            uuid: Some(existing.uuid),
            importance: Some(0.5),
            ..Default::default()
        }];
        let input = ImportInput {
            namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
            items,
            embed: false,
            on_conflict: ConflictStrategy::Skip,
        };
        let out = import_memories(&svc, input).await.unwrap();
        assert_eq!(out.imported, 0);
        assert_eq!(out.duplicates, 1);
    }

    #[tokio::test]
    async fn import_partial_errors() {
        let svc = temp_svc().await;
        let items = vec![
            ImportItem {
                content: "valid item".into(),
                importance: Some(0.5),
                ..Default::default()
            },
            ImportItem {
                content: "".into(),
                importance: Some(0.5),
                ..Default::default()
            },
            ImportItem {
                content: "bad importance".into(),
                importance: Some(1.5),
                ..Default::default()
            },
            ImportItem {
                content: "another valid".into(),
                importance: Some(0.3),
                ..Default::default()
            },
        ];
        let input = ImportInput {
            namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
            items,
            embed: false,
            on_conflict: ConflictStrategy::Skip,
        };
        let out = import_memories(&svc, input).await.unwrap();
        assert_eq!(out.received, 4);
        assert_eq!(out.imported, 2);
        assert_eq!(out.duplicates, 0);
        assert_eq!(out.errors.len(), 2);
    }

    /// Reports `dims` in its metadata but returns vectors of a different
    /// length, which is what a misconfigured or mismatched provider does.
    #[derive(Debug)]
    struct MismatchedEmbedding {
        meta: crate::embedding::EmbeddingMeta,
        produced: usize,
    }

    #[async_trait::async_trait]
    impl crate::embedding::EmbeddingProvider for MismatchedEmbedding {
        fn meta(&self) -> &crate::embedding::EmbeddingMeta {
            &self.meta
        }

        async fn embed_batch(&self, texts: &[&str]) -> crate::error::NovaResult<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.0f32; self.produced]).collect())
        }
    }

    fn svc_with_embedding(
        db: Database,
        provider: &str,
        dims: usize,
        produced: usize,
    ) -> crate::memory::MemoryService {
        let shared: crate::embedding::SharedEmbeddingProvider =
            std::sync::Arc::new(MismatchedEmbedding {
                meta: crate::embedding::EmbeddingMeta {
                    provider: provider.to_string(),
                    model: format!("test-{dims}d"),
                    dims,
                },
                produced,
            });
        crate::memory::MemoryService::new(db, shared)
    }

    async fn temp_db() -> Database {
        let dir = std::env::temp_dir().join(format!("yq-nova-import-emb-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = StorageConfig {
            db_path: dir.join("test.db"),
            pool_max_connections: 2,
            pool_min_connections: 0,
            ..StorageConfig::default()
        };
        Database::open(cfg).await.unwrap()
    }

    #[tokio::test]
    async fn import_reports_embedding_dimension_mismatch() {
        let svc = svc_with_embedding(temp_db().await, "mismatched", 8, 4);
        let out = import_memories(
            &svc,
            ImportInput {
                namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
                items: vec![ImportItem {
                    content: "memory whose embedding is the wrong size".into(),
                    ..Default::default()
                }],
                embed: true,
                on_conflict: ConflictStrategy::Skip,
            },
        )
        .await
        .unwrap();

        // the row is stored, so it still counts as imported, but the caller
        // must learn that it was never embedded
        assert_eq!(out.imported, 1);
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert_eq!(out.errors[0].index, 0);
        assert!(
            out.errors[0].message.contains("not embedded"),
            "unexpected message: {}",
            out.errors[0].message
        );
    }

    #[tokio::test]
    async fn import_reports_vector_store_failures() {
        // a provider without a name is rejected by the vector store
        let svc = svc_with_embedding(temp_db().await, "", 4, 4);
        let out = import_memories(
            &svc,
            ImportInput {
                namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
                items: vec![ImportItem {
                    content: "memory the vector store refuses".into(),
                    ..Default::default()
                }],
                embed: true,
                on_conflict: ConflictStrategy::Skip,
            },
        )
        .await
        .unwrap();

        assert_eq!(out.imported, 1);
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert!(
            out.errors[0].message.contains("vector storage failed"),
            "unexpected message: {}",
            out.errors[0].message
        );
    }
    #[tokio::test]
    async fn import_reports_tag_attachment_failures() {
        let svc = temp_svc().await;
        // Rename the tag table so attaching tags fails while the memory insert
        // still succeeds, which is the partial failure this reports.
        sqlx::query("ALTER TABLE tags RENAME TO tags_hidden")
            .execute(&svc.database.pool)
            .await
            .unwrap();

        let out = import_memories(
            &svc,
            ImportInput {
                namespace_id: crate::storage::namespace::DEFAULT_NAMESPACE_ID,
                items: vec![ImportItem {
                    content: "memory whose tags cannot be stored".into(),
                    tags: Some(vec!["ops".into()]),
                    ..Default::default()
                }],
                embed: false,
                on_conflict: ConflictStrategy::Skip,
            },
        )
        .await
        .unwrap();

        assert_eq!(out.imported, 1, "the row is still stored");
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert_eq!(out.errors[0].index, 0);
        assert!(
            out.errors[0].message.contains("tagging failed"),
            "unexpected message: {}",
            out.errors[0].message
        );
    }
}
