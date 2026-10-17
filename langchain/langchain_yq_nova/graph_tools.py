"""Knowledge-graph tools for LangChain agents.

The tools in `tools.py` cover the memory surface (remember / recall / forget).
This module adds the graph surface so an agent can inspect and extend the
entity-relation graph that recall also traverses: look up entities, walk the
graph, list or create edges, and pull entities and relations out of free text.

Every tool takes the same `yq_nova.Client`, so a single client (and therefore a
single namespace and API key) backs the whole tool set.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional

from langchain_core.tools import StructuredTool

from yq_nova import Client


def create_list_entities_tool(
    client: Client,
    name: str = "nova_list_entities",
    limit_default: int = 20,
) -> StructuredTool:
    def list_entities(
        name_prefix: Optional[str] = None,
        entity_type: Optional[str] = None,
        limit: int = limit_default,
    ) -> List[Dict[str, Any]]:
        if limit < 1:
            raise ValueError("limit must be >= 1")
        return client.list_entities(
            name_prefix=name_prefix or None,
            entity_type=entity_type or None,
            limit=limit,
        )

    return StructuredTool.from_function(
        func=list_entities,
        name=name,
        description=(
            "Look up knowledge-graph entities by name prefix and/or entity type. "
            "Use this before traversing so you know which entity uuid to start from."
        ),
    )


def create_traverse_tool(
    client: Client,
    name: str = "nova_traverse",
    max_depth_default: int = 2,
    max_nodes_default: int = 50,
) -> StructuredTool:
    def traverse(
        start: str,
        max_depth: int = max_depth_default,
        max_nodes: int = max_nodes_default,
        predicate_whitelist: Optional[List[str]] = None,
        min_confidence: float = 0.0,
    ) -> List[Dict[str, Any]]:
        if not start or not start.strip():
            raise ValueError("start must be a non-empty entity uuid")
        if max_depth < 1:
            raise ValueError("max_depth must be >= 1")
        if max_nodes < 1:
            raise ValueError("max_nodes must be >= 1")
        return client.traverse(
            start=start,
            max_depth=max_depth,
            max_nodes=max_nodes,
            predicate_whitelist=predicate_whitelist or [],
            min_confidence=min_confidence,
        )

    return StructuredTool.from_function(
        func=traverse,
        name=name,
        description=(
            "Walk the knowledge graph outward from an entity uuid and return the "
            "connected subgraph. Optionally restrict which predicates are followed."
        ),
    )


def create_list_relations_tool(
    client: Client,
    name: str = "nova_list_relations",
    limit_default: int = 50,
) -> StructuredTool:
    def list_relations(
        source: Optional[str] = None,
        target: Optional[str] = None,
        predicate: Optional[str] = None,
        limit: int = limit_default,
    ) -> List[Dict[str, Any]]:
        if limit < 1:
            raise ValueError("limit must be >= 1")
        return client.list_relations(
            source=source or None,
            target=target or None,
            predicate=predicate or None,
            limit=limit,
        )

    return StructuredTool.from_function(
        func=list_relations,
        name=name,
        description=(
            "List knowledge-graph relations, optionally filtering by source uuid, "
            "target uuid and/or predicate. With no filter, returns all relations."
        ),
    )


def create_link_entities_tool(client: Client, name: str = "nova_link_entities") -> StructuredTool:
    def link_entities(
        source_uuid: str,
        target_uuid: str,
        predicate: str,
        confidence: float = 1.0,
    ) -> Dict[str, Any]:
        if not source_uuid or not source_uuid.strip():
            raise ValueError("source_uuid must be non-empty")
        if not target_uuid or not target_uuid.strip():
            raise ValueError("target_uuid must be non-empty")
        if not predicate or not predicate.strip():
            raise ValueError("predicate must be non-empty")
        if not 0.0 <= confidence <= 1.0:
            raise ValueError("confidence must be between 0.0 and 1.0")
        return client.upsert_relation(
            source_uuid=source_uuid,
            target_uuid=target_uuid,
            predicate=predicate,
            confidence=confidence,
        )

    return StructuredTool.from_function(
        func=link_entities,
        name=name,
        description=(
            "Create or strengthen a typed relation between two existing entities. "
            "Both entities must already exist; look them up with nova_list_entities first."
        ),
    )


def create_extract_graph_tool(client: Client, name: str = "nova_extract_graph") -> StructuredTool:
    def extract_graph(text: str, create_relations: bool = True) -> Dict[str, Any]:
        if not text or not text.strip():
            raise ValueError("text must be non-empty")
        return client.extract_and_link(
            text,
            opts={
                "enabled": True,
                "upsert_entities": True,
                "create_relations": create_relations,
                "min_confidence": 0.0,
            },
        )

    return StructuredTool.from_function(
        func=extract_graph,
        name=name,
        description=(
            "Extract entities and relations from text using the configured LLM "
            "extractor and link them into the knowledge graph. Requires the server "
            "to have graph.extract_llm configured."
        ),
    )


def create_graph_tools(client: Client) -> List[StructuredTool]:
    """Return the full graph tool set for `client`."""
    return [
        create_list_entities_tool(client),
        create_traverse_tool(client),
        create_list_relations_tool(client),
        create_link_entities_tool(client),
        create_extract_graph_tool(client),
    ]


def create_all_tools(client: Client) -> List[StructuredTool]:
    """Return memory tools plus graph tools.

    Convenience for the common case where an agent should be able to both
    store/retrieve memories and reason over the entity graph.
    """
    from .tools import create_memory_tools

    return create_memory_tools(client) + create_graph_tools(client)
