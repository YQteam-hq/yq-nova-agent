use std::{path::PathBuf, sync::Arc};

use yq_nova_core::{
    Uuid,
    config::StorageConfig,
    embedding::{MockEmbeddingProvider, SharedEmbeddingProvider},
    error::ErrorCode,
    graph::{GraphExtractOpts, GraphService, extractor::RegexWikiExtractor},
    memory::MemoryService,
    storage::Database,
};
use yq_nova_sdk::{EmbeddedNova, http_client};

fn tmp_db(tag: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
    let mut p = std::env::temp_dir();
    p.push(format!("yqnova-embedded-{tag}-{}.db", n ^ 0x9e3779b97f4a7c15));
    p
}

#[tokio::test]
async fn remember_recall_forget_flow() {
    let nova = EmbeddedNova::open(tmp_db("rr")).await.expect("open embedded nova");

    let out = nova
        .remember(http_client::RememberRequest {
            content: "Rust memory: impl Deref for MyBox via Box-like layout".into(),
            importance: 0.9,
            tags: vec!["rust".into()],
            ..Default::default()
        })
        .await
        .expect("remember");
    assert!(!out.uuid.is_nil(), "new memory must get a non-nil uuid");

    let rec = nova.get_memory(out.uuid).await.expect("get_memory");
    assert_eq!(rec.content, "Rust memory: impl Deref for MyBox via Box-like layout");

    let recall = nova
        .recall(http_client::RecallRequest {
            query: "rust deref".into(),
            top_k: 5,
            ..Default::default()
        })
        .await
        .expect("recall");
    assert!(
        recall.hits.iter().any(|h| h.memory.uuid == out.uuid),
        "recall should return the remembered uuid"
    );

    nova.delete_memory(out.uuid).await.expect("delete_memory");
    let err = nova.get_memory(out.uuid).await.expect_err("get after delete should fail");
    assert_eq!(err.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn stats_reflect_counts() {
    let nova = EmbeddedNova::open(tmp_db("stats")).await.expect("open");

    let s0 = nova.stats().await.expect("stats before");
    assert_eq!(s0.memory_active, 0);
    assert_eq!(s0.memory_total, 0);

    let out = nova
        .remember(http_client::RememberRequest {
            content: "stats counting note".into(),
            tags: vec!["counted".into()],
            ..Default::default()
        })
        .await
        .expect("remember");

    let s1 = nova.stats().await.expect("stats after");
    assert_eq!(s1.memory_active, 1);
    assert_eq!(s1.memory_total, 1);
    assert!(s1.tag_count >= 1);
    let _ = out;
}

#[tokio::test]
async fn graph_extract_and_link_works() {
    let db_path = tmp_db("graph");
    let db = Database::open(StorageConfig {
        db_path,
        ..Default::default()
    })
    .await
    .expect("open db");
    let embedding: SharedEmbeddingProvider = Arc::new(MockEmbeddingProvider::new(64));
    let memory = MemoryService::new(db.clone(), embedding);
    let graph = GraphService::with_parts(db.clone(), Arc::new(RegexWikiExtractor::new()));
    let nova = EmbeddedNova::from_services(db, memory, graph);

    let out = nova
        .extract_and_link(http_client::ExtractAndLinkRequest {
            text: "[[Alice]] works with [[Bob]]".into(),
            opts: GraphExtractOpts {
                enabled: true,
                ..Default::default()
            },
        })
        .await
        .expect("extract_and_link");
    assert!(out.entities_upserted >= 2, "expected >= 2 entities, got {}", out.entities_upserted);
    assert!(out.entities.iter().any(|(e, _)| e.name == "Alice"));
    assert!(out.entities.iter().any(|(e, _)| e.name == "Bob"));
}

#[tokio::test]
async fn health_returns_ok() {
    let nova = EmbeddedNova::open(tmp_db("health")).await.expect("open");
    let h = nova.health().await.expect("health");
    assert_eq!(h.status, "ok");
    assert_eq!(h.version, yq_nova_core::VERSION);
    let _ = Uuid::new_v4();
}

/// Build an EmbeddedNova whose extractor recognises `[[Wiki]]` links, so a
/// graph test can create entities and relations without an LLM.
async fn graph_nova(tag: &str) -> EmbeddedNova {
    let db = Database::open(StorageConfig {
        db_path: tmp_db(tag),
        ..Default::default()
    })
    .await
    .expect("open db");
    let embedding: SharedEmbeddingProvider = Arc::new(MockEmbeddingProvider::new(64));
    let memory = MemoryService::new(db.clone(), embedding);
    let graph = GraphService::with_parts(db.clone(), Arc::new(RegexWikiExtractor::new()));
    EmbeddedNova::from_services(db, memory, graph)
}

#[tokio::test]
async fn embedded_list_entities_supports_prefix_and_type_filters() {
    let nova = graph_nova("list-entities").await;

    nova.upsert_entity(http_client::UpsertEntityRequest {
        name: "Alice".into(),
        entity_type: "person".into(),
        description: None,
        metadata: None,
    })
    .await
    .expect("upsert Alice");
    nova.upsert_entity(http_client::UpsertEntityRequest {
        name: "Acme Corp".into(),
        entity_type: "org".into(),
        description: None,
        metadata: None,
    })
    .await
    .expect("upsert Acme");

    let all = nova.list_entities(None, None, 10, 0).await.expect("list all");
    assert_eq!(all.len(), 2);

    // A prefix filter must match literally, not as a LIKE wildcard.
    let by_prefix = nova.list_entities(Some("Al"), None, 10, 0).await.expect("list by prefix");
    assert_eq!(by_prefix.len(), 1);
    assert_eq!(by_prefix[0].name, "Alice");

    let by_type = nova.list_entities(None, Some("person"), 10, 0).await.expect("list by type");
    assert_eq!(by_type.len(), 1);
    assert_eq!(by_type[0].r#type, "person");

    // An empty filter string means "no filter", matching the HTTP client.
    let empty_prefix =
        nova.list_entities(Some(""), Some(""), 10, 0).await.expect("list with empty filters");
    assert_eq!(empty_prefix.len(), 2);
}

#[tokio::test]
async fn embedded_list_entities_paginates() {
    let nova = graph_nova("list-entities-page").await;
    for name in ["A", "B", "C"] {
        nova.upsert_entity(http_client::UpsertEntityRequest {
            name: name.into(),
            entity_type: "thing".into(),
            description: None,
            metadata: None,
        })
        .await
        .expect("upsert");
    }

    let first = nova.list_entities(None, None, 2, 0).await.expect("page 0");
    assert_eq!(first.len(), 2);
    let second = nova.list_entities(None, None, 2, 2).await.expect("page 1");
    assert_eq!(second.len(), 1);

    let first_ids: Vec<i64> = first.iter().map(|e| e.id).collect();
    let second_ids: Vec<i64> = second.iter().map(|e| e.id).collect();
    assert!(
        first_ids.iter().all(|id| !second_ids.contains(id)),
        "pages must not overlap: {first_ids:?} vs {second_ids:?}"
    );
}

#[tokio::test]
async fn embedded_upsert_relation_rejects_incomplete_input() {
    let nova = graph_nova("upsert-bad").await;
    let alice = nova
        .upsert_entity(http_client::UpsertEntityRequest {
            name: "Alice".into(),
            entity_type: "person".into(),
            description: None,
            metadata: None,
        })
        .await
        .expect("upsert Alice");

    let mk = |source: Uuid, target: Uuid, predicate: &str| http_client::UpsertRelationRequest {
        source_uuid: source,
        target_uuid: target,
        predicate: predicate.into(),
        confidence: 0.9,
        memory_uuid: None,
        metadata: None,
        idempotent: true,
    };

    // A nil endpoint is rejected before touching the database.
    let err = nova
        .upsert_relation(mk(Uuid::nil(), alice.entity.uuid, "knows"))
        .await
        .expect_err("nil source must be rejected");
    assert_eq!(err.code(), ErrorCode::Validation, "got {err:?}");

    let err = nova
        .upsert_relation(mk(alice.entity.uuid, Uuid::nil(), "knows"))
        .await
        .expect_err("nil target must be rejected");
    assert_eq!(err.code(), ErrorCode::Validation, "got {err:?}");

    // An empty predicate is rejected too.
    let err = nova
        .upsert_relation(mk(alice.entity.uuid, alice.entity.uuid, "   "))
        .await
        .expect_err("blank predicate must be rejected");
    assert_eq!(err.code(), ErrorCode::Validation, "got {err:?}");
}

#[tokio::test]
async fn embedded_upsert_relation_then_list_and_filter() {
    let nova = graph_nova("upsert-ok").await;

    let alice = nova
        .upsert_entity(http_client::UpsertEntityRequest {
            name: "Alice".into(),
            entity_type: "person".into(),
            description: None,
            metadata: None,
        })
        .await
        .expect("upsert Alice")
        .entity;
    let bob = nova
        .upsert_entity(http_client::UpsertEntityRequest {
            name: "Bob".into(),
            entity_type: "person".into(),
            description: None,
            metadata: None,
        })
        .await
        .expect("upsert Bob")
        .entity;

    let inserted = nova
        .upsert_relation(http_client::UpsertRelationRequest {
            source_uuid: alice.uuid,
            target_uuid: bob.uuid,
            predicate: "knows".into(),
            confidence: 0.9,
            memory_uuid: None,
            metadata: None,
            idempotent: true,
        })
        .await
        .expect("upsert relation");
    assert!(inserted.inserted, "the first write must report inserted");
    assert!(!inserted.updated);
    assert_eq!(inserted.relation.source_uuid, alice.uuid);
    assert_eq!(inserted.relation.target_uuid, bob.uuid);
    assert_eq!(inserted.relation.predicate, "knows");

    // A weaker idempotent re-insert keeps the stored confidence, so the
    // repository reports Unchanged: neither inserted nor updated.
    let weaker = nova
        .upsert_relation(http_client::UpsertRelationRequest {
            source_uuid: alice.uuid,
            target_uuid: bob.uuid,
            predicate: "knows".into(),
            confidence: 0.5,
            memory_uuid: None,
            metadata: None,
            idempotent: true,
        })
        .await
        .expect("upsert relation again");
    assert!(!weaker.inserted, "an unchanged edge is not a fresh insert");
    assert!(!weaker.updated, "a weaker edge must not overwrite the stored confidence");
    assert_eq!(
        weaker.relation_uuid, inserted.relation_uuid,
        "idempotent upsert must reuse the same relation"
    );
    assert!(
        weaker.relation.confidence >= 0.9,
        "the stored confidence must not regress, got {}",
        weaker.relation.confidence
    );

    // A stronger idempotent re-insert does raise the stored confidence.
    let stronger = nova
        .upsert_relation(http_client::UpsertRelationRequest {
            source_uuid: alice.uuid,
            target_uuid: bob.uuid,
            predicate: "knows".into(),
            confidence: 0.99,
            memory_uuid: None,
            metadata: None,
            idempotent: true,
        })
        .await
        .expect("upsert relation stronger");
    assert!(stronger.updated, "a stronger edge must report updated");
    assert!(!stronger.inserted);
    assert_eq!(stronger.relation_uuid, inserted.relation_uuid);

    let outgoing =
        nova.list_relations(Some(alice.uuid), None, None, 10, 0).await.expect("list outgoing");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].target_uuid, bob.uuid);

    let incoming =
        nova.list_relations(None, Some(bob.uuid), None, 10, 0).await.expect("list incoming");
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].source_uuid, alice.uuid);

    // Both endpoints together narrow the outgoing set to the matching edge.
    let exact = nova
        .list_relations(Some(alice.uuid), Some(bob.uuid), None, 10, 0)
        .await
        .expect("list exact");
    assert_eq!(exact.len(), 1);

    // A predicate that does not match yields nothing rather than everything.
    let other_predicate = nova
        .list_relations(Some(alice.uuid), None, Some("owes"), 10, 0)
        .await
        .expect("list by predicate");
    assert!(other_predicate.is_empty());

    // With neither endpoint given, all relations are reachable.
    let all = nova.list_relations(None, None, None, 10, 0).await.expect("list all");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].predicate, "knows");
}

