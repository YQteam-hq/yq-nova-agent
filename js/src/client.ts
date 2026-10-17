import type {
  BatchRememberInput,
  BatchRememberOutput,
  CreateNamespaceInput,
  DeleteNamespaceOutput,
  EntityRecord,
  ExportInput,
  ExtractLinkInput,
  ForgetInput,
  ForgetOutput,
  HealthOutput,
  ImportInput,
  ImportOutput,
  ListEntitiesParams,
  ListInput,
  ListNamespacesParams,
  ListOutput,
  ListRelationsParams,
  ListTagsParams,
  MergeEntitiesInput,
  MergeEntitiesOutput,
  MergeInput,
  MergeOutput,
  MemoryRecord,
  NamespaceListOutput,
  NamespaceRecord,
  RecallInput,
  RecallOutput,
  RememberInput,
  RememberOutput,
  RelationRecord,
  StatsOutput,
  TagListOutput,
  TraverseInput,
  UpdateMemoryInput,
  UpdateNamespaceInput,
  UpsertEntityInput,
  UpsertEntityOutput,
  UpsertRelationInput,
  UpsertRelationOutput,
} from "./types.js";

export interface NovaClientOptions {
  apiKey?: string;
  timeoutMs?: number;
  namespace?: string;
  fetch?: typeof fetch;
}

// A namespace travels in the `x-namespace` HTTP header, so only characters
// that are safe inside a header value are accepted:
//   [A-Za-z0-9]    - the first character must be an ASCII letter or digit
//   [A-Za-z0-9_-]* - the remainder may be letters, digits, hyphens or
//                    underscores (anchored by ^ and $)
// This structurally rules out header injection (CR/LF/colon/space).
const NAMESPACE_RE = /^[A-Za-z0-9][A-Za-z0-9_-]*$/;

/** Trim `name`, rejecting empty or whitespace-only input. */
function ensureNonEmptyName(name: string, where: string): string {
  const cleaned = name.trim();
  if (!cleaned) {
    throw new Error(`${where}: name must not be empty`);
  }
  return cleaned;
}

/** Validate a tenant namespace name, returning it stripped. */
function validateNamespace(name: string, where: string): string {
  const cleaned = ensureNonEmptyName(name, where);
  if (!NAMESPACE_RE.test(cleaned)) {
    throw new Error(
      `${where}: '${cleaned}' contains invalid characters; a namespace must start ` +
        "with a letter or digit and may then contain only letters, digits, " +
        "hyphens and underscores",
    );
  }
  return cleaned;
}

/** Validate a request timeout in milliseconds. */
function validateTimeout(timeoutMs: number): number {
  if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
    throw new Error("timeoutMs must be a finite, non-negative number of milliseconds");
  }
  return timeoutMs;
}

export class NovaApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly traceId: string | null;

  constructor(code: string, message: string, status: number, traceId: string | null = null) {
    super(message);
    this.name = "NovaApiError";
    this.code = code;
    this.status = status;
    this.traceId = traceId;
  }
}

interface RequestOptions {
  method: string;
  path: string;
  body?: unknown;
  params?: object;
}

export class NovaClient {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly timeoutMs: number;
  private readonly namespace: string;
  private readonly fetchImpl: typeof fetch;
  private readonly options: NovaClientOptions;

  constructor(baseUrl: string, options: NovaClientOptions = {}) {
    if (!baseUrl) {
      throw new Error("baseUrl must not be empty");
    }
    const base = baseUrl.trim().replace(/\/+$/, "");
    if (!base) {
      throw new Error("baseUrl must not be empty");
    }
    this.baseUrl = base;
    this.apiKey = options.apiKey;
    // The default applies only when the caller omits the option; whatever is
    // used afterwards has been through validateTimeout, which throws on NaN,
    // Infinity and negative input.
    const timeoutMs = options.timeoutMs ?? 30000;
    this.timeoutMs = validateTimeout(timeoutMs);
    this.namespace = validateNamespace(options.namespace ?? "default", "namespace");
    this.fetchImpl = options.fetch ?? ((typeof globalThis !== "undefined" && globalThis.fetch) as typeof fetch);
    if (!this.fetchImpl) {
      throw new Error("A fetch implementation is required; Node 18+ and browsers provide one globally");
    }
    this.options = { ...options };
  }

