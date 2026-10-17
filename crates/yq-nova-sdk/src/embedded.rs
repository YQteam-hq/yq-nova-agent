use std::{path::PathBuf, sync::Arc};

use yq_nova_core::{
    Uuid,
    config::StorageConfig,
    embedding::{MockEmbeddingProvider, SharedEmbeddingProvider},
    error::{NovaError, NovaResult},
    graph::{GraphService, LinkResult, MergeEntitiesInput, TraverseNode, TraverseOpts},
    memory::{
        BatchRememberInput, BatchRememberOutput, ConflictStrategy, ExportInput, ForgetInput,
        ForgetOutput, ImportInput, ImportItem, ListInput, ListOutput, MemoryService, RecallOutput,
        RememberOutput, TagDeleteInput, TagDeleteOutput, TagListInput, TagListOutput,
        TagRenameInput, TagRenameOutput, UpdateInput, ops_forget, ops_recall, ops_remember,
    },
    storage::{
        Database, MemoryRecord, RelationRecord,
        entity::{EntityRecord, EntityRepository},
        relation::RelationRepository,
    },
};

use crate::http_client;

/// Upper bound applied to a single page of graph query results, mirroring the
/// `.min(500)` clamp the HTTP handlers perform on `limit`.
const MAX_GRAPH_PAGE: usize = 500;

#[derive(Clone)]
pub struct EmbeddedNova {
    pub database: Database,

    pub memory: MemoryService,

    pub graph: GraphService,
}

impl EmbeddedNova {
    pub async fn open(db_path: impl Into<PathBuf>) -> NovaResult<Self> {
        let database = Database::open(StorageConfig {
            db_path: db_path.into(),
            ..Default::default()
        })
        .await?;
        let embedding: SharedEmbeddingProvider = Arc::new(MockEmbeddingProvider::new(64));
        let memory = MemoryService::new(database.clone(), embedding);
        let graph = GraphService::new(database.clone());
        Ok(Self {
            database,
            memory,
            graph,
        })
    }

    pub fn from_services(database: Database, memory: MemoryService, graph: GraphService) -> Self {
        Self {
            database,
            memory,
            graph,
        }
    }

    pub async fn remember(&self, req: http_client::RememberRequest) -> NovaResult<RememberOutput> {
        let input = ops_remember::RememberInput {
            namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
            content: &req.content,
            source: req.source,
            importance: req.importance,
            metadata: req.metadata.as_ref(),
            expires_at: req.expires_at,
            tags: req.tags.as_ref(),
            embed: req.embed,
            extract_graph: req.extract_graph,
            chunk_options: req.chunk_options,
            dedup: None,
        };
        self.memory.remember(input).await
    }

    pub async fn recall(&self, req: http_client::RecallRequest) -> NovaResult<RecallOutput> {
        let input = ops_recall::RecallInput {
            query: &req.query,
            top_k: req.top_k,
            score_threshold: req.score_threshold,
            similarity_threshold: req.similarity_threshold,
            mode: req.mode,
            graph: req.graph,
            hybrid_weights: req.hybrid_weights,
            rrf_k: req.rrf_k,
            rank_weights: req.rank_weights,
            filter: req.filter,
            group_chunks: req.group_chunks,
            entity_focus: req.entity_focus,
            rebalance_importance: false,
        };
        self.memory.recall(input).await
    }

    pub async fn forget(&self, input: ForgetInput) -> NovaResult<ForgetOutput> {
        self.memory.forget(input).await
    }

    pub async fn get_memory(&self, uuid: Uuid) -> NovaResult<yq_nova_core::storage::MemoryRecord> {
        self.memory.get_memory(uuid).await
    }

    pub async fn delete_memory(&self, uuid: Uuid) -> NovaResult<ForgetOutput> {
        self.memory
            .forget(ops_forget::ForgetInput {
                namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
                target: ops_forget::ForgetTarget::One(uuid),
                mode: ops_forget::ForgetMode::Hard,
                gc_graph: false,
                batch_limit: 1,
            })
            .await
    }

    pub async fn update_memory(
        &self,
        uuid: Uuid,
        req: http_client::UpdateMemoryRequest,
    ) -> NovaResult<MemoryRecord> {
        let input = UpdateInput {
            namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
            content: req.content,
            importance: req.importance,
            metadata: req.metadata,
            tags: req.tags,
            expires_at: req.expires_at,
        };
        self.memory.update(uuid, input).await
    }

    pub async fn merge_memories(
        &self,
        req: http_client::MergeMemoriesRequest,
    ) -> NovaResult<http_client::MergeMemoriesResponse> {
        let input = yq_nova_core::memory::ops_merge::MergeInput {
            namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
            uuids: req.uuids,
            keep_uuid: req.keep_uuid,
        };
        let out = self.memory.merge(input).await?;
        Ok(http_client::MergeMemoriesResponse {
            kept_uuid: out.kept_uuid,
            merged: out.merged,
            remapped_relations: out.remapped_relations,
        })
    }

