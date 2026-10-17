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

## Multi-tenancy

The server isolates data per namespace (v0.4.0+). Pass a `namespace` option and
it is sent as the `x-namespace` header on every request:

```ts
const client = new NovaClient("http://127.0.0.1:7999", {
  apiKey: "optional-key",
  namespace: "team-a",
});

// Or derive a scoped copy from an existing client:
const scoped = client.withNamespace("team-b");
```

A namespace must start with a letter or digit and may then contain only
letters, digits, hyphens and underscores. Because the name travels in an HTTP
header, characters that could split or forge a header (CR/LF, colon, spaces,
non-ASCII, NUL) are rejected with an `Error`.

### Managing namespaces

An admin-scoped API key can also manage the namespaces themselves:

```ts
await client.createNamespace({ name: "team-a", description: "Team A", config: { tier: "gold" } });
await client.listNamespaces({ limit: 50, offset: 0 });
await client.getNamespace("team-a");

// Only the fields you pass are sent; the rest keep their stored values.
await client.updateNamespace("team-a", { description: "Team A (EU)" });

const result = await client.deleteNamespace("team-a");
console.log(result.deleted, result.protected);
```

A few behaviours worth knowing:

- `createNamespace` validates the name against the same rule as
  `withNamespace`, so any namespace you create can immediately be scoped to. The
  reserved name `default` is rejected case-insensitively.
- `updateNamespace` sends only the keys present in the input object. Omitted
  fields are left untouched, while `description: ""` clears the description.
  There is no rename operation.
- `deleteNamespace` never removes the built-in `default` namespace; the result
  reports `protected: true` in that case.
- Tenant-scoped keys may only read their own namespace. Creating, updating
  another tenant or deleting anything returns `403` from the server.

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

Every method has a concrete return type, so response fields are checked at
compile time rather than at runtime:

```ts
const out = await client.traverse({ start: uuid });
for (const node of out) {
  console.log(node.entity.name, node.depth, node.path.length);
}

const merged = await client.mergeEntities({ keep_uuid: "a", discard_uuids: ["b"] });
console.log(merged.merged.length, merged.remapped_relations);
```

## Errors

Every non-2xx response throws a `NovaApiError` carrying `code`, `message`,
`status` and an optional `traceId`. Network and timeout failures throw a
`NovaApiError` with status `0` and code `network` or `timeout`.

## Building from source

```bash
npm install
npm run build

# Type-check only, without emitting.
npm run typecheck

# Type-check the compile-time contract tests in test/types.contract.ts.
npm run typecheck:tests

# Build, contract tests and runtime tests.
npm test
```

## License

Business Source License 1.1.