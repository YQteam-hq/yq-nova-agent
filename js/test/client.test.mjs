import { test } from "node:test";
import assert from "node:assert/strict";
import { NovaClient, NovaApiError } from "../dist/index.js";

function recordClient(opts = {}) {
  const calls = [];
  const stubFetch = async (url, init) => {
    calls.push({ url, init });
    const status = opts.status ?? 200;
    if (opts.raw !== undefined) {
      return new Response(opts.raw, { status, headers: { "content-type": "text/plain" } });
    }
    const body = opts.body ?? { ok: true };
    return new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  };
  const client = new NovaClient("http://127.0.0.1:7999", {
    apiKey: "k",
    namespace: opts.namespace,
    fetch: stubFetch,
  });
  return { client, calls };
}

test("remember posts JSON to the right path", async () => {
  const { client, calls } = recordClient({
    body: { uuid: "u1", duplicate: false },
  });
  const out = await client.remember({ content: "hello", tags: ["a"] });
  assert.equal(out.uuid, "u1");
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/memory/remember");
  assert.equal(calls[0].init.method, "POST");
  assert.equal(calls[0].init.headers["Authorization"], "Bearer k");
  assert.deepEqual(JSON.parse(calls[0].init.body), { content: "hello", tags: ["a"] });
});

test("GET endpoints encode query params", async () => {
  const { client, calls } = recordClient({ body: [] });
  await client.listRelations({ source: "s", limit: 5, offset: 2 });
  await client.listTags({ limit: 10 });
  assert.equal(
    calls[0].url,
    "http://127.0.0.1:7999/v1/graph/relations?source=s&limit=5&offset=2",
  );
  assert.equal(calls[1].url, "http://127.0.0.1:7999/v1/tags?limit=10");
});

test("path segments are URL encoded", async () => {
  const { client, calls } = recordClient({ body: {} });
  await client.renameTag("a/b", "c");
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/tags/a%2Fb");
});

test("listTags returns a paged output", async () => {
  const { client } = recordClient({
    body: {
      total: 1,
      count: 1,
      limit: 10,
      offset: 0,
      items: [{ id: 1, name: "a", color: null, memory_count: 0 }],
    },
  });
  const out = await client.listTags({ limit: 10 });
  assert.equal(out.total, 1);
  assert.equal(out.items.length, 1);
  assert.equal(out.items[0].name, "a");
});

test("invalid JSON body throws instead of silently returning empty", async () => {
  const { client } = recordClient({ raw: "<html>oops</html>", status: 200 });
  await assert.rejects(
    () => client.health(),
    (err) =>
      err instanceof NovaApiError &&
      err.status === 200 &&
      err.message.includes("invalid JSON response body"),
  );
});

test("non-2xx responses throw NovaApiError", async () => {
  const { client } = recordClient({
    status: 404,
    body: { code: "not_found", message: "missing", trace_id: "t1" },
  });
  await assert.rejects(
    () => client.getMemory("nope"),
    (err) =>
      err instanceof NovaApiError &&
      err.code === "not_found" &&
      err.status === 404 &&
      err.traceId === "t1",
  );
});

test("empty base url is rejected", () => {
  assert.throws(() => new NovaClient(""), /must not be empty/);
});

test("x-namespace header defaults to \"default\" and can be configured", async () => {
  const def = recordClient();
  await def.client.health();
  assert.equal(def.calls[0].init.headers["x-namespace"], "default");

  const scoped = recordClient({ namespace: "team-a" });
  await scoped.client.health();
  assert.equal(scoped.calls[0].init.headers["x-namespace"], "team-a");

  // Namespaces are stripped of surrounding whitespace.
  const padded = recordClient({ namespace: "  team-a  " });
  await padded.client.health();
  assert.equal(padded.calls[0].init.headers["x-namespace"], "team-a");
});

test("unsafe namespace names are rejected", () => {
  // The namespace is sent in the x-namespace header, so characters that could
  // split or forge a header must be rejected.
  for (const name of [
    "team\na", // newline injection
    "team\r\nX-Evil: 1", // CRLF header splitting
    "team a", // space inside the name
    "team:a", // colon
    "-team", // must start alphanumeric
    "team/../etc", // path characters
    "team\u00e9", // non-ASCII letter
    "team\x00", // NUL byte
  ]) {
    assert.throws(
      () => new NovaClient("http://127.0.0.1:7999", { namespace: name, fetch: async () => {} }),
      /invalid characters/,
      `expected rejection for ${JSON.stringify(name)}`,
    );
  }
  assert.throws(
    () => new NovaClient("http://127.0.0.1:7999", { namespace: "   ", fetch: async () => {} }),
    /must not be empty/,
  );
});

test("withNamespace returns a scoped copy carrying all options", async () => {
  const { client, calls } = recordClient({ namespace: "team-a" });
  const scoped = client.withNamespace("team-b");
  assert.notEqual(scoped, client);

  await scoped.remember({ content: "hello" });
  assert.equal(calls[0].init.headers["x-namespace"], "team-b");
  assert.equal(calls[0].init.headers["Authorization"], "Bearer k");
  assert.equal(calls[0].init.headers["Content-Type"], "application/json");

  // The original client is untouched and still sends its own namespace.
  await client.remember({ content: "hello" });
  assert.equal(calls[1].init.headers["x-namespace"], "team-a");

  // The scoped client validates its input too.
  assert.throws(() => client.withNamespace("bad name"), /invalid characters/);
});

test("withNamespace reports validation errors under its own name", () => {
  // The `where` label must name the method the caller used, so the message
  // never says "namespace" for a `withNamespace` call.
  const { client } = recordClient();
  const startsWith = (prefix) => (err) =>
    err instanceof Error && err.message.startsWith(prefix);
  assert.throws(
    () => client.withNamespace("bad name"),
    startsWith("withNamespace: 'bad name' contains invalid characters"),
  );
  assert.throws(() => client.withNamespace("   "), startsWith("withNamespace: name must not be empty"));
  // The constructor keeps its own label.
  assert.throws(
    () => new NovaClient("http://127.0.0.1:7999", { namespace: "bad name" }),
    startsWith("namespace: 'bad name' contains invalid characters"),
  );
});

test("withNamespace carries timeoutMs over", async () => {
  const calls = [];
  // A fetch stub that honours AbortSignal, so the timeout is observable.
  const slowFetch = (url, init) =>
    new Promise((resolve, reject) => {
      calls.push({ url, init });
      const timer = setTimeout(() => resolve(new Response("{}")), 200);
      init.signal?.addEventListener("abort", () => {
        clearTimeout(timer);
        const err = new Error("aborted");
        err.name = "AbortError";
        reject(err);
      });
    });
  const client = new NovaClient("http://127.0.0.1:7999", {
    apiKey: "k",
    timeoutMs: 10,
    fetch: slowFetch,
  });
  const scoped = client.withNamespace("team-b");
  await assert.rejects(
    () => scoped.health(),
    (err) => err instanceof NovaApiError && err.code === "timeout",
    "the scoped client must inherit the original timeout",
  );
  assert.equal(calls[0].init.headers["Authorization"], "Bearer k");
  assert.equal(calls[0].init.headers["x-namespace"], "team-b");
});

test("invalid timeoutMs is rejected", () => {
  const stub = async () => new Response("{}", { status: 200 });
  for (const timeoutMs of [-1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(
      () => new NovaClient("http://127.0.0.1:7999", { timeoutMs, fetch: stub }),
      /timeoutMs must be a finite/,
      `expected rejection for timeoutMs=${timeoutMs}`,
    );
  }
});