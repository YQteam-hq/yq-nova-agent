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
test("listNamespaces passes pagination params to the namespaced route", async () => {
  const { client, calls } = recordClient({ body: { total: 0, count: 0, limit: 25, offset: 50, items: [] } });
  const out = await client.listNamespaces({ limit: 25, offset: 50 });
  assert.equal(out.count, 0);
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/namespaces?limit=25&offset=50");
  assert.equal(calls[0].init.method, "GET");
  // The header is still required by the auth middleware to resolve the scope.
  assert.equal(calls[0].init.headers["x-namespace"], "default");
});

test("getNamespace percent-encodes the name", async () => {
  const { client, calls } = recordClient({ body: { id: 1, name: "team a/b" } });
  await client.getNamespace("team a/b");
  // Reserved characters must not be able to reshape the request path.
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/namespaces/team%20a%2Fb");
  assert.equal(calls[0].init.method, "GET");
});

test("createNamespace posts name, description and config", async () => {
  const { client, calls } = recordClient({ body: { id: 7, name: "team-a" } });
  await client.createNamespace({ name: "team-a", description: "Team A", config: { tier: "gold" } });
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/namespaces");
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    name: "team-a",
    description: "Team A",
    config: { tier: "gold" },
  });
});

test("createNamespace defaults description to null and config to an empty object", async () => {
  const { client, calls } = recordClient({ body: { id: 8, name: "team-b" } });
  await client.createNamespace({ name: "team-b" });
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    name: "team-b",
    description: null,
    config: {},
  });
});

test("createNamespace rejects the reserved name and unsafe characters", () => {
  const { client, calls } = recordClient();
  const reserved = ["default", "Default", "DEFAULT"];
  for (const name of reserved) {
    assert.throws(
      () => client.createNamespace({ name }),
      /'default' is a reserved name/,
      `expected rejection for reserved name ${name}`,
    );
  }
  for (const name of ["team a", "-team", "team\r\nX", ""]) {
    assert.throws(
      () => client.createNamespace({ name }),
      /createNamespace|reserved/,
      `expected rejection for unsafe name ${JSON.stringify(name)}`,
    );
  }
  assert.equal(calls.length, 0, "rejected names must not reach the transport");
});

test("createNamespace strips surrounding whitespace", async () => {
  const { client, calls } = recordClient({ body: { id: 9, name: "team-c" } });
  await client.createNamespace({ name: "  team-c  " });
  assert.equal(JSON.parse(calls[0].init.body).name, "team-c");
});

test("updateNamespace sends only the fields provided", async () => {
  const { client, calls } = recordClient({ body: { id: 7, name: "team-a" } });

  await client.updateNamespace("team-a", { description: "Updated" });
  assert.equal(calls[0].init.method, "PATCH");
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/namespaces/team-a");
  assert.deepEqual(JSON.parse(calls[0].init.body), { description: "Updated" });

  await client.updateNamespace("team-a", { config: { tier: "silver" } });
  assert.deepEqual(JSON.parse(calls[1].init.body), { config: { tier: "silver" } });

  await client.updateNamespace("team-a", { description: "Both", config: { k: 1 } });
  assert.deepEqual(JSON.parse(calls[2].init.body), { description: "Both", config: { k: 1 } });
});

test("updateNamespace rejects an empty name", () => {
  const { client, calls } = recordClient();
  assert.throws(() => client.updateNamespace("   "), /updateNamespace: name must not be empty/);
  assert.equal(calls.length, 0);
});

test("deleteNamespace issues a DELETE and surfaces the protected flag", async () => {
  const { client, calls } = recordClient({ body: { name: "default", deleted: false, protected: true } });
  const out = await client.deleteNamespace("default");
  assert.equal(out.protected, true);
  assert.equal(out.deleted, false);
  assert.equal(calls[0].init.method, "DELETE");
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/namespaces/default");
});

test("deleteNamespace rejects an empty name", () => {
  const { client, calls } = recordClient();
  assert.throws(() => client.deleteNamespace(""), /deleteNamespace: name must not be empty/);
  assert.equal(calls.length, 0);
});

