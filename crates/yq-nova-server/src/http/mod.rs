use axum::{
    Router,
    http::Method,
    routing::{get, patch, post},
};
use tower_http::{
    compression::CompressionLayer,
    cors::{Any, CorsLayer},
    limit::RequestBodyLimitLayer,
    trace::TraceLayer,
};

pub mod auth;
pub mod error;
pub mod graph;
pub mod memory;
pub mod meta;
pub mod namespaces;
pub mod state;
pub mod tags;

pub use error::{AppError, fallback_404};
pub use state::AppState;

pub type Result<T> = std::result::Result<T, AppError>;

pub fn build_router(state: AppState) -> Router {
    let methods =
        [Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS, Method::PATCH];

    let api_v1 = Router::new()
        .route("/health", get(meta::health))
        .route("/stats", get(meta::stats))
        .route("/memory/list", post(memory::list_memories))
        .route("/memory/remember", post(memory::remember))
        .route("/memory/remember-batch", post(memory::remember_batch))
        .route("/memory/recall", post(memory::recall))
        .route("/memory/forget", post(memory::forget))
        .route("/memory/export", post(memory::export_memories))
        .route("/memory/import", post(memory::import_memories))
        .route("/memory/merge", post(memory::merge_memories))
        .route(
            "/memory/:uuid",
            get(memory::get_memory).delete(memory::delete_memory).patch(memory::update_memory),
        )
        .route("/tags", get(tags::list_tags))
        .route("/tags/:name", patch(tags::rename_tag).delete(tags::delete_tag))
        .route("/graph/extract-and-link", post(graph::extract_and_link))
        .route("/graph/entities", post(graph::upsert_entity).get(graph::list_entities))
        .route("/graph/entities/merge", post(graph::merge_entities))
        .route("/graph/relations", post(graph::upsert_relation).get(graph::list_relations))
        .route("/graph/traverse", post(graph::traverse))
        .route("/namespaces", post(namespaces::create_namespace).get(namespaces::list_namespaces))
        .route(
            "/namespaces/:name",
            get(namespaces::get_namespace)
                .patch(namespaces::update_namespace)
                .delete(namespaces::delete_namespace),
        );

    let auth_state = state.clone();

    Router::new()
        .nest("/v1", api_v1)
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(auth_state, auth::auth_middleware))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(methods).allow_headers(Any))
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024))
        .layer(CompressionLayer::new().gzip(true).no_deflate().no_zstd())
        .layer(TraceLayer::new_for_http())
        .fallback(fallback_404)
}

#[cfg(test)]
mod relations_endpoint_tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;
    use yq_nova_core::{
        config::{ServerConfig, StorageConfig},
        graph::{GraphService, extractor::RegexWikiExtractor},
        memory::MemoryService,
        storage::{Database, Migrator},
    };

    use super::{AppState, build_router};

    async fn make_router() -> axum::Router {
        let dir = std::env::temp_dir().join(format!(
            "yq-nova-test-rel-list-{}-{}",
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
        build_router(state)
    }

    async fn send(
        router: &axum::Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().uri(uri).method(method);
        let request = match body {
            Some(v) => builder
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => {
                builder = builder.header("accept", "application/json");
                builder.body(Body::empty()).unwrap()
            },
        };
        let resp = router.clone().oneshot(request).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn create_entity(router: &axum::Router, name: &str) -> Uuid {
        let (status, body) =
            send(router, "POST", "/v1/graph/entities", Some(json!({"name": name, "type": "node"})))
                .await;
        assert_eq!(status, StatusCode::OK, "entity upsert failed: {body}");
        Uuid::parse_str(body["entity"]["uuid"].as_str().expect("entity uuid")).unwrap()
    }

    async fn link(router: &axum::Router, source: Uuid, target: Uuid) {
        let (status, body) = send(
            router,
            "POST",
            "/v1/graph/relations",
            Some(json!({
                "source_uuid": source.to_string(),
                "target_uuid": target.to_string(),
                "predicate": "knows",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "relation upsert failed: {body}");
    }

    #[tokio::test]
    async fn relations_endpoint_lists_relations_of_every_entity() {
        let router = make_router().await;
        let target = create_entity(&router, "Target").await;

        // more sources than any single page of entities, so a handler that
        // walks entities and stops after the first page loses some of them
        const SOURCES: usize = 55;
        for i in 0..SOURCES {
            let source = create_entity(&router, &format!("Source{i:03}")).await;
            link(&router, source, target).await;
        }

        let (status, body) = send(&router, "GET", "/v1/graph/relations?limit=500", None).await;
        assert_eq!(status, StatusCode::OK, "listing failed: {body}");
        let listed = body.as_array().expect("array of relations");
        assert_eq!(listed.len(), SOURCES, "every relation must be listed");

        // pages are disjoint and continue where the previous one stopped
        let (_, first_page) = send(&router, "GET", "/v1/graph/relations?limit=10", None).await;
        let (_, second_page) =
            send(&router, "GET", "/v1/graph/relations?limit=10&offset=10", None).await;
        let ids = |v: &Value| -> Vec<i64> {
            v.as_array()
                .expect("array")
                .iter()
                .map(|r| r["id"].as_i64().expect("relation id"))
                .collect()
        };
        let (first_ids, second_ids) = (ids(&first_page), ids(&second_page));
        assert_eq!(first_ids.len(), 10);
        assert_eq!(second_ids.len(), 10);
        assert!(
            first_ids.iter().all(|id| !second_ids.contains(id)),
            "consecutive pages must not overlap"
        );
        assert!(first_ids.iter().min() > second_ids.iter().max(), "rows come back newest first");

        let (_, tail) =
            send(&router, "GET", &format!("/v1/graph/relations?limit=10&offset={SOURCES}"), None)
                .await;
        assert!(tail.as_array().expect("array").is_empty(), "paging past the end is empty");

        // filtering by source still narrows the result
        let (_, filtered) = send(
            &router,
            "GET",
            &format!("/v1/graph/relations?source={}&limit=500", Uuid::nil()),
            None,
        )
        .await;
        assert!(filtered.as_array().expect("array").is_empty());
    }
}
