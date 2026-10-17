# Changelog

All notable changes to **yq-nova-agent** are documented in this file. The project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- JS SDK: concrete return types for the remaining untyped endpoints — `ExportOutput`, `TagRenameOutput`, `TagDeleteOutput`, `TraverseNode`, `ExtractLinkOutput` and `EntityCandidate` — so no method returns `Record<string, unknown>` any more.
- JS SDK: `npm run typecheck:tests`, a compile-time contract test (`test/types.contract.ts`) that fails the build if a declared response type drifts from the server's wire format. Wired into `npm test`.
- LangChain and LlamaIndex integrations: new knowledge-graph tool sets (`create_graph_tools`, plus `create_all_tools` for the memory and graph tools combined). The five new tools are `nova_list_entities`, `nova_traverse`, `nova_list_relations`, `nova_link_entities` and `nova_extract_graph`, letting an agent inspect and extend the entity-relation graph that recall already traverses. Traversal depth and node count are capped by default, and arguments are validated before any request is issued.
- Rust SDK (`EmbeddedNova`): `list_entities`, `upsert_relation` and `list_relations`, closing the gap with `HttpClient` so in-process callers can query and write the entity graph without an HTTP round trip. Filters, pagination and the `MAX_GRAPH_PAGE` limit clamp behave the same as the equivalent HTTP routes.
- Python SDK: namespace management methods `list_namespaces`, `get_namespace`, `create_namespace`, `update_namespace` and `delete_namespace`, mirroring the existing `/v1/namespaces` routes. `create_namespace` rejects the reserved `default` name and enforces the same character rules as `with_namespace`; `update_namespace` sends only the fields you pass so the server leaves the rest untouched.
- JS SDK: the same five namespace management methods (`listNamespaces`, `getNamespace`, `createNamespace`, `updateNamespace`, `deleteNamespace`) together with the `NamespaceRecord`, `NamespaceListOutput`, `ListNamespacesParams`, `CreateNamespaceInput`, `UpdateNamespaceInput` and `DeleteNamespaceOutput` types.

### Fixed
- JS SDK: `MergeEntitiesOutput` declared a `discard_uuids` field that the server never returns. The response is `{ kept_uuid, merged, remapped_relations }`, so `out.merged` and `out.remapped_relations` did not type-check and `out.discard_uuids` typed as `string[]` while being `undefined` at runtime.
- `RetryConfig` with `max_attempts = 0` no longer panics: the attempt budget is clamped to one, so a misconfigured deployment retries once and reports an embedding error instead of aborting the request task. Callers relied on an error being returned here, never on the panic, so this is a bug fix rather than a behavior contract change.
- A `RetryConfig` whose `base_sleep` exceeds `max_sleep` no longer panics on the first retry; the sleep window is clamped so the sampling range stays valid.
- Retry backoff now uses saturating duration multiplication, so extreme `base_sleep` values cannot overflow.

## [0.4.0] - 2026-09-14

### Added

#### Multi-tenant namespaces (#12)
- Every memory, tag, entity and relation is now scoped to a namespace.
- New admin API `POST|GET /v1/namespaces` to create and list namespaces; a default `default` namespace is always present so single-tenant deployments are unaffected.
- Requests scope data through the `x-namespace` header; per-namespace API keys can be bound in `[server].namespace_keys` for tenant-scoped isolation.

#### Additional embedding providers (#9)
- New first-class embedding providers: **Zhipu** (`embedding.zhipu`), **Qwen / DashScope** (`embedding.qwen`), **Baichuan** (`embedding.baichuan`) and **Jina** (`embedding.jina`), alongside the existing OpenAI-compatible and mock providers.
- Each provider supports configurable model, dimensions, batch size, retry and request timeout.

#### Ecosystem SDKs & framework integrations (#10)
- New TypeScript SDK (`js/`) for Node.js and browsers.
- New Python client additions (`python/`), plus first-party **LangChain** (`langchain/langchain-yq-nova`) and **LlamaIndex** (`llamaindex/llama-index-yq-nova`) memory providers and tools.

#### Memory quality toolkit (#11)
- New `memory::quality` module exposed through the SDK / core service:
  - Deduplication (`dedup::detect`) to find near-duplicate memories.
  - Summarization (`summary::summarize_group`) to condense groups of related memories.
  - Importance rebalancing (`importance::rebalance`).
  - Association discovery (`associate::discover`).
- Recall now supports `rebalance_importance`.

#### LLM entity-relation auto extraction (#8)
- New `POST /v1/graph/extract-and-link` endpoint that uses a chat provider (`graph.extract_llm`) to extract entities and relations from text and link them into the graph.

#### Memory operations & tags (#6)
- New `POST /v1/memory/list` with filters, pagination and multiple sort orders.
- New `POST /v1/memory/remember-batch` for batch ingest (up to 200 items per round trip).
- Full tag management: `GET|PATCH|DELETE /v1/tags` with counting, rename and delete.

### Changed
- Workspace and crate versions bumped to `0.4.0` (`yq-nova-core`, `yq-nova-sdk`, `yq-nova-server`).
- Ecosystem SDK package versions bumped to `0.4.0` (`js`, `python`, `langchain`, `llamaindex`).
- Documentation unified to English and the README repaired (#5); internal clippy warnings cleared (#4).

## [0.3.0] - 2026-09-07

- Memory operations: PATCH update, merge, export/import, chunking, group chunks in recall.
- Graph memory: entity_focus recall, enforced metadata_match and traverse predicates, entity merge, hard-forget relation cleanup.
- New `yq-nova-mcp` crate exposing an MCP server; Agent integration docs and Python OpenAI-tools example.

## [0.2.0]

- Complete optimization suite; BSL-1.0 license change.

## [0.1.0]

- Initial beta release of yq-nova-agent.