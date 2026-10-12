/**
 * BROWSER TRANSPORT FOR ONTOLOGY REVIEW
 *
 * # The problem this solves
 *
 * `OntologyReview.tsx` calls `api.ontologyReviewStatus()` and friends. Those go
 * through Tauri IPC, which only exists inside the desktop app. Opened in a plain
 * browser, `invoke` is undefined and the page is dead on arrival.
 *
 * # The approach
 *
 * The page does not change. The *transport under it* changes: Tauri IPC inside the
 * desktop app, `fetch` in a browser. Both land on the same Rust functions
 * (`record_human_decision`, `validate_shape`, `thumbnail_bytes`), so there is one
 * implementation of the decision rules and one place a decision can be written.
 *
 * That is the whole reason this is a thin adapter rather than a second data layer.
 * A browser-only API would have had to re-implement validation, and the copy that
 * drifts would be the one reachable without the desktop app.
 *
 * # Same-origin only
 *
 * The browser talks to the relative path, never to an absolute host. There is no
 * configurable base URL, so this file cannot be pointed at a remote server, and no
 * CORS request is ever made. The server it talks to is bound to 127.0.0.1.
 */

/**
 * Path prefix for the local review API.
 *
 * Relative, deliberately. A constant that could be edited to point at another host
 * would turn a local-only review tool into an exfiltration target for human
 * decisions, so there is no knob here.
 */
import { invoke } from "@tauri-apps/api/core";

export const ONTOLOGY_API_BASE = "/api/ontology";

/**
 * Fish label review API.
 *
 * Its own constant rather than a `/api/ontology/../labels/` trick: the `..` form
 * has to be normalised by the browser and by the server's own path matcher, and
 * getting that wrong is a silently empty image grid rather than an error.
 */
export const LABELS_API_BASE = "/api/labels";

/**
 * True when running inside the Tauri shell.
 *
 * Checks for the internal object Tauri injects, rather than a user agent or a
 * build flag: it is the actual capability being asked about. If it is absent, IPC
 * cannot work, so the HTTP path is the only option.
 */
export function isTauri(): boolean {
  if (typeof window === "undefined") return false;
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown; __TAURI__?: unknown };
  return Boolean(w.__TAURI_INTERNALS__ || w.__TAURI__);
}

/** The URL of an ontology evidence image, for use as an `<img src>`. */
export function imageUrl(imageId: string, full = false): string {
  const base = `${ONTOLOGY_API_BASE}/images/${encodeURIComponent(imageId)}`;
  return full ? `${base}/full` : base;
}

class HttpError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
}

async function getJson<T>(url: string): Promise<T> {
  const res = await fetch(url, {
    method: "GET",
    headers: { Accept: "application/json" },
    // Same-origin by construction: no absolute host appears in `url`.
    credentials: "omit",
  });
  if (!res.ok) throw new HttpError(res.status, await describe(res));
  return (await res.json()) as T;
}

async function postJson<T>(url: string, body: unknown): Promise<T> {
  const { token } = await getJson<{ token: string }>("/api/csrf");
  const res = await fetch(url, {
    method: "POST",
    // Server checks the token, local Host, Origin and JSON before any write.
    headers: { "Content-Type": "application/json", Accept: "application/json", "X-GPO-CSRF": token },
    body: JSON.stringify(body),
    credentials: "omit",
  });
  if (!res.ok) throw new HttpError(res.status, await describe(res));
  return (await res.json()) as T;
}

/** Turn an error response into the server's own message.
 *
 * The server explains exactly which field is missing ("NEW_ENTITY requires
 * proposedEntityName"), and that text is what the UI shows. Replacing it with a
 * generic "request failed" would throw away the most useful part of the response.
 */
async function describe(res: Response): Promise<string> {
  try {
    const j = (await res.json()) as { error?: string };
    if (typeof j.error === "string" && j.error) return j.error;
  } catch {
    /* not JSON; fall through */
  }
  return `HTTP ${res.status}`;
}

/**
 * Read through HTTP when in a browser, through Tauri IPC otherwise.
 *
 * `args` is passed only to the IPC path; the HTTP path encodes everything in `path`.
 */
export function browserOrInvoke<T>(
  path: string,
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isTauri()) return invoke<T>(command, args);
  return getJson<T>(`${ONTOLOGY_API_BASE}${path}`);
}

/** The write equivalent. POST always, so a GET can never cause a write. */
export function browserOrInvokePost<T>(
  path: string,
  command: string,
  args: Record<string, unknown>,
  body: unknown,
): Promise<T> {
  if (isTauri()) return invoke<T>(command, args);
  return postJson<T>(`${ONTOLOGY_API_BASE}${path}`, body);
}

/**
 * Read an image through HTTP in a browser, through Tauri IPC otherwise.
 *
 * Separate from `browserOrInvoke` because an image is not JSON: the HTTP branch
 * fetches bytes and base64s them, while the IPC branch receives a data URL already
 * encoded by the Rust side. Both branches return the same type so callers do not
 * branch on transport.
 */
export function browserOrInvokeImage(
  url: string,
  command: string,
  args: Record<string, unknown>,
): Promise<string | null> {
  if (isTauri()) return invoke<string | null>(command, args);
  return fetchImageDataUrl(url);
}

/**
 * Fetch an evidence image and return it as a data URL.
 *
 * Matches the shape the Tauri command returns, so `Thumb` and `ImageViewer` do not
 * know or care which transport produced the image.
 *
 * A 404 resolves to `null` rather than rejecting: one unavailable thumbnail is not
 * an error worth interrupting a review for, and the components already render a
 * "thumbnail unavailable" placeholder for it. A genuine network failure still
 * rejects, so the failure is not silently hidden.
 */
export async function fetchImageDataUrl(url: string): Promise<string | null> {
  const res = await fetch(url, { method: "GET", credentials: "omit" });
  if (res.status === 404) return null;
  if (!res.ok) throw new HttpError(res.status, await describe(res));
  const blob = await res.blob();
  return await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("could not read image"));
    reader.readAsDataURL(blob);
  });
}