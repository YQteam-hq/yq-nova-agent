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
`traverse`, `extract_and_link` and the five namespace admin methods
(`list_namespaces`, `get_namespace`, `create_namespace`, `update_namespace`,
`delete_namespace`). A runnable walkthrough of all of them is in
[`examples/full_api_demo.py`](examples/full_api_demo.py).

## Multi-tenant namespaces

By default the client talks to the `default` namespace. Pass `namespace=` to
scope every request to one tenant — it is sent as the `x-namespace` header,
which the server uses to route reads and writes:

```python
client = Client("http://127.0.0.1:7999", namespace="team-acme")

# Retarget the same client at another tenant and keep using it
client.with_namespace("team-beta")
print(client.namespace)  # 'team-beta'
```

Omitting the namespace leaves the header out entirely, which makes the server
fall back to `default`. A blank namespace is rejected client-side, matching
the server. Namespace admin (server-side management, not tenant-scoped):

```python
client.create_namespace("team-acme", description="Acme Corp")
print(client.list_namespaces(limit=50))
print(client.get_namespace("team-acme"))
client.update_namespace("team-acme", description="Acme Corporation")
client.delete_namespace("team-acme")  # 'default' itself is reserved
```

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