# yq-nova JS/TS SDK

TypeScript client for the [yq-nova agent memory service](..). It covers every
HTTP endpoint (20+ APIs) with typed methods, and runs in both **Node.js (18+)**
and **browsers** without any runtime dependency.

## Install

```bash
npm install yq-nova
```

## Usage

```ts
import { NovaClient } from "yq-nova";

const client = new NovaClient("http://127.0.0.1:7999", {
  apiKey: "optional-key",
  timeoutMs: 30000,
});

await client.remember({
  content: "yq-nova stores agent memory in SQLite",
  importance: 0.8,
  tags: ["nova", "storage"],
});

const { hits } = await client.recall({ query: "SQLite memory", top_k: 5 });
const results = hits.map((hit) => ({
  content: hit.memory.content,
  score: hit.final_score,
}));

const page = await client.listMemories({
  filter: { tags_all: ["nova"] },
  sort: "importance_desc",
  limit: 20,
});
const count = page.items.length;
```

In a browser the global `fetch` is used automatically. You can inject a custom
fetch (for example one that adds auth headers):

```ts
const client = new NovaClient("http://127.0.0.1:7999", {
  fetch: myCoolFetch,
});
```

## API coverage

| Method | HTTP endpoint |
|--------|---------------|
| `health()` | `GET /v1/health` |
| `stats()` | `GET /v1/stats` |
| `remember()` | `POST /v1/memory/remember` |
| `rememberBatch()` | `POST /v1/memory/remember-batch` |
| `recall()` | `POST /v1/memory/recall` |
| `listMemories()` | `POST /v1/memory/list` |
| `forget()` | `POST /v1/memory/forget` |
| `getMemory()` | `GET /v1/memory/:uuid` |
| `updateMemory()` | `PATCH /v1/memory/:uuid` |
| `deleteMemory()` | `DELETE /v1/memory/:uuid` |
| `exportMemories()` | `POST /v1/memory/export` |
| `importMemories()` | `POST /v1/memory/import` |
| `mergeMemories()` | `POST /v1/memory/merge` |
| `listTags()` | `GET /v1/tags` |
| `renameTag()` | `PATCH /v1/tags/:name` |
| `deleteTag()` | `DELETE /v1/tags/:name` |
| `extractAndLink()` | `POST /v1/graph/extract-and-link` |
| `upsertEntity()` | `POST /v1/graph/entities` |
| `listEntities()` | `GET /v1/graph/entities` |
| `mergeEntities()` | `POST /v1/graph/entities/merge` |
| `upsertRelation()` | `POST /v1/graph/relations` |
| `listRelations()` | `GET /v1/graph/relations` |
| `traverse()` | `POST /v1/graph/traverse` |
| `listNamespaces()` | `GET /v1/namespaces` |
| `getNamespace()` | `GET /v1/namespaces/:name` |
| `createNamespace()` | `POST /v1/namespaces` |
| `updateNamespace()` | `PATCH /v1/namespaces/:name` |
| `deleteNamespace()` | `DELETE /v1/namespaces/:name` |

## Multi-tenant namespaces

Since 0.4.0 every memory, tag, entity and relation is scoped to a namespace, and
the server falls back to the `default` namespace when no `x-namespace` header is
sent. Pass one to target a tenant:

```ts
const client = new NovaClient("http://127.0.0.1:7999", {
  apiKey: "<tenant-key>",
  namespace: "team-a",
});

// Every request from this client now carries `x-namespace: team-a`.
await client.remember({ content: "tenant scoped memory" });

// Point the same client at another tenant.
client.withNamespace("team-b");
client.namespace; // "team-b"
```

Namespace administration needs an admin client (the global `auth_token`):

```ts
await client.listNamespaces({ limit: 100 });
await client.getNamespace("team-a");
await client.createNamespace({ name: "team-c", description: "third tenant" });
await client.updateNamespace("team-c", { description: "renamed" });
await client.deleteNamespace("team-c");
```

A namespace must be non-empty when provided, and `default` is reserved.

## Errors

Every non-2xx response throws a `NovaApiError` carrying `code`, `message`,
`status` and an optional `traceId`. Network and timeout failures throw a
`NovaApiError` with status `0` and code `network` or `timeout`.

## Building from source

```bash
npm install
npm run build
```

## License

Business Source License 1.1.