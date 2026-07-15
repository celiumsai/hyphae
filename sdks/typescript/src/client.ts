// SPDX-License-Identifier: Apache-2.0

import type {
  CapabilitiesV1,
  CommitReceiptV1,
  DeleteRequestV1,
  ErrorV1,
  GetRequestV1,
  GetResponseV1,
  HealthV1,
  ProofV1,
  PutRequestV1,
  QueryRequestV1,
  QueryResponseV1,
} from "./generated.js";
import { parseHyphaeJson, stringifyHyphaeJson } from "./json.js";

const DEFAULT_RESPONSE_BYTES = 32 * 1024 * 1024;
const DEFAULT_WITNESS_BYTES = 512 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS = 60_000;

export interface HyphaeClientOptions {
  readonly bearerToken?: string;
  readonly timeoutMs?: number;
  readonly responseBytes?: number;
  readonly witnessBytes?: number;
  readonly fetch?: typeof globalThis.fetch;
}

export interface ApiResponse<T> {
  readonly value: T;
  readonly requestId: string;
}

export class HyphaeApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly requestId: string;

  constructor(status: number, envelope: ErrorV1) {
    super(`Hyphae API returned HTTP ${status} ${envelope.code} (request ${envelope.request_id})`);
    this.name = "HyphaeApiError";
    this.status = status;
    this.code = envelope.code;
    this.requestId = envelope.request_id;
  }
}

export class HyphaeClientError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "HyphaeClientError";
  }
}

export class HyphaeClient {
  readonly #origin: URL;
  readonly #bearerToken: string | undefined;
  readonly #timeoutMs: number;
  readonly #responseBytes: number;
  readonly #witnessBytes: number;
  readonly #fetch: typeof globalThis.fetch;

  constructor(baseUrl: string, options: HyphaeClientOptions = {}) {
    let origin: URL;
    try {
      origin = new URL(baseUrl);
    } catch (cause) {
      throw new HyphaeClientError("invalid Hyphae base URL", { cause });
    }
    if ((origin.protocol !== "http:" && origin.protocol !== "https:") ||
        origin.username !== "" || origin.password !== "" || origin.search !== "" ||
        origin.hash !== "" || (origin.pathname !== "" && origin.pathname !== "/")) {
      throw new HyphaeClientError("Hyphae base URL must be a root HTTP(S) origin");
    }
    const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    const responseBytes = options.responseBytes ?? DEFAULT_RESPONSE_BYTES;
    const witnessBytes = options.witnessBytes ?? DEFAULT_WITNESS_BYTES;
    if (![timeoutMs, responseBytes, witnessBytes].every(Number.isSafeInteger) ||
        timeoutMs <= 0 || responseBytes <= 0 || witnessBytes <= 0) {
      throw new HyphaeClientError("client timeout and response limits must be positive safe integers");
    }
    if (options.bearerToken !== undefined &&
        (options.bearerToken.length === 0 || /[\r\n]/u.test(options.bearerToken))) {
      throw new HyphaeClientError("invalid bearer token for an HTTP authorization header");
    }
    const fetchFunction = options.fetch ?? globalThis.fetch;
    if (typeof fetchFunction !== "function") {
      throw new HyphaeClientError("this runtime does not provide fetch");
    }
    origin.pathname = "/";
    this.#origin = origin;
    this.#bearerToken = options.bearerToken;
    this.#timeoutMs = timeoutMs;
    this.#responseBytes = responseBytes;
    this.#witnessBytes = witnessBytes;
    this.#fetch = fetchFunction;
  }

  capabilities(): Promise<ApiResponse<CapabilitiesV1>> {
    return this.#json("v1/capabilities", false);
  }

  liveness(): Promise<ApiResponse<HealthV1>> {
    return this.#json("v1/health/live", false);
  }

  readiness(): Promise<ApiResponse<HealthV1>> {
    return this.#json("v1/health/ready", false);
  }

  put(request: PutRequestV1): Promise<ApiResponse<CommitReceiptV1>> {
    return this.#json("v1/kv/put", true, request);
  }

  delete(request: DeleteRequestV1): Promise<ApiResponse<CommitReceiptV1>> {
    return this.#json("v1/kv/delete", true, request);
  }

  get(request: GetRequestV1): Promise<ApiResponse<GetResponseV1>> {
    return this.#json("v1/kv/get", true, request);
  }

  query(request: QueryRequestV1): Promise<ApiResponse<QueryResponseV1>> {
    return this.#json("v1/query", true, request);
  }

