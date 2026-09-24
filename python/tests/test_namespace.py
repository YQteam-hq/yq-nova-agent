
import json
import os
import sys
import threading
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from yq_nova import Client

class _Recorder(BaseHTTPRequestHandler):

    captured = []

    def _handle(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        type(self).captured.append(
            {
                "method": self.command,
                "path": self.path,
                "namespace": self.headers.get("x-namespace"),
                "authorization": self.headers.get("Authorization"),
                "body": json.loads(raw.decode("utf-8")) if raw else None,
            }
        )
        payload = json.dumps({"ok": True}).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    do_GET = _handle
    do_POST = _handle
    do_PATCH = _handle
    do_DELETE = _handle

    def log_message(self, *args) -> None:

        return

@contextmanager
def _server():

    _Recorder.captured = []
    httpd = HTTPServer(("127.0.0.1", 0), _Recorder)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = httpd.server_address[0], httpd.server_address[1]
        yield "http://%s:%d" % (host, port), _Recorder.captured
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=5)

def test_namespace_header_is_sent():

    with _server() as (base_url, calls):
        client = Client(base_url, api_key="k", namespace="team-a")
        client.health()
        assert len(calls) == 1
        assert calls[0]["namespace"] == "team-a"
        assert calls[0]["authorization"] == "Bearer k"

def test_absent_namespace_sends_no_header():

    with _server() as (base_url, calls):
        client = Client(base_url)
        client.stats()
        assert calls[0]["namespace"] is None

def test_namespace_is_trimmed_before_sending():

    with _server() as (base_url, calls):
        Client(base_url, namespace="  team-a  ").health()
        assert calls[0]["namespace"] == "team-a"

def test_with_namespace_switches_tenant():

    with _server() as (base_url, calls):
        client = Client(base_url)
        client.health()
        returned = client.with_namespace("team-b")
        assert returned is client
        client.health()
        assert calls[0]["namespace"] is None
        assert calls[1]["namespace"] == "team-b"

def test_namespace_endpoints():

    with _server() as (base_url, calls):
        client = Client(base_url, namespace="team-a")
        client.list_namespaces(limit=5, offset=2)
        client.get_namespace("team/a")
        client.create_namespace("team-c", description="third")
        client.update_namespace("team-c", description="renamed")
        client.delete_namespace("team-c")

        assert [c["method"] for c in calls] == ["GET", "GET", "POST", "PATCH", "DELETE"]
        assert calls[0]["path"] == "/v1/namespaces?limit=5&offset=2"
        assert calls[1]["path"] == "/v1/namespaces/team%2Fa"
        assert calls[2]["path"] == "/v1/namespaces"
        assert calls[2]["body"] == {"name": "team-c", "description": "third", "config": {}}
        assert calls[3]["path"] == "/v1/namespaces/team-c"
        assert calls[3]["body"] == {"description": "renamed"}
        assert calls[4]["path"] == "/v1/namespaces/team-c"
        for call in calls:
            assert call["namespace"] == "team-a"

def test_namespace_validation():

    for call in (
        lambda: Client("http://127.0.0.1:7999", namespace=""),
        lambda: Client("http://127.0.0.1:7999").with_namespace("  "),
        lambda: Client("http://127.0.0.1:7999").get_namespace(""),
        lambda: Client("http://127.0.0.1:7999").create_namespace(""),
        lambda: Client("http://127.0.0.1:7999").create_namespace("Default"),
        lambda: Client("http://127.0.0.1:7999").update_namespace(" "),
        lambda: Client("http://127.0.0.1:7999").delete_namespace(""),
        lambda: Client("http://127.0.0.1:7999").list_namespaces(limit=0),
    ):
        try:
            call()
            assert False, "expected ValueError for %s" % (call,)
        except ValueError:
            pass

def _run_all():

    tests = [value for name, value in sorted(globals().items()) if name.startswith("test_")]
    for test in tests:
        test()
        print("ok  %s" % test.__name__)
    print("%d tests passed" % len(tests))

if __name__ == "__main__":
    _run_all()
