use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{NovaError, NovaResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorHit {
    pub memory_uuid: Uuid,

    pub similarity: f32,
}

#[async_trait]
pub trait VectorStore: Send + Sync + std::fmt::Debug {
    fn dimensions(&self) -> usize;

    async fn insert_vector(
        &self,
        namespace_id: i64,
        memory_uuid: Uuid,
        provider: &str,
        model: &str,
        vec: &[f32],
    ) -> NovaResult<()>;

    async fn delete_vector(&self, namespace_id: i64, memory_uuid: Uuid) -> NovaResult<()>;

    /// Return up to `k` stored vectors of the namespace whose cosine
    /// similarity to `query` is at least `threshold`.
    ///
    /// Ordering guarantee: hits come back by descending similarity, and equal
    /// similarities are ordered by memory uuid. The backing query has no
    /// `ORDER BY`, so callers may rely on this ordering being stable across
    /// repeated calls for unchanged data.
    async fn knn_search(
        &self,
        namespace_id: i64,
        query: &[f32],
        k: usize,
        threshold: f32,
    ) -> NovaResult<Vec<VectorHit>>;
}

pub fn check_dims(dims: usize, v: &[f32]) -> NovaResult<()> {
    if v.len() != dims {
        return Err(NovaError::validation(format!(
            "embedding dims mismatch: expected {dims}, got {}",
            v.len()
        )));
    }

    if v.iter().any(|f| !f.is_finite()) {
        return Err(NovaError::validation("embedding contains NaN or non-finite float"));
    }
    Ok(())
}

pub fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub fn blob_to_vec(blob: &[u8]) -> NovaResult<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return Err(NovaError::storage_msg(format!(
            "corrupt vector blob: length {} not multiple of 4",
            blob.len()
        )));
    }
    let mut out = Vec::with_capacity(blob.len() / 4);
    for chunk in blob.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().map_err(|_| NovaError::storage_msg("corrupt chunk"))?;
        out.push(f32::from_le_bytes(arr));
    }
    Ok(out)
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        let x = a[i];
        let y = b[i];
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f32::EPSILON {
        return 0.0;
    }
    (dot / denom).clamp(-1.0, 1.0)
}

pub type SharedVectorStore = Arc<dyn VectorStore>;

pub const DEFAULT_DIMS: usize = 1536;

#[derive(Clone)]
pub struct SqliteVectorStore {
    pool: sqlx::SqlitePool,
    dims: usize,
}

impl std::fmt::Debug for SqliteVectorStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteVectorStore").field("dims", &self.dims).finish_non_exhaustive()
    }
}

impl SqliteVectorStore {
    pub fn new(pool: sqlx::SqlitePool, dims: usize) -> Self {
        Self {
            pool,
            dims,
        }
    }

    pub fn with_db(db: &crate::storage::Database, dims: usize) -> Self {
        Self::new(db.pool.clone(), dims)
    }

    pub fn default_dims(&self) -> usize {
        self.dims
    }
}