#[tokio::test]
async fn embedded_upsert_relation_rejects_self_loops() {
    let nova = graph_nova("self-loop").await;
    let alice = nova
        .upsert_entity(http_client::UpsertEntityRequest {
            name: "Alice".into(),
            entity_type: "person".into(),
            description: None,
            metadata: None,
        })
        .await
        .expect("upsert Alice")
        .entity;

    // The storage layer refuses a self-loop outright, which is what keeps the
    // unfiltered listing free of an edge that would otherwise be reachable
    // from both of its own endpoints.
    let err = nova
        .upsert_relation(http_client::UpsertRelationRequest {
            source_uuid: alice.uuid,
            target_uuid: alice.uuid,
            predicate: "follows".into(),
            confidence: 1.0,
            memory_uuid: None,
            metadata: None,
            idempotent: true,
        })
        .await
        .expect_err("a self-loop must be rejected");
    assert_eq!(err.code(), ErrorCode::Validation, "got {err:?}");

    assert!(
        nova.list_relations(None, None, None, 10, 0).await.expect("list all").is_empty(),
        "the rejected self-loop must not be stored"
    );
}

#[tokio::test]
async fn embedded_list_relations_requires_existing_endpoints() {
    let nova = graph_nova("missing-endpoint").await;
    let alice = nova
        .upsert_entity(http_client::UpsertEntityRequest {
            name: "Alice".into(),
            entity_type: "person".into(),
            description: None,
            metadata: None,
        })
        .await
        .expect("upsert Alice")
        .entity;

    // Inserting an edge whose target was never upserted is a validation error,
    // not an orphaned row.
    let err = nova
        .upsert_relation(http_client::UpsertRelationRequest {
            source_uuid: alice.uuid,
            target_uuid: Uuid::new_v4(),
            predicate: "knows".into(),
            confidence: 0.5,
            memory_uuid: None,
            metadata: None,
            idempotent: true,
        })
        .await
        .expect_err("an unknown target must be rejected");
    assert_eq!(err.code(), ErrorCode::Validation, "got {err:?}");
}
