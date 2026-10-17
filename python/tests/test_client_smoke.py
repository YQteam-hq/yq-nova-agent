
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

if __name__ == "__main__":
    test_imports_and_construction()
    test_error_type_attributes()
    test_client_side_validation()
    test_namespace_scoping()
    test_namespace_rejects_unsafe_characters()
    print("all smoke tests passed")