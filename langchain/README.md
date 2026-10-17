# langchain-yq-nova

A [LangChain](https://python.langchain.com) memory provider and tool set for the
[yq-nova agent memory service](..). It lets LangChain agents persist, retrieve
and forget memories through the yq-nova HTTP API.

## Features

- `YqNovaChatMessageHistory` - a `BaseChatMessageHistory` backed by yq-nova,
  so `ConversationBufferMemory` and similar memories work unchanged.
- `YqNovaMemory` - a `BaseMemory` that recalls relevant history for the current
  input and saves each exchange.
- `create_memory_tools()` - ready-to-use `nova_remember`, `nova_recall` and
  `nova_forget` tools to register on an `AgentExecutor` or
  `create_openai_tools_agent`.
- `create_graph_tools()` - knowledge-graph tools: `nova_list_entities`,
  `nova_traverse`, `nova_list_relations`, `nova_link_entities` and
  `nova_extract_graph`.
- `create_all_tools()` - the memory and graph tool sets combined.

## Install

```bash
pip install langchain-yq-nova
```

Requires a running `yq-nova` server (default `http://127.0.0.1:7999`).

## Usage

### Chat message history

```python
from yq_nova import Client
from langchain.memory import ConversationBufferMemory
from langchain_yq_nova import YqNovaChatMessageHistory

client = Client("http://127.0.0.1:7999")
history = YqNovaChatMessageHistory(client, namespace="alice")

history.add_message(HumanMessage(content="Hi, I like Rust"))
print(history.messages)

memory = ConversationBufferMemory(chat_memory=history, return_messages=True)
```

### BaseMemory provider

```python
from langchain_yq_nova import YqNovaMemory

provider = YqNovaMemory(client, namespace="alice", top_k=10)
provider.save_context({"input": "My name is Alice"}, {"output": "Nice to meet you Alice"})
vars = provider.load_memory_variables({"input": "What is my name?"})
print(vars["history"])
```

### Agent tools

```python
from langchain_yq_nova import create_all_tools, create_graph_tools, create_memory_tools

# Memory only: store, search and forget.
tools = create_memory_tools(client)

# Memory plus the knowledge graph.
tools = create_all_tools(client)

# Or just the graph half.
tools = create_graph_tools(client)
```

Register `tools` on your agent, then the model can store, search and forget
memories at runtime.

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