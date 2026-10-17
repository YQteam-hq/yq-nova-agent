
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from yq_nova import Client, NovaApiError  

def test_imports_and_construction():
    client = Client("http://127.0.0.1:7999")
    assert client.base_url == "http://127.0.0.1:7999"

    client2 = Client("http://127.0.0.1:7999/")
    assert client2.base_url == "http://127.0.0.1:7999"

    try:
        Client("")
        assert False, "expected ValueError for empty base_url"
    except ValueError:
        pass

def test_error_type_attributes():
    err = NovaApiError("not_found", "boom", 404, trace_id="abc")
    assert err.code == "not_found"
    assert err.message == "boom"
    assert err.status == 404
    assert err.trace_id == "abc"

def test_client_side_validation():
    client = Client("http://127.0.0.1:7999")

    for call in (
        lambda: client.remember(""),
        lambda: client.recall(""),
        lambda: client.recall("q", top_k=0),
        lambda: client.forget(),
        lambda: client.get_memory(""),
        lambda: client.upsert_entity(""),
        lambda: client.traverse(""),
        lambda: client.extract_and_link(""),
    ):
        try:
            call()
            assert False, f"expected ValueError for {call}"
        except ValueError:
            pass

def test_namespace_scoping():
    client = Client("http://127.0.0.1:7999")
    assert client.namespace == "default"

    scoped = client.with_namespace("team-a")
    assert scoped.namespace == "team-a"
    assert client.namespace == "default"

    direct = Client("http://127.0.0.1:7999", namespace="team-b")
    assert direct.namespace == "team-b"

    try:
        Client("http://127.0.0.1:7999", namespace="")
        assert False, "expected ValueError for empty namespace"
    except ValueError:
        pass

    try:
        client.with_namespace("   ")
        assert False, "expected ValueError for blank namespace"
    except ValueError:
        pass

def test_namespace_rejects_unsafe_characters():
    # The namespace is sent in the x-namespace header, so characters that
    # could split or forge a header must be rejected.
    client = Client("http://127.0.0.1:7999")
    unsafe = [
        "team\na",              # newline injection
        "team\r\nX-Evil: 1",    # CRLF header splitting
        "team a",               # space inside the name
        "team:a",               # colon
        "-team",                # must start alphanumeric
        "team/../etc",          # path characters
        "t\u00e9am",            # non-ASCII letter
        "team\x00",             # NUL byte
    ]
    for name in unsafe:
        try:
            Client("http://127.0.0.1:7999", namespace=name)
            assert False, f"expected ValueError for namespace {name!r}"
        except ValueError:
            pass
        try:
            client.with_namespace(name)
            assert False, f"expected ValueError for with_namespace({name!r})"
        except ValueError:
            pass

    # Safe names are accepted and stripped.
    for name in ["team-a", "team_b", "Team9", "  padded  "]:
        c = Client("http://127.0.0.1:7999", namespace=name)
        assert c.namespace == name.strip()

def test_namespace_regex_boundaries():
    # Direct coverage for the _NAMESPACE_RE pattern itself, independent of
    # the Client wrapper: the first character must be alphanumeric and the
    # remainder may only contain alphanumerics, hyphens and underscores.
    from yq_nova.client import _NAMESPACE_RE

    valid = [
        "a", "Z", "0", "9", "team-a", "team_b", "A-b_C-9",
        "x" * 512, "0" * 100, "-".join("abcdefghij"),
    ]
    for name in valid:
        assert _NAMESPACE_RE.match(name), f"expected {name!r} to match"

    invalid = [
        "",                  # empty
        "-team",             # leading hyphen
        "_team",             # leading underscore
        "team a",            # embedded space
        "team\ta",           # embedded tab
        "team\na",           # embedded newline
        "team\r\nX-Evil: 1", # CRLF header splitting
        "team:a",            # colon
        "team/../etc",       # path characters
        "t\u00e9am",         # non-ASCII letter
        "team\x00",          # NUL byte
        "team.",             # dot
        "team+",             # plus sign
        "team%20",           # percent-encoding
        "team;",             # semicolon
    ]
    for name in invalid:
        assert not _NAMESPACE_RE.match(name), f"expected {name!r} to be rejected"

def test_with_namespace_copies_all_attributes():
    client = Client("http://127.0.0.1:7999", api_key="secret-key", timeout=12.5)
    scoped = client.with_namespace("team-a")
    # The scoped client must carry over every configuration attribute, not
    # just the ones enumerated in its implementation.
    assert scoped is not client
    assert scoped.base_url == client.base_url
    assert scoped.api_key == client.api_key
    assert scoped.timeout == client.timeout
    assert scoped.namespace == "team-a"
    assert client.namespace == "default"

def _recording_client():
    """Return (client, calls) where `calls` records every _request invocation.

    The namespace helpers build a URL and body before touching the network, so
    swapping `_request` lets the smoke tests assert on the outgoing call
    without a live server.
    """
    client = Client("http://127.0.0.1:7999", api_key="secret-key")
    calls = []

    def recorder(method, path, body=None, params=None):
        calls.append({"method": method, "path": path, "body": body, "params": params})
        return {"ok": True}

    client._request = recorder
    return client, calls

