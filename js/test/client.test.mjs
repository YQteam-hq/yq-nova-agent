import { test } from "node:test";
import assert from "node:assert/strict";
import { NovaClient, NovaApiError } from "../dist/index.js";

function recordClient(opts = {}, clientOpts = {}) {
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
    fetch: stubFetch,
    ...clientOpts,
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
test("namespace header is sent when configured", async () => {
  const { client, calls } = recordClient({}, { namespace: "team-acme" });
  await client.health();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].init.headers["x-namespace"], "team-acme");
});

test("namespace header is omitted when not configured", async () => {
  const { client, calls } = recordClient();
  await client.health();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].init.headers["x-namespace"], undefined);
});

test("namespace is trimmed and can be switched at runtime", async () => {
  const { client, calls } = recordClient({}, { namespace: "  team-acme  " });
  assert.equal(client.namespace, "team-acme");
  await client.health();
  assert.equal(calls[0].init.headers["x-namespace"], "team-acme");

  const chained = client.withNamespace("team-beta");
  assert.equal(chained, client);
  assert.equal(client.namespace, "team-beta");
  await client.health();
  assert.equal(calls[1].init.headers["x-namespace"], "team-beta");
});

test("namespace admin endpoints hit the right URLs", async () => {
  const ns = { id: 7, uuid: "u", name: "team-acme", description: null, config: {}, created_at: "t", updated_at: "t" };
  const { client, calls } = recordClient({ body: { total: 1, count: 1, limit: 100, offset: 0, items: [ns] } });

  await client.listNamespaces(25, 50);
  assert.equal(calls[0].url, "http://127.0.0.1:7999/v1/namespaces?limit=25&offset=50");
  assert.equal(calls[0].init.method, "GET");

  await client.getNamespace("team/a");
  assert.equal(calls[1].url, "http://127.0.0.1:7999/v1/namespaces/team%2Fa");
  assert.equal(calls[1].init.method, "GET");

  await client.deleteNamespace("team-acme");
  assert.equal(calls[2].url, "http://127.0.0.1:7999/v1/namespaces/team-acme");
  assert.equal(calls[2].init.method, "DELETE");
});

test("create and update namespace bodies", async () => {
  const { client, calls } = recordClient({ body: { id: 1 } });
  await client.createNamespace({ name: "  new-tenant  ", description: "desc", config: { quota: 100 } });
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    name: "new-tenant",
    description: "desc",
    config: { quota: 100 },
  });

  await client.updateNamespace("team-acme", { description: "renamed" });
  assert.equal(calls[1].init.method, "PATCH");
  assert.deepEqual(JSON.parse(calls[1].init.body), { description: "renamed" });
});

test("namespace validation rules", () => {
  assert.throws(() => new NovaClient("http://127.0.0.1:7999", { namespace: "" }), /must not be empty/);
  assert.throws(() => new NovaClient("http://127.0.0.1:7999", { namespace: "   " }), /must not be empty/);

  const { client } = recordClient();
  assert.throws(() => client.withNamespace(""), /must not be empty/);

  for (const call of [
    () => client.createNamespace({ name: "" }),
    () => client.createNamespace({ name: "default" }),
    () => client.getNamespace(""),
    () => client.updateNamespace("  "),
    () => client.deleteNamespace(""),
  ]) {
    assert.throws(call);
  }
});
