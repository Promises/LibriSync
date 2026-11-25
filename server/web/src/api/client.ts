import type { ApiErrorCode } from './types';

export const API_BASE = '/api/v1';

/**
 * A non-2xx response. `error` is the server's machine code when it sent one;
 * `network` and `bad_response` are the client's own for failures that never got a
 * well-formed answer.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly error: ApiErrorCode | 'network' | 'bad_response' | string;
  /** Dotted path of the offending field on `400 invalid_input` ("audible.dailyLicenceCap"). */
  readonly field: string | null;

  constructor(status: number, error: string, message: string, field: string | null = null) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.error = error;
    this.field = field;
  }

  get isUnauthorized(): boolean {
    return this.status === 401;
  }
}

export function isApiError(e: unknown): e is ApiError {
  return e instanceof ApiError;
}

/** Human text for any thrown value, for toasts and inline errors. */
export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

type UnauthorizedListener = () => void;
const unauthorizedListeners = new Set<UnauthorizedListener>();

/** Called whenever an authenticated request comes back 401 (session expired, logged out elsewhere). */
export function onUnauthorized(listener: UnauthorizedListener): () => void {
  unauthorizedListeners.add(listener);
  return () => unauthorizedListeners.delete(listener);
}

type Query = Record<string, string | number | boolean | string[] | null | undefined>;

export function buildQuery(query: Query | undefined): string {
  if (!query) return '';
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null || value === '') continue;
    if (Array.isArray(value)) {
      if (value.length > 0) params.set(key, value.join(','));
    } else {
      params.set(key, String(value));
    }
  }
  const s = params.toString();
  return s ? `?${s}` : '';
}

/** Ids may contain `:` (Audible) or other reserved characters. */
export function seg(id: string | number): string {
  return encodeURIComponent(String(id));
}

type RequestOptions = {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  body?: unknown;
  query?: Query;
  signal?: AbortSignal;
};

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = 'GET', body, query, signal } = options;
  const headers: Record<string, string> = { Accept: 'application/json' };
  if (body !== undefined) headers['Content-Type'] = 'application/json';

  let res: Response;
  try {
    res = await fetch(`${API_BASE}${path}${buildQuery(query)}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: 'same-origin',
      signal,
    });
  } catch (e) {
    if (e instanceof DOMException && e.name === 'AbortError') throw e;
    throw new ApiError(0, 'network', 'Cannot reach the LibriSync server.');
  }

  if (res.status === 204 || res.status === 202) {
    // 202 bodies are not part of the contract; ignore whatever comes back.
    return undefined as T;
  }

  const text = await res.text();
  let json: unknown = undefined;
  if (text) {
    try {
      json = JSON.parse(text);
    } catch {
      if (res.ok) throw new ApiError(res.status, 'bad_response', 'The server sent a response that is not JSON.');
    }
  }

  if (!res.ok) {
    const obj = (json && typeof json === 'object' ? json : {}) as Record<string, unknown>;
    const code = typeof obj.error === 'string' ? obj.error : `http_${res.status}`;
    const message =
      typeof obj.message === 'string' && obj.message
        ? obj.message
        : `${res.status} ${res.statusText || 'Request failed'}`;
    const field = typeof obj.field === 'string' ? obj.field : null;
    const err = new ApiError(res.status, code, message, field);
    // Auth endpoints answer 401 for bad credentials; that is not a lost session.
    if (res.status === 401 && !path.startsWith('/auth/')) {
      unauthorizedListeners.forEach((l) => l());
    }
    throw err;
  }

  return json as T;
}
