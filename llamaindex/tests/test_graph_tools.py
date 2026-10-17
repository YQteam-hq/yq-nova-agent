"""Behavioural tests for the LlamaIndex graph tools.

These exercise the tool wrappers without a live server: the yq_nova client's
`_request` is swapped for a recorder, so each test can assert on the exact
method, path, query params and body the tool would have sent. The wrappers'
own argument validation is checked directly.

Run with `python3 langchain/tests/test_graph_tools.py` or under pytest.
"""

import os
import sys

_REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(_REPO_ROOT, "python"))
sys.path.insert(0, os.path.join(_REPO_ROOT, "llamaindex"))

from yq_nova import Client  # noqa: E402

import llama_index_yq_nova as pkg  # noqa: E402


def recording_client():
    """Return (client, calls) where calls records every outgoing request."""
    client = Client("http://127.0.0.1:7999", api_key="secret-key")
    calls = []

    def recorder(method, path, body=None, params=None):
        calls.append({"method": method, "path": path, "body": body, "params": params})
        return {"ok": True}

    client._request = recorder
    return client, calls


def expect_value_error(fn, label):
    try:
        fn()
        raise AssertionError(f"expected ValueError for {label}")
    except ValueError:
        pass


def tool_name(tool):
    """LlamaIndex carries the name inside `metadata`, unlike LangChain."""
    return tool.metadata.name


def test_all_tools_are_well_formed():
    client, _ = recording_client()
    tools = pkg.create_all_tools(client)

    assert len(tools) == 8, f"expected 8 tools, got {len(tools)}"
    names = [tool_name(t) for t in tools]
    assert len(set(names)) == len(names), f"tool names must be unique: {names}"
    for tool in tools:
        # An LLM can only call a tool that advertises both a description and an
        # introspectable argument schema.
        assert tool.metadata.description, f"{tool_name(tool)} is missing a description"
        assert tool.metadata.fn_schema is not None, f"{tool_name(tool)} is missing a fn schema"
        assert callable(tool.fn), f"{tool_name(tool)} is not directly callable"

    assert [tool_name(t) for t in pkg.create_graph_tools(client)] == [
        "nova_list_entities",
        "nova_traverse",
        "nova_list_relations",
        "nova_link_entities",
        "nova_extract_graph",
    ]


def test_list_entities_forwards_filters():
    client, calls = recording_client()
    tool = pkg.create_list_entities_tool(client)

    tool.fn(name_prefix="Al", entity_type="person", limit=5)
    assert calls[-1]["method"] == "GET"
    assert calls[-1]["path"] == "/v1/graph/entities"
    assert calls[-1]["params"]["name_prefix"] == "Al"
    assert calls[-1]["params"]["entity_type"] == "person"
    assert calls[-1]["params"]["limit"] == 5

    # An empty string means "no filter" rather than "filter by empty string".
    tool.fn(name_prefix="", entity_type="")
    assert calls[-1]["params"] == {"limit": 20, "offset": 0}

    expect_value_error(lambda: tool.fn(limit=0), "limit=0")


def test_traverse_forwards_graph_options():
    client, calls = recording_client()
    tool = pkg.create_traverse_tool(client)

    tool.fn(start="u1", max_depth=2)
    assert calls[-1]["method"] == "POST"
    assert calls[-1]["path"] == "/v1/graph/traverse"
    body = calls[-1]["body"]
    assert body["start"] == "u1"
    assert body["max_depth"] == 2
    # The default caps traversal so an agent cannot walk an unbounded graph.
    assert body["max_nodes"] == 50
    assert body["predicate_whitelist"] == []

    tool.fn(start="u1", predicate_whitelist=["knows"], min_confidence=0.5)
    assert calls[-1]["body"]["predicate_whitelist"] == ["knows"]
    assert calls[-1]["body"]["min_confidence"] == 0.5

    for bad, label in (
        (lambda: tool.fn(start="  "), "blank start"),
        (lambda: tool.fn(start="u", max_depth=0), "max_depth=0"),
        (lambda: tool.fn(start="u", max_nodes=0), "max_nodes=0"),
    ):
        expect_value_error(bad, label)


def test_list_relations_forwards_endpoint_filters():
    client, calls = recording_client()
    tool = pkg.create_list_relations_tool(client)

    tool.fn(predicate="knows")
    assert calls[-1]["path"] == "/v1/graph/relations"
    assert calls[-1]["params"]["predicate"] == "knows"

    tool.fn(source="a", target="b")
    assert calls[-1]["params"]["source"] == "a"
    assert calls[-1]["params"]["target"] == "b"

    # Omitted filters are left out entirely instead of being sent as blanks.
    tool.fn()
    assert calls[-1]["params"] == {"limit": 50, "offset": 0}

    expect_value_error(lambda: tool.fn(limit=0), "limit=0")


def test_link_entities_validates_before_writing():
    client, calls = recording_client()
    tool = pkg.create_link_entities_tool(client)

    tool.fn(source_uuid="a", target_uuid="b", predicate="knows")
    assert calls[-1]["method"] == "POST"
    assert calls[-1]["path"] == "/v1/graph/relations"
    assert calls[-1]["body"]["confidence"] == 1.0

    tool.fn(source_uuid="a", target_uuid="b", predicate="k", confidence=0.25)
    assert calls[-1]["body"]["confidence"] == 0.25

    for bad, label in (
        (lambda: tool.fn(source_uuid="", target_uuid="b", predicate="k"), "blank source"),
        (lambda: tool.fn(source_uuid="a", target_uuid="", predicate="k"), "blank target"),
        (lambda: tool.fn(source_uuid="a", target_uuid="b", predicate=" "), "blank predicate"),
        (lambda: tool.fn(source_uuid="a", target_uuid="b", predicate="k", confidence=1.5), "confidence > 1"),
        (lambda: tool.fn(source_uuid="a", target_uuid="b", predicate="k", confidence=-0.1), "confidence < 0"),
    ):
        expect_value_error(bad, label)

    # None of the rejected calls may have reached the transport.
    assert len(calls) == 2, f"rejected input reached the transport: {len(calls)} calls"


def test_extract_graph_sets_opts_explicitly():
    client, calls = recording_client()
    tool = pkg.create_extract_graph_tool(client)

    tool.fn(text="Alice works with Bob")
    assert calls[-1]["path"] == "/v1/graph/extract-and-link"
    assert calls[-1]["body"]["text"] == "Alice works with Bob"
    assert calls[-1]["body"]["opts"] == {
        "enabled": True,
        "upsert_entities": True,
        "create_relations": True,
        "min_confidence": 0.0,
    }

    tool.fn(text="Alice works with Bob", create_relations=False)
    assert calls[-1]["body"]["opts"]["create_relations"] is False

    expect_value_error(lambda: tool.fn(text=""), "blank text")


def test_memory_tools_still_work_alongside_graph_tools():
    client, _ = recording_client()
    memory_tools = pkg.create_memory_tools(client)
    assert len(memory_tools) == 3
    assert memory_tools[0].fn(content="hello") is not None

    # create_all_tools must not mutate the caller's client namespace.
    assert client.namespace == "default"


if __name__ == "__main__":
    test_all_tools_are_well_formed()
    test_list_entities_forwards_filters()
    test_traverse_forwards_graph_options()
    test_list_relations_forwards_endpoint_filters()
    test_link_entities_validates_before_writing()
    test_extract_graph_sets_opts_explicitly()
    test_memory_tools_still_work_alongside_graph_tools()
    print("all llamaindex graph tool tests passed")