    pub async fn export_memories(
        &self,
        req: http_client::ExportMemoriesRequest,
    ) -> NovaResult<http_client::ExportMemoriesResponse> {
        let input = ExportInput {
            filter: req.filter,
            limit: req.limit,
            offset: req.offset,
        };
        let out = yq_nova_core::memory::ops_export::export_memories(&self.memory, input).await?;
        Ok(http_client::ExportMemoriesResponse {
            count: out.count,
            offset: out.offset,
            items: out.items,
        })
    }

    pub async fn import_memories(
        &self,
        req: http_client::ImportMemoriesRequest,
    ) -> NovaResult<http_client::ImportMemoriesResponse> {
        let items: Vec<ImportItem> = req
            .items
            .into_iter()
            .map(|i| ImportItem {
                content: i.content,
                uuid: i.uuid,
                metadata: i.metadata,
                importance: i.importance,
                source: i.source,
                tags: i.tags,
                expires_at: i.expires_at,
            })
            .collect();
        let on_conflict = ConflictStrategy::Skip;
        let input = ImportInput {
            namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
            items,
            embed: req.embed,
            on_conflict,
        };
        let out = yq_nova_core::memory::ops_import::import_memories(&self.memory, input).await?;
        Ok(http_client::ImportMemoriesResponse {
            received: out.received,
            imported: out.imported,
            duplicates: out.duplicates,
            errors: out
                .errors
                .into_iter()
                .map(|e| http_client::ImportError {
                    index: e.index,
                    message: e.message,
                })
                .collect(),
        })
    }

    pub async fn list_memories(&self, req: ListInput) -> NovaResult<ListOutput> {
        self.memory.list_memories(req).await
    }

    pub async fn remember_batch(&self, req: BatchRememberInput) -> NovaResult<BatchRememberOutput> {
        self.memory.remember_batch(req).await
    }

    pub async fn list_tags(&self, limit: u32, offset: u32) -> NovaResult<TagListOutput> {
        let input = TagListInput {
            namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
            limit,
            offset,
        };
        self.memory.list_tags(input).await
    }

    pub async fn rename_tag(&self, name: &str, new_name: &str) -> NovaResult<TagRenameOutput> {
        let input = TagRenameInput {
            namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
            name: name.to_string(),
            new_name: new_name.to_string(),
        };
        self.memory.rename_tag(input).await
    }

    pub async fn delete_tag(&self, name: &str) -> NovaResult<TagDeleteOutput> {
        let input = TagDeleteInput {
            namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
            name: name.to_string(),
        };
        self.memory.delete_tag(input).await
    }

    pub async fn upsert_entity(
        &self,
        req: http_client::UpsertEntityRequest,
    ) -> NovaResult<http_client::UpsertEntityResponse> {
        let outcome = self
            .graph
            .entity_repo
            .upsert(
                &self.graph.database,
                yq_nova_core::storage::entity::UpsertEntityInput {
                    namespace_id: yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
                    name: &req.name,
                    r#type: &req.entity_type,
                    description: req.description.as_deref(),
                    metadata: req.metadata.as_ref(),
                },
            )
            .await?;
        let uuid = outcome.uuid();
        let entity = self
            .graph
            .entity_repo
            .get_by_uuid(
                &self.graph.database,
                yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
                uuid,
            )
            .await?;
        Ok(http_client::UpsertEntityResponse {
            outcome,
            entity,
        })
    }

