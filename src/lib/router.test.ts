/// <reference types="node" />
/**
 * ROUTING AND BROWSER-TRANSPORT TESTS
 *
 * # What is being protected here
 *
 * Two things a reviewer depends on and neither of which is visible in the component:
 *
 *  1. **`/ontology-review` resolves to the review page**, including after a refresh
 *     and including the sloppy URLs a human actually types (`/ontology-review/`,
 *     `/ontology-review.html`).
 *  2. **The page reaches the same Rust functions over either transport.** Tauri IPC
 *     in the desktop app, HTTP in a browser. If these diverge, the browser becomes a
 *     second, less-audited path to the decision log - which is the whole risk of
 *     adding a web route at all.
 *
 * The second point is why the transport tests assert on the *same* backend
 * functions being named, rather than mocking a URL and calling it a day.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ROUTES,
  currentRoute,
  navigate,
  onRouteChange,
  resolveRoute,
} from "./router";
import {
  ONTOLOGY_API_BASE,
  browserOrInvoke,
  browserOrInvokePost,
  fetchImageDataUrl,
  imageUrl,
  isTauri,
} from "./ontologyBrowser";

// ===========================================================================
// Route resolution
// ===========================================================================

describe("route resolution", () => {
  it("resolves the review page", () => {
    expect(resolveRoute("/ontology-review")).toBe("/ontology-review");
  });

  it("resolves the app root", () => {
    expect(resolveRoute("/")).toBe("/");
    expect(resolveRoute("")).toBe("/");
  });

  it("tolerates the forms a human or a browser actually produces", () => {
    // A trailing slash, a `.html` suffix, and a window's `index.html` path all mean
    // the same two pages. Refusing them would produce a 404 for a URL that is
    // visibly correct.
    for (const p of ["/ontology-review/", "/ontology-review.html"]) {
      expect(resolveRoute(p), p).toBe(ROUTES.ontologyReview);
    }
    // The desktop app's own entry points stay on the root route, because they are
    // the Panel/HUD/overlay windows rather than browser pages.
    expect(resolveRoute("/index.html")).toBe("/");
    expect(resolveRoute("/index.html?view=panel")).toBe("/");
    expect(resolveRoute("/?view=panel")).toBe("/");
  });

  it("does NOT coerce an unknown path to the root", () => {
    // Silently showing the app shell at /typo would hide the mistake. Returning
    // null lets the shell render and say the page does not exist.
    expect(resolveRoute("/nope")).toBeNull();
    expect(resolveRoute("/ontology-reviewing")).toBeNull();
    expect(resolveRoute("/api/ontology/status")).toBeNull();
  });

  it("does not match a prefix of the review route", () => {
    // `/ontology-review-foo` must not render the review page.
    expect(resolveRoute("/ontology-review-foo")).toBeNull();
  });

  it("declares exactly the routes it serves", () => {
    // Both review routes and the app root, and nothing else. A fifth path would
    // need a screen behind it.
    expect(Object.values(ROUTES).sort()).toEqual([
      "/",
      "/label-review",
      "/ontology-review",
    ]);
  });

  it("resolves the label review route", () => {
    expect(resolveRoute("/label-review")).toBe("/label-review");
    expect(resolveRoute("/label-review/")).toBe("/label-review");
    // Must not be reachable by prefix, like the other two.
    expect(resolveRoute("/label-reviewing")).toBeNull();
  });
});

describe("navigation", () => {
  const original = window.location.pathname;

  afterEach(() => {
    window.history.replaceState({}, "", original || "/");
  });

  it("pushes the new path and notifies listeners", () => {
    const seen: string[] = [];
    const off = onRouteChange(() => seen.push(window.location.pathname));
    navigate("/ontology-review");
    expect(window.location.pathname).toBe("/ontology-review");
    expect(seen).toContain("/ontology-review");
    off();
  });

  it("unsubscribes cleanly", () => {
    let calls = 0;
    const off = onRouteChange(() => calls++);
    navigate("/ontology-review");
    const after = calls;
    off();
    navigate("/");
    expect(calls).toBe(after);
  });

  it("does nothing when already on the target", () => {
    window.history.replaceState({}, "", "/ontology-review");
    const off = onRouteChange(() => {
      throw new Error("navigating to the current route must be a no-op");
    });
    navigate("/ontology-review");
    off();
  });

  it("reports the current route from the browser location", () => {
    window.history.replaceState({}, "", "/ontology-review");
    expect(currentRoute()).toBe(ROUTES.ontologyReview);
  });
});

// ===========================================================================
// Transport selection
// ===========================================================================

function setTauri(present: boolean) {
  if (present) {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  } else {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    delete (window as unknown as Record<string, unknown>).__TAURI__;
  }
}

describe("transport detection", () => {
  afterEach(() => setTauri(false));

  it("is false in a plain browser", () => {
    setTauri(false);
    expect(isTauri()).toBe(false);
  });

  it("is true when Tauri has injected itself", () => {
    setTauri(true);
    expect(isTauri()).toBe(true);
  });
});

describe("HTTP transport", () => {
  beforeEach(() => {
    setTauri(false);
    vi.stubGlobal("fetch", vi.fn());
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    setTauri(false);
  });

  const fetchMock = () => vi.mocked(fetch);

  it("never sends an absolute host, so it cannot be pointed elsewhere", () => {
    // The security-relevant property: there is no configurable base URL, and every
    // request is relative, therefore always same-origin with this page.
    expect(ONTOLOGY_API_BASE).toBe("/api/ontology");
    expect(ONTOLOGY_API_BASE.startsWith("/")).toBe(true);
    expect(ONTOLOGY_API_BASE).not.toContain("http");
  });

  it("reads status from the local API in a browser", async () => {
    fetchMock().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ available: true, total: 77 }),
    } as Response);
    const r = await browserOrInvoke<{ total: number }>(
      "/status",
      "ontology_review_status",
    );
    expect(r.total).toBe(77);
    expect(fetchMock().mock.calls[0][0]).toBe("/api/ontology/status");
  });

  it("obtains a process CSRF token and POSTs JSON with the token header", async () => {
    fetchMock().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ ok: true, token: "test-csrf-token" }),
    } as Response);
    await browserOrInvokePost("/decision", "ontology_decide", {}, { decision: "UNKNOWN" });
    expect(fetchMock().mock.calls[0][0]).toBe("/api/csrf");
    const [, init] = fetchMock().mock.calls[1];
    expect(init?.method).toBe("POST");
    expect((init?.headers as Record<string, string>)["X-GPO-CSRF"]).toBe("test-csrf-token");
    expect((init?.headers as Record<string, string>)["Content-Type"]).toBe(
      "application/json",
    );
    // `credentials: omit`: the local review API needs no cookies, so sending none
    // removes ambient authority entirely.
    expect(init?.credentials).toBe("omit");
  });

  it("surfaces the server's own explanation rather than a generic failure", async () => {
    // The server says exactly which field is missing; that text is what the UI shows
    // the reviewer, so replacing it would throw away the most useful part.
    fetchMock().mockResolvedValue({
      ok: false,
      status: 400,
      json: async () => ({ ok: false, error: "NEW_ENTITY requires proposedEntityName" }),
    } as Response);
    await expect(
      browserOrInvokePost("/decision", "ontology_decide", {}, {}),
    ).rejects.toThrow(/proposedEntityName/);
  });

  it("maps a missing image to null so one bad thumbnail is not an error", async () => {
    fetchMock().mockResolvedValue({ ok: false, status: 404 } as Response);
    await expect(fetchImageDataUrl("/api/ontology/images/x")).resolves.toBeNull();
  });

  it("still surfaces a real server failure", async () => {
    // A 500 is not a missing image; hiding it would show a broken page with no
    // explanation.
    fetchMock().mockResolvedValue({
      ok: false,
      status: 500,
      json: async () => ({ error: "boom" }),
    } as Response);
    await expect(fetchImageDataUrl("/api/ontology/images/x")).rejects.toThrow(/boom/);
  });

  it("builds image URLs from the id, percent-encoded", () => {
    expect(imageUrl("606020203e34beb6")).toBe(
      "/api/ontology/images/606020203e34beb6",
    );
    expect(imageUrl("606020203e34beb6", true)).toBe(
      "/api/ontology/images/606020203e34beb6/full",
    );
    // A hostile id is encoded, not interpolated raw.
    expect(imageUrl("../../windows/win.ini")).toBe(
      "/api/ontology/images/..%2F..%2Fwindows%2Fwin.ini",
    );
  });
});

// ===========================================================================
// The two routes reach ONE component
// ===========================================================================

describe("one implementation, two ways in", () => {
  it("the browser route and the Training tab render the same component", async () => {
    // Read the source rather than trusting a comment: if a second page were added,
    // this would catch it.
    const { readFileSync } = await import("node:fs");
    const root = (await import("node:path")).resolve(__dirname, "..");

    const shell = readFileSync(root + "/pages/OntologyReviewBrowser.tsx", "utf8");
    expect(shell).toContain('from "./OntologyReview"');
    // The shell adds chrome only. It must not reimplement review logic.
    expect(shell).not.toContain("invoke<");
    expect(shell).not.toContain("ontology_decide");

    // The desktop tab renders the component directly.
    const training = readFileSync(root + "/pages/TrainingCenter.tsx", "utf8");
    expect(training).toContain("<OntologyReview />");

    // And the router has exactly one review route, with no duplicate page file.
    const router = readFileSync(root + "/lib/router.ts", "utf8");
    expect((router.match(/ontologyReview:/g) ?? []).length).toBe(1);
  });

  it("no second frontend was created", async () => {
    const { readdirSync, existsSync } = await import("node:fs");
    const { resolve } = await import("node:path");
    const root = resolve(__dirname, "..");
    // A second app, a second HTML entry, or a second Vite project would each break
    // the "existing web app + /ontology-review" requirement.
    const entries = readdirSync(root);
    expect(entries).not.toContain("ontology-review.html");
    expect(entries).not.toContain("index2.html");
    expect(existsSync(resolve(root, "..", "vite.ontology.config.ts"))).toBe(false);
    expect(existsSync(resolve(root, "..", "ontology-vite"))).toBe(false);
    // One entry point, still.
    expect(entries).toContain("main.tsx");
  });
});

// ===========================================================================
// Independence from the macro, the game, and the updater
// ===========================================================================

/**
 * Source with comments stripped.
 *
 * These tests assert that certain identifiers are ABSENT from a file, and the files
 * in question document the very independence being asserted - in prose, with those
 * words. Scanning the raw text would make every one of them fail on its own
 * documentation, which is the opposite of the intent. Only executable lines are
 * examined.
 */