  /** Return a new client scoped to another namespace. */
  withNamespace(name: string): NovaClient {
    // Why spreading instead of naming each option: an explicit list would
    // silently drop any option added to `NovaClientOptions` later, while the
    // stored options are a copy of exactly what the caller passed. The
    // constructor still runs every validation and returns a fresh instance,
    // so no property of this client is ever mutated.
    const namespace = validateNamespace(name, "withNamespace");
    return new NovaClient(this.baseUrl, { ...this.options, namespace });
  }

  health() {
    return this.request<HealthOutput>("GET", "/v1/health");
  }

  stats() {
    return this.request<StatsOutput>("GET", "/v1/stats");
  }

  remember(input: RememberInput) {
    return this.request<RememberOutput>("POST", "/v1/memory/remember", input);
  }

  rememberBatch(input: BatchRememberInput) {
    return this.request<BatchRememberOutput>("POST", "/v1/memory/remember-batch", input);
  }

  recall(input: RecallInput) {
    return this.request<RecallOutput>("POST", "/v1/memory/recall", input);
  }

  listMemories(input: ListInput = {}) {
    return this.request<ListOutput>("POST", "/v1/memory/list", input);
  }

  forget(input: ForgetInput) {
    return this.request<ForgetOutput>("POST", "/v1/memory/forget", input);
  }

  getMemory(uuid: string) {
    return this.request<MemoryRecord>("GET", `/v1/memory/${this.quote(uuid)}`);
  }

  updateMemory(uuid: string, input: UpdateMemoryInput) {
    return this.request<MemoryRecord>("PATCH", `/v1/memory/${this.quote(uuid)}`, input);
  }

  deleteMemory(uuid: string) {
    return this.request<ForgetOutput>("DELETE", `/v1/memory/${this.quote(uuid)}`);
  }

  exportMemories(input: ExportInput = {}) {
    return this.request<Record<string, unknown>>("POST", "/v1/memory/export", input);
  }

  importMemories(input: ImportInput) {
    return this.request<ImportOutput>("POST", "/v1/memory/import", input);
  }

  mergeMemories(input: MergeInput) {
    return this.request<MergeOutput>("POST", "/v1/memory/merge", input);
  }

  listTags(params: ListTagsParams = {}) {
    return this.request<TagListOutput>("GET", "/v1/tags", undefined, params);
  }

  renameTag(name: string, newName: string) {
    return this.request<Record<string, unknown>>("PATCH", `/v1/tags/${this.quote(name)}`, { new_name: newName });
  }

  deleteTag(name: string) {
    return this.request<Record<string, unknown>>("DELETE", `/v1/tags/${this.quote(name)}`);
  }

  upsertEntity(input: UpsertEntityInput) {
    return this.request<UpsertEntityOutput>("POST", "/v1/graph/entities", input);
  }

  listEntities(params: ListEntitiesParams = {}) {
    return this.request<EntityRecord[]>("GET", "/v1/graph/entities", undefined, params);
  }

  mergeEntities(input: MergeEntitiesInput) {
    return this.request<MergeEntitiesOutput>("POST", "/v1/graph/entities/merge", input);
  }

  upsertRelation(input: UpsertRelationInput) {
    return this.request<UpsertRelationOutput>("POST", "/v1/graph/relations", input);
  }

  listRelations(params: ListRelationsParams = {}) {
    return this.request<RelationRecord[]>("GET", "/v1/graph/relations", undefined, params);
  }

  traverse(input: TraverseInput) {
    return this.request<Array<Record<string, unknown>>>("POST", "/v1/graph/traverse", input);
  }

  extractAndLink(input: ExtractLinkInput) {
    return this.request<Record<string, unknown>>("POST", "/v1/graph/extract-and-link", input);
  }

  // ------------------------------------------------------------------
  // Namespace management (admin).
  //
  // These routes require an admin-scoped API key; a tenant-scoped key gets
  // 403. The `x-namespace` header still rides along on every call because
  // the auth middleware needs it to resolve the calling scope.
  // ------------------------------------------------------------------

  /** List tenant namespaces. */
  listNamespaces(params: ListNamespacesParams = {}) {
    return this.request<NamespaceListOutput>("GET", "/v1/namespaces", undefined, params);
  }

