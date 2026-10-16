use axum::{
    Json,
    extract::{Extension, Query, State},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use yq_nova_core::{
    graph::{
        GraphExtractOpts, GraphService, LinkResult, MergeEntitiesInput, MergeEntitiesOutput,
        TraverseOpts,
    },
    storage::{
        EntityRecord, EntityRepository, InsertRelationInput, InsertRelationOutcome, RelationRecord,
        RelationRepository, SqliteEntityRepository, SqliteRelationRepository, TraverseNode,
        UpsertEntityInput, UpsertOutcome,
    },
};

use crate::http::{AppState, Result, auth::NamespaceContext};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpsertEntityRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertEntityResponse {
    #[serde(flatten)]
    pub outcome: UpsertOutcome,
    pub entity: EntityRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListEntitiesQuery {
    pub name_prefix: Option<String>,
    pub entity_type: Option<String>,
    #[serde(default = "default_limit_200")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit_200() -> usize {
    200
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpsertRelationRequest {
    pub source_uuid: Uuid,
    pub target_uuid: Uuid,
    pub predicate: String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    pub metadata: Option<serde_json::Value>,
    #[serde(default = "default_true")]
    pub idempotent: bool,
    pub memory_uuid: Option<Uuid>,
}
fn default_confidence() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct UpsertRelationResponse {
    pub inserted: bool,
    pub updated: bool,
    pub relation_uuid: Uuid,
    pub relation: RelationRecord,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListRelationsQuery {
    pub source: Option<Uuid>,
    pub target: Option<Uuid>,
    pub predicate: Option<String>,
    #[serde(default = "default_limit_200")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TraverseRequest {
    pub start: Uuid,
    #[serde(default = "default_max_depth")]
    pub max_depth: u8,
    #[serde(default = "default_max_nodes")]
    pub max_nodes: usize,
    #[serde(default)]
    pub predicate_whitelist: Vec<String>,
    #[serde(default)]
    pub min_confidence: f32,
}
fn default_max_depth() -> u8 {
    3
}
fn default_max_nodes() -> usize {
    100
}

impl Default for TraverseRequest {
    fn default() -> Self {
        Self {
            start: Uuid::nil(),
            max_depth: default_max_depth(),
            max_nodes: default_max_nodes(),
            predicate_whitelist: vec![],
            min_confidence: 0.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ExtractLinkRequest {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub opts: GraphExtractOpts,
}

pub async fn upsert_entity(
    State(state): State<AppState>,
    Extension(ns): Extension<NamespaceContext>,
    Json(req): Json<UpsertEntityRequest>,
) -> Result<Json<UpsertEntityResponse>> {
    let repo = SqliteEntityRepository::new();
    let db = &state.db;
    let input = UpsertEntityInput {
        namespace_id: ns.id,
        name: req.name.trim(),
        r#type: req.entity_type.trim(),
        description: req.description.as_deref(),
        metadata: req.metadata.as_ref(),
    };
    let outcome = repo.upsert(db, input).await?;
    let entity = repo.get_by_uuid(db, ns.id, outcome.uuid()).await?;
    Ok(Json(UpsertEntityResponse {
        outcome,
        entity,
    }))
}

pub async fn list_entities(
    State(state): State<AppState>,
    Extension(ns): Extension<NamespaceContext>,
    Query(q): Query<ListEntitiesQuery>,
) -> Result<Json<Vec<EntityRecord>>> {
    let repo = SqliteEntityRepository::new();
    let db = &state.db;
    let limit = q.limit.min(500);

    let rows = repo
        .list(
            db,
            ns.id,
            q.name_prefix.as_deref(),
            q.entity_type.as_deref(),
            limit.saturating_add(q.offset),
            0,
        )
        .await?
        .into_iter()
        .skip(q.offset)
        .take(limit)
        .collect();
    Ok(Json(rows))
}

pub async fn upsert_relation(
    State(state): State<AppState>,
    Extension(ns): Extension<NamespaceContext>,
    Json(req): Json<UpsertRelationRequest>,
) -> Result<Json<UpsertRelationResponse>> {
    let repo = SqliteRelationRepository::new();
    let db = &state.db;
    let predicate = req.predicate.trim().to_string();
    let input = InsertRelationInput {
        namespace_id: ns.id,
        source_uuid: req.source_uuid,
        target_uuid: req.target_uuid,
        predicate: &predicate,
        confidence: req.confidence,
        memory_uuid: req.memory_uuid,
        metadata: req.metadata.as_ref(),
        idempotent: req.idempotent,
    };
    let outcome = repo.insert(db, input).await?;
    let relation_uuid = outcome.uuid();

    // Re-read the row this call wrote. Scanning the source entity's outgoing
    // relations cannot identify it reliably: the scan is capped, so a freshly
    // inserted low-confidence relation falls outside the window, and when
    // duplicates share the same source/predicate/target the lookup returns
    // whichever row sorts first instead of the one just written.
    let relation = repo.get_by_uuid(db, ns.id, relation_uuid).await?;

    let inserted = matches!(outcome, InsertRelationOutcome::Inserted(_));
    let updated = matches!(outcome, InsertRelationOutcome::Updated(_));
    Ok(Json(UpsertRelationResponse {
        inserted,
        updated,
        relation_uuid,
        relation,
    }))
}

pub async fn list_relations(
    State(state): State<AppState>,
    Extension(ns): Extension<NamespaceContext>,
    Query(q): Query<ListRelationsQuery>,
) -> Result<Json<Vec<RelationRecord>>> {
    let repo = SqliteRelationRepository::new();
    let db = &state.db;
    let limit = q.limit.min(500);
    let total_limit = limit.saturating_add(q.offset);

    let mut rows: Vec<RelationRecord> = Vec::new();

    if let Some(src) = q.source {
        let got = repo.list_outgoing(db, ns.id, src, q.predicate.as_deref(), total_limit).await?;
        if let Some(tgt) = q.target {
            rows.extend(got.into_iter().filter(|r| r.target_uuid == tgt));
        } else {
            rows = got;
        }
    } else if let Some(tgt) = q.target {
        let got = repo.list_incoming(db, ns.id, tgt, q.predicate.as_deref(), total_limit).await?;
        rows = got;
    } else {
        let ent_repo = SqliteEntityRepository::new();
        let ents = ent_repo.list(db, ns.id, None, None, 50, 0).await?;
        for e in ents {
            if rows.len() >= total_limit {
                break;
            }
            let mut got =
                repo.list_outgoing(db, ns.id, e.uuid, q.predicate.as_deref(), total_limit).await?;
            rows.append(&mut got);
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.id));
        rows.dedup_by_key(|r| r.id);
    }

    let trimmed: Vec<RelationRecord> = rows.into_iter().skip(q.offset).take(limit).collect();
    Ok(Json(trimmed))
}

pub async fn traverse(
    State(state): State<AppState>,
    Extension(ns): Extension<NamespaceContext>,
    Json(req): Json<TraverseRequest>,
) -> Result<Json<Vec<TraverseNode>>> {
    let svc: &GraphService = &state.graph;
    let opts = TraverseOpts {
        max_depth: req.max_depth,
        max_nodes: req.max_nodes,
        predicate_whitelist: req.predicate_whitelist,
        min_confidence: req.min_confidence,
    };
    let nodes = svc.traverse_graph(ns.id, req.start, opts).await?;
    Ok(Json(nodes))
}

pub async fn extract_and_link(
    State(state): State<AppState>,
    Extension(ns): Extension<NamespaceContext>,
    Json(req): Json<ExtractLinkRequest>,
) -> Result<Json<LinkResult>> {
    let svc: &GraphService = &state.graph;
    let out = svc.extract_and_link(ns.id, &req.text, &req.opts).await?;
    Ok(Json(out))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MergeEntitiesRequest {
    pub keep_uuid: Uuid,
    pub discard_uuids: Vec<Uuid>,
}

pub async fn merge_entities(
    State(state): State<AppState>,
    Extension(ns): Extension<NamespaceContext>,
    Json(req): Json<MergeEntitiesRequest>,
) -> Result<Json<MergeEntitiesOutput>> {
    let svc: &GraphService = &state.graph;
    let input = MergeEntitiesInput {
        keep_uuid: req.keep_uuid,
        discard_uuids: req.discard_uuids,
    };
    let out = svc.merge_entities(ns.id, input).await?;
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use yq_nova_core::{
        config::{ServerConfig, StorageConfig},
        graph::{GraphService, extractor::RegexWikiExtractor},
        memory::MemoryService,
        storage::{Database, Migrator},
    };

    use super::*;

    async fn make_router() -> axum::Router {
        let dir = std::env::temp_dir().join(format!(
            "yq-nova-test-graph-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let storage_cfg = StorageConfig {
            db_path: dir.join("nova.sqlite"),
            ..Default::default()
        };
        let db = Database::open(storage_cfg).await.expect("open db");
        Migrator::run(&db.pool).await.expect("migrations");

        let embed = yq_nova_core::embedding::MockEmbeddingProvider::new(16);
        let shared_embed: std::sync::Arc<dyn yq_nova_core::embedding::EmbeddingProvider> =
            std::sync::Arc::new(embed);
        let memory = MemoryService::new(db.clone(), shared_embed);
        let graph =
            GraphService::with_parts(db.clone(), std::sync::Arc::new(RegexWikiExtractor::new()));

        let state = AppState::new(ServerConfig::default(), db, memory, graph);
        crate::http::build_router(state)
    }

    async fn post_json(
        router: &axum::Router,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    async fn create_entity(router: &axum::Router, name: &str) -> Uuid {
        let (status, body) = post_json(
            router,
            "/v1/graph/entities",
            serde_json::json!({"name": name, "type": "test"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "entity upsert failed: {body}");
        Uuid::parse_str(body["entity"]["uuid"].as_str().expect("entity uuid")).unwrap()
    }

    async fn upsert_relation(
        router: &axum::Router,
        source: Uuid,
        target: Uuid,
        predicate: &str,
        confidence: f32,
    ) -> (StatusCode, serde_json::Value) {
        post_json(
            router,
            "/v1/graph/relations",
            serde_json::json!({
                "source_uuid": source.to_string(),
                "target_uuid": target.to_string(),
                "predicate": predicate,
                "confidence": confidence,
                "idempotent": false,
            }),
        )
        .await
    }

    #[tokio::test]
    async fn upsert_relation_returns_the_row_it_wrote() {
        let router = make_router().await;
        let alice = create_entity(&router, "Alice").await;
        let acme = create_entity(&router, "Acme").await;

        let (status, first) = upsert_relation(&router, alice, acme, "works_at", 1.0).await;
        assert_eq!(status, StatusCode::OK, "first upsert failed: {first}");

        // a second, non-idempotent write with the same key must still report
        // the row it created, not the higher-confidence sibling
        let (status, second) = upsert_relation(&router, alice, acme, "works_at", 0.4).await;
        assert_eq!(status, StatusCode::OK, "duplicate upsert failed: {second}");
        assert_eq!(
            second["relation_uuid"], second["relation"]["uuid"],
            "the response must describe the row that was written"
        );
        assert_ne!(
            first["relation"]["uuid"], second["relation"]["uuid"],
            "the second write created a distinct row"
        );
    }

    #[tokio::test]
    async fn upsert_relation_survives_many_siblings_with_the_same_predicate() {
        let router = make_router().await;
        let alice = create_entity(&router, "Alice").await;
        let acme = create_entity(&router, "Acme").await;

        for _ in 0..60 {
            let (status, body) = upsert_relation(&router, alice, acme, "visits", 1.0).await;
            assert_eq!(status, StatusCode::OK, "sibling upsert failed: {body}");
        }

        // the new row is the least confident one, so a capped scan of the
        // source entity's outgoing relations can no longer see it
        let (status, body) = upsert_relation(&router, alice, acme, "visits", 0.1).await;
        assert_eq!(status, StatusCode::OK, "low-confidence upsert failed: {body}");
        assert_eq!(body["relation_uuid"], body["relation"]["uuid"]);
    }
}