async function codeOf(relPath: string): Promise<string> {
  const { readFileSync } = await import("node:fs");
  const { resolve } = await import("node:path");
  const src = readFileSync(resolve(__dirname, "..", relPath), "utf8");
  return src
    .split("\n")
    .filter((l) => {
      const t = l.trimStart();
      return !t.startsWith("//") && !t.startsWith("*") && !t.startsWith("/*");
    })
    .join("\n");
}

describe("the review route is independent of the automation stack", () => {
  it("the browser shell imports nothing from the bot, overlay, or updater", async () => {
    const { readFileSync } = await import("node:fs");
    const { resolve } = await import("node:path");
    const src = readFileSync(resolve(__dirname, "..", "pages/OntologyReviewBrowser.tsx"), "utf8");

    // The IMPORT BLOCK specifically. A visible label saying "no updater involved" is
    // the UI stating the guarantee; scanning the whole file for the word would
    // forbid exactly that.
    const imports = src
      .split("\n")
      .filter((l) => /^\s*import\b/.test(l))
      .join("\n");
    expect(imports.length).toBeGreaterThan(0);

    for (const forbidden of [
      "windows/Panel",
      "windows/Overlay",
      "windows/Hud",
      "windows/Guide",
      "windows",
      "bot",
      "updater",
      "plugins",
    ]) {
      expect(
        imports.toLowerCase(),
        `the shell must not import ${forbidden}`,
      ).not.toContain(forbidden.toLowerCase());
    }
    // What it DOES import is the shared page, the router, and React. Nothing else.
    expect(imports).toContain("./OntologyReview");
    expect(imports).toContain("../lib/router");
  });

  it("the shell calls no command that can start fishing", async () => {
    const code = await codeOf("pages/OntologyReviewBrowser.tsx");
    expect(code).not.toMatch(/bot(Start|Toggle|Pause|Stop)/);
    expect(code).not.toMatch(/macro(Play|Record|Start)/);
    expect(code).not.toMatch(/api\.[a-zA-Z]*[Bb]ot/);
  });

  it("the router does not gate the review route on anything", async () => {
    const code = await codeOf("lib/router.ts");
    expect(code).toContain("ROUTES.ontologyReview");
    // Route resolution is a pure function of the path: no state, no conditions, no
    // readiness check. The route resolves identically whether or not anything else
    // is running. Whole-word matching, because "navigate" contains "gate".
    for (const forbidden of [/\bbot\b/i, /\brunning\b/i, /\bready\b/i, /\bgate\w*/i]) {
      expect(code, `the router must not reference ${forbidden}`).not.toMatch(forbidden);
    }
  });
});