    pub async fn list_entities(
        &self,
        name_prefix: Option<&str>,
        entity_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> NovaResult<Vec<EntityRecord>> {
        // Empty filter strings are treated as "no filter" so that a caller
        // passing `Some("")` gets the unfiltered page, matching the HTTP
        // client's behaviour.
        let name_prefix = name_prefix.filter(|s| !s.is_empty());
        let entity_type = entity_type.filter(|s| !s.is_empty());
        self.graph
            .entity_repo
            .list(
                &self.graph.database,
                yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
                name_prefix,
                entity_type,
                limit.min(MAX_GRAPH_PAGE),
                offset,
            )
            .await
    }

    pub async fn upsert_relation(
        &self,
        req: http_client::UpsertRelationRequest,
    ) -> NovaResult<http_client::UpsertRelationResponse> {
        if req.source_uuid.is_nil() || req.target_uuid.is_nil() {
            return Err(NovaError::validation("upsert_relation: source_uuid/target_uuid required"));
        }
        let predicate = req.predicate.trim();
        if predicate.is_empty() {
            return Err(NovaError::validation("upsert_relation: predicate required"));
        }
        let db = &self.graph.database;
        let namespace_id = yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID;
        let outcome = self
            .graph
            .relation_repo
            .insert(
                db,
                yq_nova_core::storage::relation::InsertRelationInput {
                    namespace_id,
                    source_uuid: req.source_uuid,
                    target_uuid: req.target_uuid,
                    predicate,
                    confidence: req.confidence,
                    memory_uuid: req.memory_uuid,
                    metadata: req.metadata.as_ref(),
                    idempotent: req.idempotent,
                },
            )
            .await?;
        let relation_uuid = outcome.uuid();
        let relation =
            self.graph.relation_repo.get_by_uuid(db, namespace_id, relation_uuid).await?;
        Ok(http_client::UpsertRelationResponse {
            inserted: matches!(
                outcome,
                yq_nova_core::storage::relation::InsertRelationOutcome::Inserted(_)
            ),
            updated: matches!(
                outcome,
                yq_nova_core::storage::relation::InsertRelationOutcome::Updated(_)
            ),
            relation_uuid,
            relation,
        })
    }

    pub async fn list_relations(
        &self,
        source: Option<Uuid>,
        target: Option<Uuid>,
        predicate: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> NovaResult<Vec<RelationRecord>> {
        let db = &self.graph.database;
        let ns = yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID;
        // Fetch limit+offset rows before slicing so that offset behaves the
        // same way it does over HTTP pagination.
        let total_limit = limit.min(MAX_GRAPH_PAGE).saturating_add(offset);
        let predicate = predicate.filter(|s| !s.is_empty());

        let mut rows: Vec<RelationRecord> = Vec::new();
        if let Some(src) = source {
            let got =
                self.graph.relation_repo.list_outgoing(db, ns, src, predicate, total_limit).await?;
            match target {
                // Both endpoints given narrows the outgoing set further.
                Some(tgt) => rows.extend(got.into_iter().filter(|r| r.target_uuid == tgt)),
                None => rows = got,
            }
        } else if let Some(tgt) = target {
            rows =
                self.graph.relation_repo.list_incoming(db, ns, tgt, predicate, total_limit).await?;
        } else {
            // Neither endpoint given: walk the entity table and collect each
            // entity's outgoing edges, newest first, then de-duplicate because
            // a self-loop would otherwise be seen twice.
            let ents = self.graph.entity_repo.list(db, ns, None, None, MAX_GRAPH_PAGE, 0).await?;
            for e in ents {
                if rows.len() >= total_limit {
                    break;
                }
                let mut got = self
                    .graph
                    .relation_repo
                    .list_outgoing(db, ns, e.uuid, predicate, total_limit)
                    .await?;
                rows.append(&mut got);
            }
            rows.sort_by_key(|r| std::cmp::Reverse(r.id));
            rows.dedup_by_key(|r| r.id);
        }

        Ok(rows.into_iter().skip(offset).take(limit).collect())
    }

    pub async fn traverse(
        &self,
        req: http_client::TraverseRequest,
    ) -> NovaResult<Vec<TraverseNode>> {
        let opts = TraverseOpts {
            max_depth: req.max_depth,
            max_nodes: req.max_nodes,
            predicate_whitelist: req.predicate_whitelist,
            min_confidence: req.min_confidence,
        };
        self.graph
            .traverse_graph(yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID, req.start, opts)
            .await
    }

    pub async fn extract_and_link(
        &self,
        req: http_client::ExtractAndLinkRequest,
    ) -> NovaResult<LinkResult> {
        self.graph
            .extract_and_link(
                yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID,
                &req.text,
                &req.opts,
            )
            .await
    }

    pub async fn merge_entities(
        &self,
        req: http_client::MergeEntitiesRequest,
    ) -> NovaResult<http_client::MergeEntitiesResponse> {
        let input = MergeEntitiesInput {
            keep_uuid: req.keep_uuid,
            discard_uuids: req.discard_uuids,
        };
        let out = self
            .graph
            .merge_entities(yq_nova_core::storage::namespace::DEFAULT_NAMESPACE_ID, input)
            .await?;
        Ok(http_client::MergeEntitiesResponse {
            kept_uuid: out.kept_uuid,
            merged: out.merged,
            remapped_relations: out.remapped_relations,
        })
    }

    pub async fn health(&self) -> NovaResult<http_client::HealthResponse> {
        Ok(http_client::HealthResponse {
            status: "ok".into(),
            version: yq_nova_core::VERSION.into(),
            git_sha: yq_nova_core::git_sha().into(),
            uptime_secs: 0,
        })
    }

    pub async fn stats(&self) -> NovaResult<http_client::StatsResponse> {
        Ok(http_client::StatsResponse {
            uptime_secs: 0,
            database_size_bytes: self.database.size_on_disk_bytes().unwrap_or(0),
            memory_active: self
                .count("SELECT COUNT(*) FROM memory_items WHERE status = 'active'")
                .await?,
            memory_archived: self
                .count("SELECT COUNT(*) FROM memory_items WHERE status = 'archived'")
                .await?,
            memory_total: self.count("SELECT COUNT(*) FROM memory_items").await?,
            entity_count: self.count("SELECT COUNT(*) FROM entities").await?,
            relation_count: self.count("SELECT COUNT(*) FROM relations").await?,
            tag_count: self.count("SELECT COUNT(*) FROM tags").await?,
        })
    }

    async fn count(&self, sql: &str) -> NovaResult<u64> {
        let n: i64 = sqlx::query_scalar(sql)
            .fetch_one(&self.database.pool)
            .await
            .map_err(NovaError::storage)?;
        Ok(n.max(0) as u64)
    }
}
