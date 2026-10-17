# yq-nova Python Client SDK

A minimal, **zero-dependency** HTTP client for the [yq-nova](..) Agent memory
service. It uses only the Python standard library (`urllib.request`), so no
`pip install` is required.

It talks to a **running `yq_nova` server** — start one first, then point this
client at its HTTP base URL (default `http://127.0.0.1:7999`).

## Installation

There is nothing to install — just put `yq_nova/` on your `PYTHONPATH`, or run
from this directory:

```bash
PYTHONPATH=. python your_script.py
```

Requires Python 3.8+.

## Usage

```python
from yq_nova import Client

client = Client("http://127.0.0.1:7999")
# Optionally authenticate:
# client = Client("http://127.0.0.1:7999", api_key="<your-key>")

# Health + stats
print(client.health())
print(client.stats())

# Remember a memory
result = client.remember(
    "yq-nova stores agent memory in SQLite",
    importance=0.8,
    tags=["nova", "storage"],
)
uuid = result["uuid"]
print("stored:", uuid)

# Recall relevant memories
for hit in client.recall("SQLite memory", top_k=5)["hits"]:
    print(hit["memory"]["content"], hit["final_score"])

# Forget (archive) the memory again
print(client.forget(uuid=uuid, mode="archive"))

# Fetch / hard-delete a single memory
print(client.get_memory(uuid))
client.delete_memory(uuid)
```

## Multi-tenant namespaces

Every request carries an `x-namespace` header. The default namespace is
`default`, so single-tenant deployments need no changes:

```python
from yq_nova import Client

# Bind the namespace when constructing the client
team_client = Client("http://127.0.0.1:7999", namespace="team-a")
team_client.remember("team-a memory")

# Or derive a scoped copy from an existing client; the original stays untouched
scoped = team_client.with_namespace("team-b")
print(scoped.namespace)  # team-b
print(team_client.namespace)  # team-a
```

Namespace names are validated before they reach the `x-namespace` header:
they must start with a letter or digit and may then contain only letters,
digits, hyphens and underscores. Empty, blank or otherwise unsafe names
(for example names containing spaces, colons or newlines) raise
`ValueError`.

### Managing namespaces

An admin-scoped API key can also create and manage namespaces. The client
exposes the full set of namespace routes:

```python
admin = Client("http://127.0.0.1:7999", api_key="<admin-key>")

admin.create_namespace("team-a", description="Team A", config={"tier": "gold"})
admin.list_namespaces(limit=50, offset=0)
admin.get_namespace("team-a")

# Only the fields you pass are changed; the rest keep their stored values.
admin.update_namespace("team-a", description="Team A (EU)")

admin.delete_namespace("team-a")
```

A few behaviours worth knowing:

- `create_namespace` rejects the reserved name `default` (case-insensitively)
  and enforces the same character rules as `with_namespace`, so any namespace
  you create can immediately be used with `with_namespace(name)`.
- `update_namespace` sends only the arguments you provide, so omitted fields
  are left untouched. There is no rename operation; create the target
  namespace and migrate instead.
- `delete_namespace` returns a dict with `deleted` and `protected` flags. The
  built-in `default` namespace is protected and is never removed.
- Tenant-scoped keys may only read their own namespace; creating, updating
  another tenant or deleting anything returns `403` from the server.

## Graph operations

```python
# Upsert an entity
client.upsert_entity("Alice", entity_type="person", description="Rust engineer")

# List entities with optional filters
entities = client.list_entities(entity_type="person", limit=50)

# Upsert a relation between two entities
client.upsert_relation(
    alice_uuid, bob_uuid, "reports_to", confidence=0.9, memory_uuid=memory_uuid
)

# List relations with optional source/target/predicate filters
relations = client.list_relations(source=alice_uuid, limit=50)

# BFS-traverse the graph from an entity uuid
client.traverse("<entity-uuid>", max_depth=2)

# Auto-extract entities and relations from text
client.extract_and_link(
    "I love [[Rust]] and [[Tokio]] async runtime",
    opts={"enabled": True, "upsert_entities": True, "create_relations": False},
)
```

The client covers every v1 endpoint: `health`, `stats`, `remember`,
`remember_batch`, `recall`, `list_memories`, `forget`, `get_memory`,
`update_memory`, `delete_memory`, `export_memories`, `import_memories`,
`merge_memories`, `list_tags`, `rename_tag`, `delete_tag`, `upsert_entity`,
`list_entities`, `merge_entities`, `upsert_relation`, `list_relations`,
`traverse` and `extract_and_link`. A runnable walkthrough of all of them is in
[`examples/full_api_demo.py`](examples/full_api_demo.py).

## Errors

Non-2xx responses raise `NovaApiError` with `code`, `message`, `status` and an
optional `trace_id`:

```python
from yq_nova import NovaApiError

try:
    client.get_memory("does-not-exist")
except NovaApiError as e:
    print(e.status, e.code, e.message)  # e.g. 404 not_found ...
```

## Running the tests

```bash
python3 -m py_compile yq_nova/client.py yq_nova/__init__.py   # syntax check
python3 -m pytest tests/ -q                                   # optional smoke test
```