# llama-index-yq-nova

A [LlamaIndex](https://www.llamaindex.ai) memory store and tool set for the
[yq-nova agent memory service](..). It gives LlamaIndex agents a persistent,
semantic memory backed by the yq-nova HTTP API.

## Features

- `YqNovaMemory` - a `BaseMemory` implementation that stores each `ChatMessage`
  in yq-nova and recalls the most relevant messages for the current input.
  Works with `AgentRunner`, `ChatMemoryBuffer`-style memory slots and
  `ReActAgent` / `FunctionAgent`.
- `create_memory_tools()` - ready-to-use `nova_remember`, `nova_recall` and
  `nova_forget` `FunctionTool`s to register on an agent.
- `create_graph_tools()` - knowledge-graph tools: `nova_list_entities`,
  `nova_traverse`, `nova_list_relations`, `nova_link_entities` and
  `nova_extract_graph`.
- `create_all_tools()` - the memory and graph tool sets combined.

## Install

```bash
pip install llama-index-yq-nova
```

Requires a running `yq-nova` server (default `http://127.0.0.1:7999`).

## Usage

### Memory store

```python
from yq_nova import Client
from llama_index.core.base.llms.types import ChatMessage, MessageRole
from llama_index_yq_nova import YqNovaMemory

client = Client("http://127.0.0.1:7999")
memory = YqNovaMemory(client, namespace="alice")

memory.put(ChatMessage(role=MessageRole.USER, content="I like Rust"))
history = memory.get("What does Alice like?")
print(history)

memory.reset()
```

`YqNovaMemory` plugs into a LlamaIndex agent via the standard memory protocol:

```python
from llama_index.core.agent import AgentRunner
from llama_index.core.agent.runner.base import AgentState

state = AgentState(chat_history=memory.get_all())
runner = AgentRunner(agent=..., chat_history=memory)
```

### Agent tools

```python
from llama_index_yq_nova import create_all_tools, create_graph_tools, create_memory_tools

# Memory only: store, search and forget.
tools = create_memory_tools(client)

# Memory plus the knowledge graph.
tools = create_all_tools(client)

# Or just the graph half.
tools = create_graph_tools(client)
```

Pass `tools` to a `FunctionAgent` or `ReActAgent` so the model can store, search
and forget memories at runtime.

### Knowledge-graph tools

Recall can traverse the entity-relation graph, but an agent often needs to
*inspect* it directly. `create_graph_tools()` adds five tools for that:

| Tool | Purpose |
|------|---------|
| `nova_list_entities` | Find entities by name prefix and/or type, to obtain the uuid needed to start a traversal. |
| `nova_traverse` | Walk the graph outward from an entity uuid. `max_depth` and `max_nodes` are capped by default so a traversal cannot run away. |
| `nova_list_relations` | List relations, optionally filtered by source uuid, target uuid and/or predicate. |
| `nova_link_entities` | Create or strengthen a typed relation between two existing entities. |
| `nova_extract_graph` | Extract entities and relations from free text via the server's configured LLM extractor. |

A typical agent flow is `nova_extract_graph` (or `nova_list_entities`) to find
uuids, then `nova_link_entities` to record a new fact, then `nova_traverse` to
answer a question that needs surrounding context.

`nova_link_entities` validates its arguments before issuing a request, so a
blank predicate or a confidence outside `[0.0, 1.0]` raises `ValueError`
without a round trip. Both entities must already exist; use
`nova_list_entities` first.

The package reuses the zero-dependency `yq_nova.Client`, so no extra HTTP
library is required. See `examples/` for a full walkthrough.

## License

Business Source License 1.1.