  async downloadWitness(proof: ProofV1): Promise<ApiResponse<Uint8Array>> {
    const expectedPath = `/v1/witnesses/${proof.checkpoint_sequence}/${proof.snapshot_digest}`;
    if (proof.witness.path !== expectedPath) {
      throw new HyphaeClientError("proof contains a noncanonical witness reference");
    }
    const expectedBytes = typeof proof.witness.file_bytes === "bigint"
      ? proof.witness.file_bytes
      : BigInt(proof.witness.file_bytes);
    if (expectedBytes < 0n || expectedBytes > BigInt(this.#witnessBytes)) {
      throw new HyphaeClientError(
        `Hyphae response exceeded local limit ${this.#witnessBytes} bytes`,
      );
    }
    const response = await this.#request(expectedPath.slice(1), true);
    if (!response.ok) {
      throw await this.#apiError(response);
    }
    if (response.status !== 200) {
      throw new HyphaeClientError(`Hyphae returned unexpected success status ${response.status}`);
    }
    const requestId = singleHeader(response.headers, "x-request-id");
    if (requestId === undefined) {
      throw new HyphaeClientError("Hyphae response has no single valid X-Request-Id header");
    }
    if (singleHeader(response.headers, "digest") !== `blake3=${proof.snapshot_digest}`) {
      throw new HyphaeClientError("downloaded witness digest header differs from the proof");
    }
    const value = await readBounded(response, this.#witnessBytes);
    if (BigInt(value.byteLength) !== expectedBytes) {
      throw new HyphaeClientError("downloaded witness length differs from the proof");
    }
    return { value, requestId };
  }

  async #json<T>(path: string, authenticated: boolean, body?: unknown): Promise<ApiResponse<T>> {
    const response = await this.#request(path, authenticated, body);
    if (!response.ok) {
      throw await this.#apiError(response);
    }
    if (response.status !== 200) {
      throw new HyphaeClientError(`Hyphae returned unexpected success status ${response.status}`);
    }
    requireJson(response.headers);
    const requestId = singleHeader(response.headers, "x-request-id");
    if (requestId === undefined) {
      throw new HyphaeClientError("Hyphae response has no single valid X-Request-Id header");
    }
    const encoded = await readBounded(response, this.#responseBytes);
    try {
      return { value: parseHyphaeJson(new TextDecoder("utf-8", { fatal: true }).decode(encoded)) as T, requestId };
    } catch (cause) {
      throw new HyphaeClientError("Hyphae response violated the version 1 JSON contract", { cause });
    }
  }

  async #apiError(response: Response): Promise<HyphaeApiError> {
    requireJson(response.headers);
    const requestId = singleHeader(response.headers, "x-request-id");
    if (requestId === undefined) {
      throw new HyphaeClientError("Hyphae response has no single valid X-Request-Id header");
    }
    const encoded = await readBounded(response, this.#responseBytes);
    let envelope: ErrorV1;
    try {
      envelope = parseHyphaeJson(new TextDecoder("utf-8", { fatal: true }).decode(encoded)) as ErrorV1;
    } catch (cause) {
      throw new HyphaeClientError("Hyphae error response violated the version 1 JSON contract", { cause });
    }
    if (typeof envelope !== "object" || envelope === null ||
        typeof envelope.code !== "string" || typeof envelope.message !== "string" ||
        typeof envelope.request_id !== "string") {
      throw new HyphaeClientError("Hyphae error response violated the version 1 JSON contract");
    }
    if (envelope.request_id !== requestId) {
      throw new HyphaeClientError("Hyphae error envelope request ID differs from its response header");
    }
    return new HyphaeApiError(response.status, envelope);
  }

  async #request(path: string, authenticated: boolean, body?: unknown): Promise<Response> {
    const headers = new Headers();
    if (authenticated && this.#bearerToken !== undefined) {
      headers.set("authorization", `Bearer ${this.#bearerToken}`);
    }
    let method = "GET";
    let encoded: string | undefined;
    if (body !== undefined) {
      method = "POST";
      headers.set("content-type", "application/json");
      encoded = stringifyHyphaeJson(body);
    }
    const endpoint = new URL(path, this.#origin);
    try {
      return await this.#fetch(endpoint, {
        method,
        headers,
        ...(encoded === undefined ? {} : { body: encoded }),
        signal: AbortSignal.timeout(this.#timeoutMs),
      });
    } catch (cause) {
      throw new HyphaeClientError("Hyphae HTTP transport failed", { cause });
    }
  }
}

function requireJson(headers: Headers): void {
  const contentType = singleHeader(headers, "content-type");
  const mediaType = contentType?.split(";", 1)[0]?.trim().toLowerCase();
  if (mediaType !== "application/json" &&
      !(mediaType?.startsWith("application/") === true && mediaType.endsWith("+json"))) {
    throw new HyphaeClientError("Hyphae response did not use a JSON content type");
  }
}

function singleHeader(headers: Headers, name: string): string | undefined {
  const value = headers.get(name);
  return value === null || value.length === 0 || value.includes(",") ? undefined : value;
}

async function readBounded(response: Response, maximum: number): Promise<Uint8Array> {
  const declared = response.headers.get("content-length");
  if (declared !== null && (!/^\d+$/u.test(declared) || Number(declared) > maximum)) {
    throw new HyphaeClientError(`Hyphae response exceeded local limit ${maximum} bytes`);
  }
  if (response.body === null) {
    return new Uint8Array();
  }
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let length = 0;
  for (;;) {
    const result = await reader.read();
    if (result.done) break;
    length += result.value.byteLength;
    if (length > maximum) {
      await reader.cancel();
      throw new HyphaeClientError(`Hyphae response exceeded local limit ${maximum} bytes`);
    }
    chunks.push(result.value);
  }
  const joined = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    joined.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return joined;
}