def test_list_namespaces_sends_pagination_params():
    client, calls = _recording_client()
    client.list_namespaces(limit=25, offset=50)

    assert len(calls) == 1
    call = calls[0]
    assert call["method"] == "GET"
    assert call["path"] == "/v1/namespaces"
    assert call["params"] == {"limit": 25, "offset": 50}
    assert call["body"] is None

def test_list_namespaces_defaults_and_bounds():
    client, calls = _recording_client()
    client.list_namespaces()
    assert calls[0]["params"] == {"limit": 100, "offset": 0}

    for bad in (
        lambda: client.list_namespaces(limit=0),
        lambda: client.list_namespaces(limit=-1),
        lambda: client.list_namespaces(offset=-1),
    ):
        try:
            bad()
            assert False, "expected ValueError from list_namespaces bounds check"
        except ValueError:
            pass

    assert len(calls) == 1, "rejected calls must not reach the transport"

def test_get_namespace_percent_encodes_the_name():
    client, calls = _recording_client()
    client.get_namespace("team a/b")

    assert calls[0]["method"] == "GET"
    # Reserved characters must be escaped so they cannot alter the path.
    assert calls[0]["path"] == "/v1/namespaces/team%20a%2Fb"

def test_create_namespace_sends_name_description_and_config():
    client, calls = _recording_client()
    client.create_namespace("team-a", description="Team A", config={"tier": "gold"})

    call = calls[0]
    assert call["method"] == "POST"
    assert call["path"] == "/v1/namespaces"
    assert call["body"] == {
        "name": "team-a",
        "description": "Team A",
        "config": {"tier": "gold"},
    }

def test_create_namespace_defaults_config_to_empty_object():
    client, calls = _recording_client()
    client.create_namespace("team-b")

    assert calls[0]["body"] == {"name": "team-b", "description": None, "config": {}}

def test_create_namespace_rejects_reserved_and_unsafe_names():
    client, calls = _recording_client()

    for bad in (
        "default",     # reserved by the server
        "Default",     # the check is case-insensitive
        "DEFAULT",
        "team\r\nX",   # header injection attempt
        "team a",      # space
        "-team",       # must start with alphanumeric
        "",            # empty
        "   ",         # whitespace only
    ):
        try:
            client.create_namespace(bad)
            assert False, f"expected ValueError for create_namespace({bad!r})"
        except ValueError:
            pass

    assert calls == [], "rejected names must not reach the transport"

def test_create_namespace_strips_surrounding_whitespace():
    client, calls = _recording_client()
    client.create_namespace("  team-c  ")

    assert calls[0]["body"]["name"] == "team-c"

def test_update_namespace_omits_untouched_fields():
    client, calls = _recording_client()
    client.update_namespace("team-a", description="Updated")

    call = calls[0]
    assert call["method"] == "PATCH"
    assert call["path"] == "/v1/namespaces/team-a"
    # `config` is absent so the server keeps the stored value.
    assert call["body"] == {"description": "Updated"}

    client.update_namespace("team-a", config={"tier": "silver"})
    assert calls[1]["body"] == {"config": {"tier": "silver"}}

    client.update_namespace("team-a", description="Both", config={"k": 1})
    assert calls[2]["body"] == {"description": "Both", "config": {"k": 1}}

def test_update_namespace_rejects_empty_name():
    client, calls = _recording_client()
    try:
        client.update_namespace("  ", description="x")
        assert False, "expected ValueError for empty update_namespace name"
    except ValueError:
        pass
    assert calls == []

def test_delete_namespace_uses_delete_verb():
    client, calls = _recording_client()
    client.delete_namespace("team-a")

    assert calls[0]["method"] == "DELETE"
    assert calls[0]["path"] == "/v1/namespaces/team-a"

    try:
        client.delete_namespace("")
        assert False, "expected ValueError for empty delete_namespace name"
    except ValueError:
        pass
    assert len(calls) == 1

if __name__ == "__main__":
    test_imports_and_construction()
    test_error_type_attributes()
    test_client_side_validation()
    test_namespace_scoping()
    test_namespace_rejects_unsafe_characters()
    test_namespace_regex_boundaries()
    test_with_namespace_copies_all_attributes()
    test_list_namespaces_sends_pagination_params()
    test_list_namespaces_defaults_and_bounds()
    test_get_namespace_percent_encodes_the_name()
    test_create_namespace_sends_name_description_and_config()
    test_create_namespace_defaults_config_to_empty_object()
    test_create_namespace_rejects_reserved_and_unsafe_names()
    test_create_namespace_strips_surrounding_whitespace()
    test_update_namespace_omits_untouched_fields()
    test_update_namespace_rejects_empty_name()
    test_delete_namespace_uses_delete_verb()
    print("all smoke tests passed")