/**
 * ONTOLOGY REVIEW — BROWSER ENTRY POINT
 *
 * # What this is
 *
 * The chrome-free shell for `/ontology-review` when the page is opened in a normal
 * browser. It renders the SAME `OntologyReview` component the desktop app renders —
 * there is exactly one implementation of the page, and this file adds no review
 * logic of its own.
 *
 * # Why a shell is needed at all
 *
 * `Panel.tsx` is a desktop window: it starts the bot connection, calls
 * `api.panelShow()`, checks for Roblox, and gates content behind a setup gate. None
 * of that belongs on a review page. If the review route rendered `Panel`, opening it
 * in a browser would either fail at the first IPC call or drag the entire
 * application lifecycle into a research tool. The requirement is that opening this
 * page starts no fishing, needs no game window and needs no updater; rendering
 * `Panel` would make all three true.
 *
 * So this shell provides only what a standalone page needs: a heading, the live
 * URL (so a reviewer can confirm which workstation they are on), and a link back to
 * the desktop app. Everything else is the shared component.
 */

import { useCallback, useEffect, useState } from "react";
import { OntologyReview } from "./OntologyReview";
import { navigate, ROUTES } from "../lib/router";

/** The API status the desktop app reports, used only to show the address.
 *
 * Deliberately the ONLY thing fetched here that could fail: if the review server is
 * not running, the page still renders and says so, rather than showing an error
 * instead of the reviewer's work.
 */
function useServerReachable() {
  const [state, setState] = useState<"checking" | "up" | "down">("checking");
  useEffect(() => {
    let live = true;
    const probe = () => {
      fetch("/api/ontology/status", { method: "GET", credentials: "omit" })
        .then((r) => {
          if (live) setState(r.ok ? "up" : "down");
        })
        .catch(() => {
          if (live) setState("down");
        });
    };
    probe();
    // Re-probe periodically: a reviewer may leave this tab open while the app
    // restarts, and a stale "unreachable" banner would be wrong.
    const t = window.setInterval(probe, 15_000);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, []);
  return state;
}

export function OntologyReviewBrowser() {
  const reachable = useServerReachable();
  const goHome = useCallback(() => navigate(ROUTES.root), []);

  // Number keys and Enter are the review shortcuts; the browser's own shortcuts
  // (Ctrl+T, Ctrl+W) must keep working, so Backspace is bound to go back rather than
  // repurposed.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") goHome();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [goHome]);

  return (
    <div className="min-h-screen bg-bg text-fg">
      <header className="border-b border-line/70 px-5 py-2.5 flex items-center gap-3 flex-wrap">
        <div className="flex items-baseline gap-2.5">
          <span className="text-[15px] font-semibold">Ontology Review Workstation</span>
          <span className="text-[10px] text-fg-mute">
            standalone &middot; no game, macro, or updater involved
          </span>
        </div>
        <span className="flex-1" />
        <code
          className="text-[10px] font-mono text-fg-dim border border-line/70 rounded px-1.5 py-0.5"
          title="This page's address"
        >
          {typeof window === "undefined" ? "/ontology-review" : window.location.pathname}
        </code>
        <span
          className={`text-[10px] ${
            reachable === "up"
              ? "text-ok"
              : reachable === "down"
                ? "text-bad"
                : "text-fg-mute"
          }`}
        >
          {reachable === "up"
            ? "review server connected"
            : reachable === "down"
              ? "review server not reachable"
              : "checking…"}
        </span>
        <button
          type="button"
          onClick={goHome}
          className="text-[11px] text-fg-dim hover:text-fg rounded px-1.5 py-0.5 hover:bg-white/5 focus:outline-none focus:ring-1 focus:ring-accent"
          title="Back to the app (Esc)"
        >
          Back to app
        </button>
      </header>

      <main className="px-5 py-4">
        <OntologyReview />
      </main>

      <footer className="px-5 py-3 text-[10px] text-fg-mute border-t border-line/70">
        Recording a decision appends a <span className="font-mono">human proposal</span> to{" "}
        <span className="font-mono">ontology_decisions.jsonl</span>. It does not modify the knowledge
        base, the vocabulary file, any dataset label, any split, or any model. Applying it is a
        separate, deliberate step performed by a person.
      </footer>
    </div>
  );
}

export default OntologyReviewBrowser;