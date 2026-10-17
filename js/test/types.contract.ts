/**
 * Compile-time contract tests.
 *
 * These assertions never run; they exist so `tsc` fails the build if a
 * response type drifts from what the server actually returns. Runtime tests in
 * `client.test.mjs` cannot catch this class of mistake, because a stubbed
 * `fetch` will happily return a body that does not match the declared type.
 *
 * Run with `npm run typecheck:tests`.
 */
import type {
  ExportOutput,
  ExtractLinkOutput,
  MergeEntitiesOutput,
  TagDeleteOutput,
  TagRenameOutput,
  TraverseNode,
} from "../src/index.js";
import { NovaClient } from "../src/index.js";

declare const client: NovaClient;

/** Fail compilation unless `Actual` and `Expected` are mutually assignable. */
type Exact<Actual, Expected> = [Actual] extends [Expected]
  ? [Expected] extends [Actual]
    ? true
    : { expected: Expected; actual: Actual }
  : { expected: Expected; actual: Actual };

// --- exportMemories -------------------------------------------------------
export async function checkExportMemories() {
  const out = await client.exportMemories();
  const _count: Exact<typeof out.count, number> = true;
  const _offset: Exact<typeof out.offset, number> = true;
  const _items: Exact<typeof out.items, ExportOutput["items"]> = true;
  // Each exported item is a full memory record, not an opaque blob.
  const uuid: string = out.items[0].uuid;
  const content: string = out.items[0].content;
  return { _count, _offset, _items, uuid, content };
}

// --- renameTag / deleteTag ------------------------------------------------
export async function checkRenameTag() {
  const out = await client.renameTag("old", "new");
  const _exact: Exact<typeof out, TagRenameOutput> = true;
  const name: string = out.name;
  const newName: string = out.new_name;
  return { _exact, name, newName };
}

export async function checkDeleteTag() {
  const out = await client.deleteTag("old");
  const _exact: Exact<typeof out, TagDeleteOutput> = true;
  const name: string = out.name;
  const deleted: boolean = out.deleted;
  const affected: number = out.affected_memories;
  return { _exact, name, deleted, affected };
}

// --- traverse -------------------------------------------------------------
export async function checkTraverse() {
  const nodes = await client.traverse({ start: "u1" });
  const _elem: Exact<(typeof nodes)[number], TraverseNode> = true;
  const depth: number = nodes[0].depth;
  const name: string = nodes[0].entity.name;
  const path: string[] = nodes[0].path;
  return { _elem, depth, name, path };
}

// --- extractAndLink -------------------------------------------------------
export async function checkExtractAndLink() {
  const out = await client.extractAndLink({ text: "Alice knows Bob" });
  const _exact: Exact<typeof out, ExtractLinkOutput> = true;
  const upserted: number = out.entities_upserted;
  const created: number = out.relations_created;
  const tags: string[] = out.tags;
  // `entities` is a list of (candidate, uuid) tuples.
  const [candidate, uuid] = out.entities[0];
  const candidateName: string = candidate.name;
  const entityType: string = candidate.entity_type;
  return { _exact, upserted, created, tags, candidateName, entityType, uuid };
}

// --- mergeEntities --------------------------------------------------------
// Regression guard: the response is { kept_uuid, merged, remapped_relations }.
// It was previously declared as { kept_uuid, discard_uuids }, a field the
// server never sends, so `out.merged` did not type-check at all.
export async function checkMergeEntities() {
  const out = await client.mergeEntities({ keep_uuid: "a", discard_uuids: ["b"] });
  const _exact: Exact<typeof out, MergeEntitiesOutput> = true;
  const kept: string = out.kept_uuid;
  const merged: string[] = out.merged;
  const remapped: number = out.remapped_relations;
  return { _exact, kept, merged, remapped };
}