  /** Fetch a single namespace by name. */
  getNamespace(name: string) {
    const cleaned = ensureNonEmptyName(name, "getNamespace");
    return this.request<NamespaceRecord>("GET", `/v1/namespaces/${this.quote(cleaned)}`);
  }

  /**
   * Create a tenant namespace.
   *
   * The name is validated against the same rule `withNamespace` uses, so any
   * namespace created here can immediately be scoped to. The reserved name
   * `default` cannot be created.
   */
  createNamespace(input: CreateNamespaceInput) {
    const name = validateNamespace(input.name, "createNamespace");
    if (name.toLowerCase() === "default") {
      throw new Error("createNamespace: 'default' is a reserved name");
    }
    const body = {
      name,
      description: input.description ?? null,
      config: input.config ?? {},
    };
    return this.request<NamespaceRecord>("POST", "/v1/namespaces", body);
  }

  /**
   * Update a namespace description and/or config.
   *
   * Only the fields present in `input` are sent; the server keeps the stored
   * value for anything omitted. There is no rename operation.
   */
  updateNamespace(name: string, input: UpdateNamespaceInput = {}) {
    const cleaned = ensureNonEmptyName(name, "updateNamespace");
    return this.request<NamespaceRecord>("PATCH", `/v1/namespaces/${this.quote(cleaned)}`, input);
  }

  /**
   * Delete a namespace.
   *
   * The built-in `default` namespace is protected; the response reports
   * `protected: true` when the server declines to remove it.
   */
  deleteNamespace(name: string) {
    const cleaned = ensureNonEmptyName(name, "deleteNamespace");
    return this.request<DeleteNamespaceOutput>("DELETE", `/v1/namespaces/${this.quote(cleaned)}`);
  }

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    params?: object,
  ): Promise<T> {
    const query = params ? this.encodeParams(params) : "";
    const url = this.baseUrl + path + (query ? `?${query}` : "");

    const headers: Record<string, string> = {
      Accept: "application/json",
      "x-namespace": this.namespace,
    };
    let payload: BodyInit | undefined;
    if (body !== undefined) {
      headers["Content-Type"] = "application/json";
      payload = JSON.stringify(body);
    }
    if (this.apiKey) {
      headers["Authorization"] = `Bearer ${this.apiKey}`;
    }

    const controller = "AbortController" in globalThis ? new AbortController() : undefined;
    const timer =
      controller && this.timeoutMs > 0
        ? setTimeout(() => controller.abort(), this.timeoutMs)
        : undefined;

    let response: Response;
    try {
      response = await this.fetchImpl(url, {
        method,
        headers,
        body: payload,
        signal: controller ? controller.signal : undefined,
      });
    } catch (err) {
      if (controller && err instanceof Error && err.name === "AbortError") {
        throw new NovaApiError("timeout", `request to ${url} timed out`, 0);
      }
      throw new NovaApiError("network", `request to ${url} failed: ${this.errorMessage(err)}`, 0);
    } finally {
      if (timer) {
        clearTimeout(timer);
      }
    }

    const text = await response.text();
    let data: unknown = undefined;
    let parseFailed = false;
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        parseFailed = true;
      }
    }

    if (!response.ok || parseFailed) {
      const body: Record<string, unknown> =
        typeof data === "object" && data !== null ? (data as Record<string, unknown>) : {};
      const code = typeof body.code === "string" ? body.code : `http_${response.status}`;
      const message =
        typeof body.message === "string"
          ? body.message
          : parseFailed
            ? `invalid JSON response body: ${bodySnippet(text)}`
            : `HTTP ${response.status} returned no body`;
      const traceId = typeof body.trace_id === "string" ? body.trace_id : null;
      throw new NovaApiError(code, message, response.status, traceId);
    }

    return data as T;
  }

  private encodeParams(params: object): string {
    const parts: string[] = [];
    for (const [key, value] of Object.entries(params)) {
      if (value === undefined || value === null || value === "") {
        continue;
      }
      parts.push(`${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`);
    }
    return parts.join("&");
  }

  private quote(value: string): string {
    return encodeURIComponent(value);
  }

  private errorMessage(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }
}

function bodySnippet(text: string): string {
  const s = text.replace(/\s+/g, " ").trim();
  return s.length > 160 ? `${s.slice(0, 160)}...` : s;
}