#[async_trait]
impl VectorStore for SqliteVectorStore {
    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn insert_vector(
        &self,
        namespace_id: i64,
        memory_uuid: Uuid,
        provider: &str,
        model: &str,
        vec: &[f32],
    ) -> NovaResult<()> {
        check_dims(self.dims, vec)?;
        if provider.trim().is_empty() {
            return Err(NovaError::validation("embedding.provider must not be empty"));
        }
        if model.trim().is_empty() {
            return Err(NovaError::validation("embedding.model must not be empty"));
        }

        let blob = vec_to_blob(vec);
        let now = chrono::Utc::now().timestamp();
        let mem_s = memory_uuid.to_string();
        let owns: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM memory_items WHERE uuid = ?1 AND namespace_id = ?2")
                .bind(&mem_s)
                .bind(namespace_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(NovaError::storage)?;
        if owns.is_none() {
            return Err(NovaError::not_found(format!("memory {memory_uuid} in namespace")));
        }
        sqlx::query(
            "INSERT INTO embeddings (memory_uuid, dims, provider, model, vec_blob, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(memory_uuid) DO UPDATE SET dims = \
             excluded.dims, provider = excluded.provider, model = excluded.model, vec_blob = \
             excluded.vec_blob, created_at = excluded.created_at",
        )
        .bind(&mem_s)
        .bind(self.dims as i64)
        .bind(provider.trim())
        .bind(model.trim())
        .bind(blob)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(NovaError::storage)?;
        Ok(())
    }

    async fn delete_vector(&self, namespace_id: i64, memory_uuid: Uuid) -> NovaResult<()> {
        let mem_s = memory_uuid.to_string();
        sqlx::query(
            "DELETE FROM embeddings WHERE memory_uuid = ?1 AND EXISTS (SELECT 1 FROM memory_items \
             m WHERE m.uuid = embeddings.memory_uuid AND m.namespace_id = ?2)",
        )
        .bind(&mem_s)
        .bind(namespace_id)
        .execute(&self.pool)
        .await
        .map_err(NovaError::storage)?;
        Ok(())
    }

    async fn knn_search(
        &self,
        namespace_id: i64,
        query: &[f32],
        k: usize,
        threshold: f32,
    ) -> NovaResult<Vec<VectorHit>> {
        check_dims(self.dims, query)?;
        let k = k.min(500);

        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT e.memory_uuid, e.vec_blob FROM embeddings e INNER JOIN memory_items m ON \
             m.uuid = e.memory_uuid WHERE e.dims = ?1 AND m.namespace_id = ?2",
        )
        .bind(self.dims as i64)
        .bind(namespace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(NovaError::storage)?;

        let mut hits: Vec<VectorHit> = Vec::with_capacity(rows.len().min(k * 2));
        for (mem_s, blob) in rows {
            let v = match blob_to_vec(&blob) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.len() != query.len() {
                continue;
            }
            let sim = cosine_similarity(query, &v);
            if sim >= threshold {
                let mem_uuid = match Uuid::parse_str(&mem_s) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                hits.push(VectorHit {
                    memory_uuid: mem_uuid,
                    similarity: sim,
                });
            }
        }

        // Break similarity ties on the memory uuid so the result order is
        // deterministic: the input order comes from a SQL query without
        // ORDER BY, which SQLite does not guarantee to be stable.
        //
        // A NaN similarity cannot be ordered by `partial_cmp`, and collapsing
        // it to `Equal` would also drop the uuid tie-break, so NaN is handled
        // explicitly as the lowest value and NaN pairs still fall back to the
        // uuid. The threshold above normally removes such hits already (a
        // stored vector containing NaN yields NaN, which never compares
        // greater than a threshold); the comparator stays total regardless.
        hits.sort_by(|a, b| match (a.similarity.is_nan(), b.similarity.is_nan()) {
            (true, true) => a.memory_uuid.cmp(&b.memory_uuid),
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => b
                .similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.memory_uuid.cmp(&b.memory_uuid)),
        });
        hits.truncate(k);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::StorageConfig,
        storage::{
            Database,
            memory::{InsertMemoryInput, MemoryRepository, SqliteMemoryRepository},
        },
    };

