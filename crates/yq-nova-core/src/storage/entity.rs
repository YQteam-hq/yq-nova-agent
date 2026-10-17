use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{
    error::{NovaError, NovaResult},
    storage::{Database, Repository},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id: i64,

    pub uuid: Uuid,

    pub namespace_id: i64,

    pub name: String,

    pub r#type: String,

    pub description: Option<String>,

    pub metadata: serde_json::Value,

    pub created_at: DateTime<Utc>,

    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertEntityInput<'a> {
    pub namespace_id: i64,

    pub name: &'a str,

    pub r#type: &'a str,

    pub description: Option<&'a str>,

    pub metadata: Option<&'a serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "uuid", rename_all = "snake_case")]
pub enum UpsertOutcome {
    Created(Uuid),

    Updated(Uuid),
}

impl UpsertOutcome {
    pub fn uuid(&self) -> Uuid {
        match self {
            UpsertOutcome::Created(u) | UpsertOutcome::Updated(u) => *u,
        }
    }
}

#[async_trait]
pub trait EntityRepository: Repository<EntityRecord> {
    async fn upsert(
        &self,
        db: &Database,
        input: UpsertEntityInput<'_>,
    ) -> NovaResult<UpsertOutcome>;

    async fn get_by_uuid(
        &self,
        db: &Database,
        namespace_id: i64,
        uuid: Uuid,
    ) -> NovaResult<EntityRecord>;

    async fn get_by_name_type(
        &self,
        db: &Database,
        namespace_id: i64,
        name: &str,
        r#type: &str,
    ) -> NovaResult<Option<EntityRecord>>;

    async fn delete(&self, db: &Database, namespace_id: i64, uuid: Uuid) -> NovaResult<()>;

    async fn find_by_name(
        &self,
        db: &Database,
        namespace_id: i64,
        name: &str,
    ) -> NovaResult<Vec<EntityRecord>>;

    async fn list(
        &self,
        db: &Database,
        namespace_id: i64,
        name_prefix: Option<&str>,
        type_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> NovaResult<Vec<EntityRecord>>;
}

#[derive(Clone)]
pub struct SqliteEntityRepository;

impl SqliteEntityRepository {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for SqliteEntityRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Repository<EntityRecord> for SqliteEntityRepository {
    fn name(&self) -> &'static str {
        "entity.sqlite"
    }
}

#[async_trait]
impl EntityRepository for SqliteEntityRepository {
    async fn upsert(
        &self,
        db: &Database,
        input: UpsertEntityInput<'_>,
    ) -> NovaResult<UpsertOutcome> {
        let name = input.name.trim();
        let r#type = input.r#type.trim();
        if name.is_empty() {
            return Err(NovaError::validation("entity.name must not be empty"));
        }
        if r#type.is_empty() {
            return Err(NovaError::validation("entity.type must not be empty"));
        }

        let pool = &db.pool;
        let now = Utc::now().timestamp();
        let metadata_json = input.metadata.cloned().unwrap_or_else(|| serde_json::json!({}));

        let existing: Option<(i64, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, uuid, description, metadata_json FROM entities WHERE name = ?1 AND type = \
             ?2 AND namespace_id = ?3",
        )
        .bind(name)
        .bind(r#type)
        .bind(input.namespace_id)
        .fetch_optional(pool)
        .await
        .map_err(NovaError::storage)?;

        if let Some((_id, existing_uuid_s, existing_desc, existing_meta_s)) = existing {
            let existing_uuid = Uuid::parse_str(&existing_uuid_s)
                .map_err(|e| NovaError::storage_msg(format!("bad entity uuid: {e}")))?;
            let new_desc = input.description.or(existing_desc.as_deref());
            let new_meta = if input.metadata.is_some() {
                &metadata_json
            } else {
                &serde_json::from_str::<serde_json::Value>(&existing_meta_s)
                    .map_err(|e| NovaError::storage_msg(format!("bad meta: {e}")))?
            };
            sqlx::query(
                "UPDATE entities SET description = ?1, metadata_json = ?2, updated_at = ?3 WHERE \
                 uuid = ?4 AND namespace_id = ?5",
            )
            .bind(new_desc)
            .bind(new_meta.to_string())
            .bind(now)
            .bind(existing_uuid.to_string())
            .bind(input.namespace_id)
            .execute(pool)
            .await
            .map_err(NovaError::storage)?;
            return Ok(UpsertOutcome::Updated(existing_uuid));
        }

        let uuid = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO entities (uuid, namespace_id, name, type, description, metadata_json, \
             created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(uuid.to_string())
        .bind(input.namespace_id)
        .bind(name)
        .bind(r#type)
        .bind(input.description)
        .bind(metadata_json.to_string())
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .map_err(NovaError::storage)?;
        Ok(UpsertOutcome::Created(uuid))
    }

    async fn get_by_uuid(
        &self,
        db: &Database,
        namespace_id: i64,
        uuid: Uuid,
    ) -> NovaResult<EntityRecord> {
        let row = sqlx::query(
            "SELECT id, uuid, namespace_id, name, type, description, metadata_json, created_at, \
             updated_at FROM entities WHERE uuid = ?1 AND namespace_id = ?2",
        )
        .bind(uuid.to_string())
        .bind(namespace_id)
        .fetch_optional(&db.pool)
        .await
        .map_err(NovaError::storage)?;
        let row = row.ok_or_else(|| NovaError::not_found(format!("entity {uuid}")))?;
        row_to_entity(&row)
    }

    async fn get_by_name_type(
        &self,
        db: &Database,
        namespace_id: i64,
        name: &str,
        r#type: &str,
    ) -> NovaResult<Option<EntityRecord>> {
        let row = sqlx::query(
            "SELECT id, uuid, namespace_id, name, type, description, metadata_json, created_at, \
             updated_at FROM entities WHERE name = ?1 AND type = ?2 AND namespace_id = ?3",
        )
        .bind(name)
        .bind(r#type)
        .bind(namespace_id)
        .fetch_optional(&db.pool)
        .await
        .map_err(NovaError::storage)?;
        match row {
            Some(r) => Ok(Some(row_to_entity(&r)?)),
            None => Ok(None),
        }
    }

    async fn delete(&self, db: &Database, namespace_id: i64, uuid: Uuid) -> NovaResult<()> {
        let res = sqlx::query("DELETE FROM entities WHERE uuid = ?1 AND namespace_id = ?2")
            .bind(uuid.to_string())
            .bind(namespace_id)
            .execute(&db.pool)
            .await
            .map_err(NovaError::storage)?;
        if res.rows_affected() == 0 {
            return Err(NovaError::not_found(format!("entity {uuid}")));
        }
        Ok(())
    }

    async fn find_by_name(
        &self,
        db: &Database,
        namespace_id: i64,
        name: &str,
    ) -> NovaResult<Vec<EntityRecord>> {
        let sql = "SELECT id, uuid, namespace_id, name, type, description, metadata_json, \
                   created_at, updated_at FROM entities WHERE LOWER(name) = LOWER(?1) AND \
                   namespace_id = ?2 ORDER BY updated_at DESC";
        let rows = sqlx::query(sql)
            .bind(name)
            .bind(namespace_id)
            .fetch_all(&db.pool)
            .await
            .map_err(NovaError::storage)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row_to_entity(&row)?);
        }
        Ok(out)
    }

    async fn list(
        &self,
        db: &Database,
        namespace_id: i64,
        name_prefix: Option<&str>,
        type_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> NovaResult<Vec<EntityRecord>> {
        let mut where_clauses: Vec<&str> = Vec::new();
        where_clauses.push("namespace_id = ?");
        if name_prefix.is_some() {
            where_clauses.push("name LIKE ? ESCAPE '\\'");
        }
        if type_filter.is_some() {
            where_clauses.push("type = ?");
        }
        let wc = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };
        let limit = limit.min(10_000) as i64;
        let offset = offset as i64;
        let sql = format!(
            "SELECT id, uuid, namespace_id, name, type, description, metadata_json, created_at, \
             updated_at FROM entities {wc} ORDER BY updated_at DESC LIMIT ? OFFSET ?"
        );
        let mut q = sqlx::query(&sql).bind(namespace_id);
        if let Some(np) = name_prefix {
            q = q.bind(format!("{}%", escape_like_prefix(np)));
        }
        if let Some(t) = type_filter {
            q = q.bind(t);
        }
        let rows =
            q.bind(limit).bind(offset).fetch_all(&db.pool).await.map_err(NovaError::storage)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row_to_entity(&row)?);
        }
        Ok(out)
    }
}

/// Escape `input` so it can be used as a literal prefix inside a SQLite
/// `LIKE` pattern.
///
/// SQLite only treats backslash as an escape character when the query
/// declares `ESCAPE '\'`; without it `\%` is a literal backslash followed by
/// "match anything", and `_` silently keeps acting as a single-character
/// wildcard. Both the escape character itself and the two wildcards are
/// therefore escaped here, and the query above declares the matching ESCAPE
/// clause.
fn escape_like_prefix(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn row_to_entity(row: &sqlx::sqlite::SqliteRow) -> NovaResult<EntityRecord> {
    let id: i64 = row.try_get("id").map_err(NovaError::storage)?;
    let uuid_s: String = row.try_get("uuid").map_err(NovaError::storage)?;
    let uuid =
        Uuid::parse_str(&uuid_s).map_err(|e| NovaError::storage_msg(format!("bad uuid: {e}")))?;
    let namespace_id: i64 = row.try_get("namespace_id").map_err(NovaError::storage)?;
    let name: String = row.try_get("name").map_err(NovaError::storage)?;
    let r#type: String = row.try_get("type").map_err(NovaError::storage)?;
    let description: Option<String> = row.try_get("description").map_err(NovaError::storage)?;
    let meta_s: String = row.try_get("metadata_json").map_err(NovaError::storage)?;
    let metadata: serde_json::Value = serde_json::from_str(&meta_s)
        .map_err(|e| NovaError::storage_msg(format!("entity meta json: {e}")))?;
    let created_ts: i64 = row.try_get("created_at").map_err(NovaError::storage)?;
    let updated_ts: i64 = row.try_get("updated_at").map_err(NovaError::storage)?;
    Ok(EntityRecord {
        id,
        uuid,
        namespace_id,
        name,
        r#type,
        description,
        metadata,
        created_at: ts_to_dt(created_ts)?,
        updated_at: ts_to_dt(updated_ts)?,
    })
}

fn ts_to_dt(ts: i64) -> NovaResult<DateTime<Utc>> {
    DateTime::from_timestamp(ts, 0).ok_or_else(|| NovaError::storage_msg(format!("bad ts {ts}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Out,
    In,
    #[default]
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraverseNode {
    pub entity: EntityRecord,

    pub depth: u8,

    pub path: Vec<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StorageConfig;

    const NS: i64 = crate::storage::namespace::DEFAULT_NAMESPACE_ID;

    async fn temp_db() -> Database {
        let dir = std::env::temp_dir().join(format!("yq-nova-m2-ent-{}", Uuid::new_v4()));
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
    async fn upsert_creates_then_updates_metadata_preserves_uuid() {
        let db = temp_db().await;
        let repo = SqliteEntityRepository::new();
        let first = repo
            .upsert(
                &db,
                UpsertEntityInput {
                    namespace_id: NS,
                    name: "Alice",
                    r#type: "person",
                    description: Some("desc 1"),
                    metadata: Some(&serde_json::json!({"role":"ceo"})),
                },
            )
            .await
            .unwrap();
        assert!(matches!(first, UpsertOutcome::Created(_)));
        let uuid = first.uuid();

        let second = repo
            .upsert(
                &db,
                UpsertEntityInput {
                    namespace_id: NS,
                    name: "Alice",
                    r#type: "person",
                    description: None,
                    metadata: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(second, UpsertOutcome::Updated(_)));
        assert_eq!(second.uuid(), uuid);

        let got = repo.get_by_uuid(&db, NS, uuid).await.unwrap();
        assert_eq!(got.description.as_deref(), Some("desc 1"));
        assert_eq!(got.metadata, serde_json::json!({"role":"ceo"}));
    }

    #[tokio::test]
    async fn upsert_rejects_blank_name_or_type() {
        let db = temp_db().await;
        let repo = SqliteEntityRepository::new();
        let err = repo
            .upsert(
                &db,
                UpsertEntityInput {
                    namespace_id: NS,
                    name: "  ",
                    r#type: "x",
                    description: None,
                    metadata: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), crate::error::ErrorCode::Validation);
        let err = repo
            .upsert(
                &db,
                UpsertEntityInput {
                    namespace_id: NS,
                    name: "x",
                    r#type: "",
                    description: None,
                    metadata: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), crate::error::ErrorCode::Validation);
    }

    #[tokio::test]
    async fn get_by_uuid_returns_not_found_for_missing() {
        let db = temp_db().await;
        let repo = SqliteEntityRepository::new();
        let err = repo.get_by_uuid(&db, NS, Uuid::new_v4()).await.unwrap_err();
        assert_eq!(err.code(), crate::error::ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn list_filters_by_name_prefix() {
        let db = temp_db().await;
        let repo = SqliteEntityRepository::new();
        for (n, t) in [("Alice", "person"), ("Bob", "person"), ("Acme", "org")] {
            repo.upsert(
                &db,
                UpsertEntityInput {
                    namespace_id: NS,
                    name: n,
                    r#type: t,
                    description: None,
                    metadata: None,
                },
            )
            .await
            .unwrap();
        }
        let list = repo.list(&db, NS, Some("Al"), None, 100, 0).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Alice");
    }

    #[tokio::test]
    async fn namespaces_are_isolated() {
        let db = temp_db().await;
        let repo = SqliteEntityRepository::new();
        let ns2 = 999i64;
        sqlx::query(
            "INSERT OR IGNORE INTO namespaces (id, uuid, name, description, config_json, \
             created_at, updated_at) VALUES (?1, ?2, ?3, NULL, '{}', 1, 1)",
        )
        .bind(ns2)
        .bind("00000000-0000-0000-0000-000000000999")
        .bind("ns-999")
        .execute(&db.pool)
        .await
        .unwrap();
        repo.upsert(
            &db,
            UpsertEntityInput {
                namespace_id: NS,
                name: "Shared",
                r#type: "person",
                description: None,
                metadata: None,
            },
        )
        .await
        .unwrap();
        let created = repo
            .upsert(
                &db,
                UpsertEntityInput {
                    namespace_id: ns2,
                    name: "Shared",
                    r#type: "person",
                    description: Some("other"),
                    metadata: Some(&serde_json::json!({"ns":2})),
                },
            )
            .await
            .unwrap();
        assert!(matches!(created, UpsertOutcome::Created(_)));
        let a = repo.get_by_name_type(&db, NS, "Shared", "person").await.unwrap().unwrap();
        let b = repo.get_by_name_type(&db, ns2, "Shared", "person").await.unwrap().unwrap();
        assert_ne!(a.uuid, b.uuid);
        assert_eq!(a.description.as_deref(), None);
        assert_eq!(b.description.as_deref(), Some("other"));
    }

    #[test]
    fn escape_like_prefix_escapes_wildcards_and_backslash() {
        assert_eq!(escape_like_prefix("plain"), "plain");
        assert_eq!(escape_like_prefix("100%"), "100\\%");
        assert_eq!(escape_like_prefix("a_c"), "a\\_c");
        assert_eq!(escape_like_prefix("a\\b"), "a\\\\b");
        assert_eq!(escape_like_prefix("%_\\"), "\\%\\_\\\\");
    }

    #[tokio::test]
    async fn name_prefix_treats_wildcards_literally() {
        let db = temp_db().await;
        let repo = SqliteEntityRepository::new();
        for name in ["100% done", "1000 items", "a_c", "abc", "plain"] {
            repo.upsert(
                &db,
                UpsertEntityInput {
                    namespace_id: NS,
                    name,
                    r#type: "concept",
                    description: None,
                    metadata: None,
                },
            )
            .await
            .unwrap();
        }

        let pct = repo.list(&db, NS, Some("100%"), None, 100, 0).await.unwrap();
        let names: Vec<&str> = pct.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["100% done"], "'%' must match literally, not as a wildcard");

        let underscore = repo.list(&db, NS, Some("a_c"), None, 100, 0).await.unwrap();
        let names: Vec<&str> = underscore.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a_c"], "'_' must match literally, not any single char");

        let plain = repo.list(&db, NS, Some("pla"), None, 100, 0).await.unwrap();
        assert_eq!(plain.len(), 1, "ordinary prefixes still match");
        assert_eq!(plain[0].name, "plain");
    }
}
