
"""Namespace (multi-tenant) support for the Python client.

Drives the real ``Client`` against a throwaway ``http.server`` on a loopback
port and asserts on the requests the server actually received: the
``x-namespace`` header, the five admin endpoint paths/methods/bodies and
every client-side validation rule.

Standard library only: runs under a bare ``python3`` and under ``pytest``.
"""

import json
import os
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from yq_nova import Client


class _RecordingHandler(BaseHTTPRequestHandler):
    """Records every request and replies with a fixed JSON body."""

    def log_message(self, format, *args):  # silence stderr noise
        pass

    def _record(self):
        parsed = urllib.parse.urlparse(self.path)
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        body = json.loads(raw.decode("utf-8")) if raw else None
        self.server.requests.append(
            {
                "method": self.command,
                "path": parsed.path,
                "query": urllib.parse.parse_qs(parsed.query),
                "headers": dict(self.headers),
                "body": body,
            }
        )
        payload = json.dumps({"ok": True}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = _record
    do_POST = _record
    do_PATCH = _record
    do_DELETE = _record


class _Server:

    def __init__(self):
        self.httpd = HTTPServer(("127.0.0.1", 0), _RecordingHandler)
        self.httpd.requests = []
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *exc):
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join(timeout=5)

    @property
    def base_url(self):
        host, port = self.httpd.server_address
        return f"http://{host}:{port}"

    @property
    def requests(self):
        return self.httpd.requests

    def last(self):
        return self.requests[-1]


def _header(sent: dict, name: str):
    """Case-insensitive header lookup: http.server title-cases incoming names."""
    wanted = name.lower()
    for key, value in sent["headers"].items():
        if key.lower() == wanted:
            return value
    return None


def test_namespace_header_sent_when_configured():
    with _Server() as server:
        Client(server.base_url, namespace="team-acme").health()
        sent = server.last()
        assert _header(sent, "x-namespace") == "team-acme"


def test_namespace_header_absent_when_not_configured():
    with _Server() as server:
        Client(server.base_url).health()
        sent = server.last()
        assert _header(sent, "x-namespace") is None


def test_namespace_is_trimmed_on_construction():
    with _Server() as server:
        client = Client(server.base_url, namespace="  team-acme  ")
        assert client.namespace == "team-acme"
        client.health()
        assert _header(server.last(), "x-namespace") == "team-acme"


def test_with_namespace_retargets_in_place():
    with _Server() as server:
        client = Client(server.base_url, namespace="tenant-a")
        client.with_namespace("tenant-b")
        assert client.namespace == "tenant-b"
        client.health()
        assert _header(server.last(), "x-namespace") == "tenant-b"


def test_namespace_validation_rejects_blank():
    for blank in ("", "   ", "\t"):
        try:
            Client("http://127.0.0.1:1", namespace=blank)
            assert False, f"expected ValueError for namespace={blank!r}"
        except ValueError:
            pass
    try:
        Client("http://127.0.0.1:1").with_namespace("")
        assert False, "expected ValueError from with_namespace('')"
    except ValueError:
        pass


def test_list_namespaces_sends_limit_and_offset():
    with _Server() as server:
        Client(server.base_url).list_namespaces(limit=25, offset=50)
        sent = server.last()
        assert sent["method"] == "GET"
        assert sent["path"] == "/v1/namespaces"
        assert sent["query"] == {"limit": ["25"], "offset": ["50"]}


def test_get_namespace_percent_encodes_name():
    with _Server() as server:
        Client(server.base_url).get_namespace("team/a")
        sent = server.last()
        assert sent["method"] == "GET"
        assert sent["path"] == "/v1/namespaces/team%2Fa"


def test_create_namespace_body_and_reserved_default():
    with _Server() as server:
        client = Client(server.base_url)
        client.create_namespace("  new-tenant  ", description="desc", config={"quota": 100})
        sent = server.last()
        assert sent["method"] == "POST"
        assert sent["path"] == "/v1/namespaces"
        assert sent["body"] == {
            "name": "new-tenant",
            "description": "desc",
            "config": {"quota": 100},
        }

        try:
            client.create_namespace("default")
            assert False, "expected ValueError for reserved name 'default'"
        except ValueError:
            pass
        try:
            client.create_namespace("")
            assert False, "expected ValueError for empty name"
        except ValueError:
            pass


def test_update_namespace_sends_partial_body():
    with _Server() as server:
        Client(server.base_url).update_namespace("team-acme", description="renamed")
        sent = server.last()
        assert sent["method"] == "PATCH"
        assert sent["path"] == "/v1/namespaces/team-acme"
        assert sent["body"] == {"description": "renamed", "config": None}


def test_delete_namespace_uses_delete_verb():
    with _Server() as server:
        Client(server.base_url).delete_namespace("team-acme")
        sent = server.last()
        assert sent["method"] == "DELETE"
        assert sent["path"] == "/v1/namespaces/team-acme"


if __name__ == "__main__":
    test_namespace_header_sent_when_configured()
    test_namespace_header_absent_when_not_configured()
    test_namespace_is_trimmed_on_construction()
    test_with_namespace_retargets_in_place()
    test_namespace_validation_rejects_blank()
    test_list_namespaces_sends_limit_and_offset()
    test_get_namespace_percent_encodes_name()
    test_create_namespace_body_and_reserved_default()
    test_update_namespace_sends_partial_body()
    test_delete_namespace_uses_delete_verb()
    print("10 tests passed")
