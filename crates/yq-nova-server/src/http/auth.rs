use axum::{
    Json,
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use yq_nova_core::storage::{NamespaceRepository, SqliteNamespaceRepository};

use crate::http::{AppState, error::ErrorBody};

static NAMESPACE_HEADER: &str = "x-namespace";

#[derive(Debug, Clone, Copy)]
pub struct NamespaceContext {
    pub id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientScope {
    Admin,
    Tenant,
}

pub async fn tenant_ns(
    db: &yq_nova_core::storage::Database,
    ns_id: i64,
) -> yq_nova_core::error::NovaResult<yq_nova_core::storage::namespace::NamespaceRecord> {
    let repo = SqliteNamespaceRepository::new();
    repo.get_by_id(db, ns_id).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn bearer_token(headers: &axum::http::HeaderMap) -> &str {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let trimmed = v.trim();
            if let Some(rest) = trimmed.strip_prefix("Bearer ") {
                rest.trim()
            } else if let Some(rest) = trimmed.strip_prefix("bearer ") {
                rest.trim()
            } else {
                trimmed
            }
        })
        .unwrap_or("")
}

fn header_namespace(req: &Request<Body>) -> Option<&str> {
    req.headers()
        .get(NAMESPACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

fn unauthorized(message: &str) -> Response {
    let body = ErrorBody {
        code: "forbidden",
        message: message.to_string(),
        trace_id: None,
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

fn not_found(message: String) -> Response {
    let body = ErrorBody {
        code: "not_found",
        message,
        trace_id: None,
    };
    (StatusCode::NOT_FOUND, Json(body)).into_response()
}

fn internal(e: yq_nova_core::error::NovaError) -> Response {
    let body = ErrorBody {
        code: "internal_error",
        message: e.to_string(),
        trace_id: None,
    };
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let provided = bearer_token(req.headers()).to_string();
    let ns_repo = SqliteNamespaceRepository::new();
    let configured = state.server_cfg.auth_token.clone();
    let per_ns_keys = state.server_cfg.namespace_keys.clone();

    if !per_ns_keys.is_empty()
        && (configured.is_empty()
            || provided.is_empty()
            || !constant_time_eq(configured.as_bytes(), provided.as_bytes()))
    {
        let bound = per_ns_keys.iter().find(|(_, key)| {
            !key.is_empty()
                && !provided.is_empty()
                && constant_time_eq(key.as_bytes(), provided.as_bytes())
        });
        let Some((ns_name, _)) = bound else {
            return unauthorized("unauthorized: missing or invalid namespace API key");
        };
        return match ns_repo.get_by_name(&state.db, ns_name).await {
            Ok(Some(ns)) => {
                req.extensions_mut().insert(NamespaceContext {
                    id: ns.id,
                });
                req.extensions_mut().insert(ClientScope::Tenant);
                next.run(req).await
            },
            Ok(None) => not_found(format!("namespace '{ns_name}' not found")),
            Err(e) => internal(e),
        };
    }

    if configured.is_empty() {
        let name = header_namespace(&req).map(str::to_string).unwrap_or_else(|| {
            yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_NAME.to_string()
        });
        return match ns_repo.get_by_name(&state.db, &name).await {
            Ok(Some(ns)) => {
                req.extensions_mut().insert(NamespaceContext {
                    id: ns.id,
                });
                req.extensions_mut().insert(ClientScope::Admin);
                next.run(req).await
            },
            Ok(None) => not_found(format!("namespace '{name}' not found")),
            Err(e) => internal(e),
        };
    }

    if provided.is_empty() || !constant_time_eq(configured.as_bytes(), provided.as_bytes()) {
        return unauthorized("unauthorized: invalid or missing API token");
    }
    let name = header_namespace(&req)
        .map(str::to_string)
        .unwrap_or_else(|| yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_NAME.to_string());
    match ns_repo.get_by_name(&state.db, &name).await {
        Ok(Some(ns)) => {
            req.extensions_mut().insert(NamespaceContext {
                id: ns.id,
            });
            req.extensions_mut().insert(ClientScope::Admin);
            next.run(req).await
        },
        Ok(None) => not_found(format!("namespace '{name}' not found")),
        Err(e) => internal(e),
    }
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
        graph::extractor::RegexWikiExtractor,
        memory::MemoryService,
        storage::{Database, Migrator},
    };

    use super::*;

    async fn make_router(auth_token: &str) -> axum::Router {
        make_router_full(auth_token, std::collections::BTreeMap::new(), &[]).await
    }

    async fn make_router_full(
        auth_token: &str,
        namespace_keys: std::collections::BTreeMap<String, String>,
        seed_namespaces: &[&str],
    ) -> axum::Router {
        let (router, _db) =
            make_router_full_with_db(auth_token, namespace_keys, seed_namespaces).await;
        router
    }

    async fn make_router_full_with_db(
        auth_token: &str,
        namespace_keys: std::collections::BTreeMap<String, String>,
        seed_namespaces: &[&str],
    ) -> (axum::Router, yq_nova_core::storage::Database) {
        let dir = std::env::temp_dir().join(format!(
            "yq-nova-test-auth-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let db_path = dir.join("nova.sqlite");
        let storage_cfg = StorageConfig {
            db_path,
            ..Default::default()
        };
        let db = Database::open(storage_cfg.clone()).await.expect("open db");
        Migrator::run(&db.pool).await.expect("migrations");

        for name in seed_namespaces {
            yq_nova_core::storage::SqliteNamespaceRepository::new()
                .create(
                    &db,
                    yq_nova_core::storage::namespace::CreateNamespaceInput {
                        name,
                        description: None,
                        config: None,
                    },
                )
                .await
                .expect("seed namespace");
        }

        let embed = std::sync::Arc::new(yq_nova_core::embedding::MockEmbeddingProvider::new(16));
        let memory = MemoryService::new(db.clone(), embed);
        let graph = yq_nova_core::graph::GraphService::with_parts(
            db.clone(),
            std::sync::Arc::new(RegexWikiExtractor::new()),
        );

        let srv_cfg = ServerConfig {
            auth_token: auth_token.into(),
            namespace_keys,
            ..Default::default()
        };
        let state = AppState::new(srv_cfg, db.clone(), memory, graph);
        (crate::http::build_router(state), db)
    }

    async fn post_json(
        router: axum::Router,
        uri: &str,
        token: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value =
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn ns_id(db: &yq_nova_core::storage::Database, name: &str) -> i64 {
        yq_nova_core::storage::SqliteNamespaceRepository::new()
            .get_by_name(db, name)
            .await
            .expect("query namespace")
            .expect("namespace exists")
            .id
    }

    const HEALTH: &str = "/v1/health";

    #[tokio::test]
    async fn missing_auth_header_returns_401() {
        let router = make_router("secret123").await;
        let req = Request::builder().uri(HEALTH).method("GET").body(Body::empty()).unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "forbidden");
        assert_eq!(body["message"], "unauthorized: invalid or missing API token");
    }

    #[tokio::test]
    async fn wrong_token_returns_401() {
        let router = make_router("secret123").await;
        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn correct_bearer_token_passes() {
        let router = make_router("secret123").await;
        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer secret123")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn empty_token_disables_auth() {
        let router = make_router("").await;
        let req = Request::builder().uri(HEALTH).method("GET").body(Body::empty()).unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn namespace_key_required_and_bound_to_tenant() {
        let mut keys = std::collections::BTreeMap::new();
        keys.insert("tenant-a".to_string(), "key-a".to_string());
        keys.insert("tenant-b".to_string(), "key-b".to_string());
        let router = make_router_full("", keys, &["tenant-a", "tenant-b"]).await;

        let req = Request::builder().uri(HEALTH).method("GET").body(Body::empty()).unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer no-such-key")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer key-a")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer key-b")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn namespace_key_to_missing_namespace_returns_404() {
        let mut keys = std::collections::BTreeMap::new();
        keys.insert("ghost".to_string(), "ghost-key".to_string());
        let router = make_router_full("", keys, &[]).await;
        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer ghost-key")
            .body(Body::empty())
            .unwrap();
        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn global_token_works_alongside_namespace_keys() {
        let mut keys = std::collections::BTreeMap::new();
        keys.insert("tenant-a".to_string(), "key-a".to_string());
        let router = make_router_full("global-secret", keys, &["tenant-a"]).await;

        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer global-secret")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer key-a")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let req = Request::builder()
            .uri(HEALTH)
            .method("GET")
            .header("authorization", "Bearer neither")
            .body(Body::empty())
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    async fn two_tenant_router() -> (axum::Router, yq_nova_core::storage::Database) {
        let mut keys = std::collections::BTreeMap::new();
        keys.insert("tenant-a".to_string(), "key-a".to_string());
        keys.insert("tenant-b".to_string(), "key-b".to_string());
        make_router_full_with_db("", keys, &["tenant-a", "tenant-b"]).await
    }

    #[tokio::test]
    async fn recall_filter_cannot_escape_authenticated_namespace() {
        let (router, db) = two_tenant_router().await;
        let tenant_a_id = ns_id(&db, "tenant-a").await;

        // tenant-a stores a memory only it should ever see.
        let (status, out) = post_json(
            router.clone(),
            "/v1/memory/remember",
            "key-a",
            serde_json::json!({"content": "tenant-a confidential quarterly plan"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let secret = out["uuid"].as_str().expect("uuid in response").to_string();

        // tenant-a can recall its own memory normally.
        let (status, out) = post_json(
            router.clone(),
            "/v1/memory/recall",
            "key-a",
            serde_json::json!({
                "query": "quarterly plan",
                "top_k": 10,
                "score_threshold": 0.0,
                "similarity_threshold": -1.0,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            out["hits"].as_array().unwrap().iter().any(|h| h["memory"]["uuid"] == secret),
            "tenant-a should recall its own memory"
        );

        // tenant-b forges filter.namespace_id to point at tenant-a: the
        // authenticated namespace must still win, so nothing leaks.
        let (status, out) = post_json(
            router.clone(),
            "/v1/memory/recall",
            "key-b",
            serde_json::json!({
                "query": "quarterly plan",
                "top_k": 10,
                "score_threshold": 0.0,
                "similarity_threshold": -1.0,
                "filter": {"namespace_id": tenant_a_id},
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let hits_empty = out["hits"].as_array().map(|h| h.is_empty()).unwrap_or(true);
        assert!(
            hits_empty,
            "forged filter.namespace_id must not return tenant-a memories, got: {out}"
        );
    }

    #[tokio::test]
    async fn list_filter_cannot_escape_authenticated_namespace() {
        let (router, db) = two_tenant_router().await;
        let tenant_a_id = ns_id(&db, "tenant-a").await;

        let (status, _) = post_json(
            router.clone(),
            "/v1/memory/remember",
            "key-a",
            serde_json::json!({"content": "tenant-a confidential quarterly plan"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // tenant-b forges filter.namespace_id to list tenant-a's memories.
        let (status, out) = post_json(
            router.clone(),
            "/v1/memory/list",
            "key-b",
            serde_json::json!({
                "limit": 50,
                "filter": {"namespace_id": tenant_a_id},
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            out["total"], 0,
            "forged filter.namespace_id must not list tenant-a memories, got: {out}"
        );

        // tenant-a still sees its own memory.
        let (status, out) =
            post_json(router.clone(), "/v1/memory/list", "key-a", serde_json::json!({"limit": 50}))
                .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(out["total"], 1, "tenant-a should list its own memory, got: {out}");
    }
}