    async fn temp_db() -> Database {
        let dir = std::env::temp_dir().join(format!("yq-nova-m2-vec-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = StorageConfig {
            db_path: dir.join("test.db"),
            pool_max_connections: 2,
            pool_min_connections: 0,
            ..StorageConfig::default()
        };
        Database::open(cfg).await.expect("open temp db")
    }

    #[tokio::test]
    async fn insert_upsert_and_delete_vector() {
        let db = temp_db().await;
        let mem = SqliteMemoryRepository::new();
        let muuid = mem
            .insert(
                &db,
                InsertMemoryInput {
                    content: "x",
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .uuid();
        let store = SqliteVectorStore::with_db(&db, 4);
        let v = [0.1f32, 0.2, 0.3, 0.4];
        const NS: i64 = crate::storage::namespace::DEFAULT_NAMESPACE_ID;
        store.insert_vector(NS, muuid, "test", "m1", &v).await.unwrap();

        store.insert_vector(NS, muuid, "test2", "m2", &v).await.unwrap();
        store.delete_vector(NS, muuid).await.unwrap();

        let hits = store.knn_search(NS, &v, 10, -1.0).await.unwrap();
        assert!(hits.iter().all(|h| h.memory_uuid != muuid));
    }

    #[tokio::test]
    async fn knn_picks_most_similar_and_filters_threshold() {
        let db = temp_db().await;
        let mem = SqliteMemoryRepository::new();
        let store = SqliteVectorStore::with_db(&db, 3);
        const NS: i64 = crate::storage::namespace::DEFAULT_NAMESPACE_ID;

        let u0 = mem
            .insert(
                &db,
                InsertMemoryInput {
                    content: "m0",
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .uuid();
        store.insert_vector(NS, u0, "t", "m", &[1.0_f32, 0.0, 0.0]).await.unwrap();
        let rows: Vec<(String, Vec<u8>, i64)> =
            sqlx::query_as("SELECT memory_uuid, vec_blob, dims FROM embeddings")
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, 3);
        let decoded = blob_to_vec(&rows[0].1).unwrap();
        assert_eq!(decoded, vec![1.0_f32, 0.0, 0.0]);

        let u1 = mem
            .insert(
                &db,
                InsertMemoryInput {
                    content: "m1",
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .uuid();
        let u2 = mem
            .insert(
                &db,
                InsertMemoryInput {
                    content: "m2",
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .uuid();
        store.insert_vector(NS, u1, "t", "m", &[-1.0_f32, 0.0, 0.0]).await.unwrap();
        store.insert_vector(NS, u2, "t", "m", &[0.7_f32, 0.7, 0.0]).await.unwrap();

        let q = [1.0f32, 0.0, 0.0];

        let all3 = store.knn_search(NS, &q, 10, -2.0).await.unwrap();
        assert_eq!(all3.len(), 3, "all 3 vectors should pass threshold=-2, got {all3:?}");

        let nonneg = store.knn_search(NS, &q, 5, 0.0).await.unwrap();
        assert_eq!(nonneg.len(), 2, "nonneg count wrong: {nonneg:?}");
        let top = &nonneg[0];
        assert_eq!(top.memory_uuid, u0, "top should be u0, got sim={}", top.similarity);
        assert!(
            (top.similarity - 1.0).abs() < 1e-4,
            "u0 sim should be ~1.0, got {}",
            top.similarity
        );
        assert_eq!(nonneg[1].memory_uuid, u2);

        let hi = store.knn_search(NS, &q, 5, 0.9).await.unwrap();
        assert_eq!(hi.len(), 1);
        assert_eq!(hi[0].memory_uuid, u0);
    }

    #[tokio::test]
    async fn knn_ties_are_ordered_deterministically() {
        let db = temp_db().await;
        let mem = SqliteMemoryRepository::new();
        let store = SqliteVectorStore::with_db(&db, 3);
        const NS: i64 = crate::storage::namespace::DEFAULT_NAMESPACE_ID;

        // Five memories holding the exact same vector: every similarity is a
        // tie, so only the uuid tie-break can order the result. The backing
        // query has no ORDER BY, so without the tie-break the order follows
        // whatever row order SQLite happens to return.
        let mut uuids = Vec::new();
        let contents: Vec<String> = (0..5).map(|i| format!("tie {i}")).collect();
        for c in &contents {
            let u = mem
                .insert(
                    &db,
                    InsertMemoryInput {
                        content: c,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .uuid();
            store.insert_vector(NS, u, "t", "m", &[1.0_f32, 0.0, 0.0]).await.unwrap();
            uuids.push(u);
        }
        uuids.sort();

        let q = [1.0f32, 0.0, 0.0];
        let first = store.knn_search(NS, &q, 10, 0.0).await.unwrap();
        let second = store.knn_search(NS, &q, 10, 0.0).await.unwrap();
        let order: Vec<Uuid> = first.iter().map(|h| h.memory_uuid).collect();
        let order2: Vec<Uuid> = second.iter().map(|h| h.memory_uuid).collect();

        assert_eq!(first.len(), 5, "all tied vectors should be returned");
        assert_eq!(order, uuids, "ties should be ordered by uuid, got {order:?}");
        assert_eq!(order, order2, "repeated searches must return the same order");
        for h in &first {
            assert!((h.similarity - 1.0).abs() < 1e-4, "tied sim should be ~1.0");
        }
    }

    #[test]
    fn roundtrip_blob_identical() {
        let v: Vec<f32> = (0..16).map(|i| i as f32 * 0.1).collect();
        let blob = vec_to_blob(&v);
        assert_eq!(blob.len(), 64);
        let back = blob_to_vec(&blob).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn cosine_identical_vectors_are_one() {
        let a = [1.0f32, 2.0, 3.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = [1.0, 0.0, 0.0];
        let b = [0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_is_minus_one() {
        let a = [1.0, 0.0];
        let b = [-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn bad_dims_rejected() {
        let err = check_dims(4, &[1.0, 2.0, 3.0]).unwrap_err();
        assert_eq!(err.code(), crate::error::ErrorCode::Validation);
    }

    #[test]
    fn nan_rejected() {
        let err = check_dims(2, &[1.0, f32::NAN]).unwrap_err();
        assert_eq!(err.code(), crate::error::ErrorCode::Validation);
    }
    #[tokio::test]
    async fn knn_orders_by_descending_similarity_before_uuid() {
        let db = temp_db().await;
        let mem = SqliteMemoryRepository::new();
        let store = SqliteVectorStore::with_db(&db, 3);
        const NS: i64 = crate::storage::namespace::DEFAULT_NAMESPACE_ID;

        let query = [1.0f32, 0.0, 0.0];
        // vectors whose similarities differ clearly (1.0, ~0.707, 0.0)
        let rows: Vec<(&str, [f32; 3])> = vec![
            ("exact", [1.0, 0.0, 0.0]),
            ("diagonal", [1.0, 1.0, 0.0]),
            ("orthogonal", [0.0, 1.0, 0.0]),
        ];
        for (content, vec) in &rows {
            let u = mem
                .insert(
                    &db,
                    InsertMemoryInput {
                        content,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .uuid();
            store.insert_vector(NS, u, "t", "m", vec).await.unwrap();
        }

        let hits = store.knn_search(NS, &query, 10, 0.0).await.unwrap();
        let mut contents: Vec<String> = Vec::with_capacity(hits.len());
        for h in &hits {
            contents.push(mem.get_by_uuid(&db, NS, h.memory_uuid).await.unwrap().content);
        }
        assert_eq!(
            contents,
            vec!["exact".to_string(), "diagonal".to_string(), "orthogonal".to_string()],
            "differing similarities must drive the order, the uuid only breaks ties"
        );
        assert!(hits[0].similarity > hits[1].similarity);
        assert!(hits[1].similarity > hits[2].similarity);
    }

    #[tokio::test]
    async fn knn_similarities_stay_finite() {
        let db = temp_db().await;
        let mem = SqliteMemoryRepository::new();
        let store = SqliteVectorStore::with_db(&db, 3);
        const NS: i64 = crate::storage::namespace::DEFAULT_NAMESPACE_ID;

        let mut uuids = Vec::new();
        for content in ["usable", "zeroed"] {
            let u = mem
                .insert(
                    &db,
                    InsertMemoryInput {
                        content,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .uuid();
            uuids.push(u);
        }
        store.insert_vector(NS, uuids[0], "t", "m", &[1.0, 0.0, 0.0]).await.unwrap();
        store.insert_vector(NS, uuids[1], "t", "m", &[0.0, 0.0, 0.0]).await.unwrap();

        // a NaN vector cannot be stored, which is what keeps similarities
        // finite and NaN out of the ordering below
        let rejected =
            store.insert_vector(NS, uuids[0], "t", "m", &[f32::NAN, 0.0, 0.0]).await.unwrap_err();
        assert_eq!(rejected.code(), crate::error::ErrorCode::Validation);

        let hits = store.knn_search(NS, &[1.0f32, 0.0, 0.0], 10, 0.0).await.unwrap();
        assert!(hits.iter().all(|h| h.similarity.is_finite()));
        // a zero vector divides by zero in a naive implementation; it must
        // come back as a finite 0.0 similarity instead
        let zeroed = hits.iter().find(|h| h.memory_uuid == uuids[1]).expect("zero vector listed");
        assert_eq!(zeroed.similarity, 0.0);
        assert_eq!(hits[0].memory_uuid, uuids[0]);
    }
}