test("namespace methods honour a scoped client namespace header", async () => {
  const { client, calls } = recordClient({ namespace: "team-a" });
  await client.listNamespaces();
  assert.equal(calls[0].init.headers["x-namespace"], "team-a");
});

// The stubs below return payloads shaped exactly like the server's serde
// output, so a mismatch between a declared return type and the real wire
// format shows up as a wrong field value rather than a silent `undefined`.

test("exportMemories returns the count/offset/items envelope", async () => {
  const record = {
    id: 1,
    uuid: "m1",
    namespace_id: 1,
    content: "hello",
    source: "agent",
    status: "active",
    importance: 0.5,
    metadata: {},
    tags: [],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    accessed_at: null,
    expires_at: null,
  };
  const { client, calls } = recordClient({ body: { count: 1, offset: 0, items: [record] } });
  const out = await client.exportMemories();
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/memory/export");
  assert.equal(calls[0].init.method, "POST");
  assert.equal(out.count, 1);
  assert.equal(out.offset, 0);
  assert.equal(out.items[0].uuid, "m1");
  assert.equal(out.items[0].content, "hello");
});

test("renameTag surfaces name and new_name", async () => {
  const { client, calls } = recordClient({ body: { name: "old", new_name: "new" } });
  const out = await client.renameTag("old", "new");
  assert.equal(calls[0].init.method, "PATCH");
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/tags/old");
  assert.deepEqual(JSON.parse(calls[0].init.body), { new_name: "new" });
  assert.equal(out.name, "old");
  assert.equal(out.new_name, "new");
});

test("deleteTag surfaces the affected memory count", async () => {
  const { client, calls } = recordClient({
    body: { name: "old", deleted: true, affected_memories: 7 },
  });
  const out = await client.deleteTag("old");
  assert.equal(calls[0].init.method, "DELETE");
  assert.equal(out.name, "old");
  assert.equal(out.deleted, true);
  assert.equal(out.affected_memories, 7);
});

test("traverse returns entity, depth and path per node", async () => {
  const entity = {
    id: 1,
    uuid: "e1",
    namespace_id: 1,
    name: "Alice",
    type: "person",
    description: null,
    metadata: {},
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
  const { client, calls } = recordClient({
    body: [{ entity, depth: 0, path: ["e1"] }],
  });
  const nodes = await client.traverse({ start: "e1" });
  assert.equal(calls[0].init.method, "POST");
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/graph/traverse");
  assert.equal(nodes[0].depth, 0);
  assert.equal(nodes[0].entity.name, "Alice");
  assert.deepEqual(nodes[0].path, ["e1"]);
});

test("extractAndLink returns candidate tuples and counters", async () => {
  const { client, calls } = recordClient({
    body: {
      entities: [[{ name: "Alice", entity_type: "person", description: null }, "e1"]],
      entities_upserted: 1,
      relations_created: 0,
      tags: ["people"],
    },
  });
  const out = await client.extractAndLink({ text: "Alice works with Bob" });
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/graph/extract-and-link");
  assert.equal(out.entities_upserted, 1);
  assert.equal(out.relations_created, 0);
  assert.deepEqual(out.tags, ["people"]);
  const [candidate, uuid] = out.entities[0];
  assert.equal(candidate.name, "Alice");
  assert.equal(candidate.entity_type, "person");
  assert.equal(candidate.description, null);
  assert.equal(uuid, "e1");
});

test("mergeEntities reads merged and remapped_relations, not discard_uuids", async () => {
  // The server responds with `merged` / `remapped_relations`; `discard_uuids`
  // is echoed only in the request. A type declaring the latter would compile
  // but yield undefined at runtime.
  const { client, calls } = recordClient({
    body: { kept_uuid: "e1", merged: ["e2", "e3"], remapped_relations: 4 },
  });
  const out = await client.mergeEntities({ keep_uuid: "e1", discard_uuids: ["e2", "e3"] });
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/graph/entities/merge");
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    keep_uuid: "e1",
    discard_uuids: ["e2", "e3"],
  });
  assert.equal(out.kept_uuid, "e1");
  assert.deepEqual(out.merged, ["e2", "e3"]);
  assert.equal(out.remapped_relations, 4);
  assert.equal("discard_uuids" in out, false, "the response must not contain discard_uuids");
});
