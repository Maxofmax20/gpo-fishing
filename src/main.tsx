import React, { Suspense, lazy, useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import OntologyReviewBrowser from "./pages/OntologyReviewBrowser";
import LabelReview from "./pages/LabelReview";
import { ROUTES, resolveRoute } from "./lib/router";

/**
 * # Why the desktop windows are lazy
 *
 * `Panel.tsx` statically imports `@tauri-apps/plugin-updater`, and `Settings.tsx`
 * does too. Those modules throw when evaluated outside the Tauri shell - they reach
 * for `window.__TAURI_INTERNALS__` at import time and dereference it.
 *
 * With a static import graph, merely having `Panel` in this file meant a plain
 * browser loading `/ontology-review` evaluated the updater plugin and crashed with
 * `Cannot read properties of undefined (reading 'transformCallback')` - a blank
 * page. That was a real failure, caught by loading the route in an actual browser
 * rather than by reading the source.
 *
 * Dynamic import fixes the cause rather than the symptom: the desktop windows, and
 * therefore the Tauri plugins, are now fetched ONLY when a route actually needs
 * them. A browser on `/ontology-review` loads none of that code, so it cannot break
 * from it.
 *
 * `OntologyReviewBrowser` stays eager: it is the browser path, and it imports no
 * Tauri plugin. The desktop windows are also eager-loaded in Tauri, where the
 * plugins are present, so the shipped app behaves exactly as before.
 */
const Hud = lazy(() => import("./windows/Hud"));
const Panel = lazy(() => import("./windows/Panel"));
const Overlay = lazy(() => import("./windows/Overlay"));
const Guide = lazy(() => import("./windows/Guide"));

/**
 * Which extra window to render, from `?view=`.
 *
 * Unchanged, and deliberately still a query parameter: the HUD, overlay and guide
 * are separate OS windows addressed by the Tauri config, not browser pages. Making
 * them paths would collide on one route.
 *
 * The RAW parameter is kept alongside the defaulted value. An earlier version
 * compared against the defaulted `view`, which is `"panel"` whenever no parameter is
 * present - so the browser route could never win, and `/ontology-review` rendered
 * the desktop shell instead. This is exactly the kind of bug that only a real
 * browser load reveals.
 */
const viewParam = new URLSearchParams(window.location.search).get("view");
const view = viewParam ?? "panel";
document.documentElement.dataset.view = view;

/**
 * Path-based routing, for the one page with a real browser URL.
 *
 * Re-read on `popstate`, so the back button and a refresh both land correctly. Any
 * route other than `/ontology-review` renders the desktop windows exactly as
 * before: this adds a branch, it does not restructure the app.
 */
/**
 * Minimal chrome for a browser route.
 *
 * Deliberately not `Panel`: that starts the bot connection, calls
 * `api.panelShow()` and gates content behind a setup check. A review page must
 * drag none of that in.
 */
function BrowserShell({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="h-screen bg-bg text-fg flex flex-col overflow-hidden">
      <header className="shrink-0 border-b border-line/70 px-5 py-2.5 flex items-center gap-3">
        <span className="text-[15px] font-semibold">{title}</span>
        <span className="text-[10px] text-fg-mute">
          standalone · no game, macro, or updater involved
        </span>
        <span className="flex-1" />
        <code className="text-[10px] font-mono text-fg-dim border border-line/70 rounded px-1.5 py-0.5">
          {window.location.pathname}
        </code>
      </header>
      {/* The app stylesheet pins the document's overflow to `hidden` because the
          desktop panel is a fixed-size window. A browser page has to scroll, so the
          scrolling container is this element: a child with `overflow-y: auto`
          scrolls even when the document cannot. */}
      <main className="flex-1 min-h-0 overflow-y-auto px-5 py-4">{children}</main>
    </div>
  );
}

function Root() {
  const [route, setRoute] = useState(() => resolveRoute(window.location.pathname));

  useEffect(() => {
    const onPop = () => setRoute(resolveRoute(window.location.pathname));
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  // The browser route wins over `?view=`: an explicit path is more specific than a
  // query parameter, and this page is only ever reached deliberately. Tested against
  // the RAW parameter, so the `panel` default does not mask the route.
  if (viewParam === null) {
    if (route === ROUTES.ontologyReview) return <OntologyReviewBrowser />;
    if (route === ROUTES.labelReview)
      return (
        <BrowserShell title="Fish Label Review">
          <LabelReview />
        </BrowserShell>
      );
  }

  const Window =
    view === "hud" ? Hud : view === "overlay" ? Overlay : view === "guide" ? Guide : Panel;
  return (
    <Suspense
      fallback={view === "panel" || view === "guide" ? (
        <div className="h-full w-full bg-bg-glass grid place-items-center text-[12px] text-fg-mute">Starting…</div>
      ) : null}
    >
      <Window />
    </Suspense>
  );
}

document.addEventListener("contextmenu", (e) => e.preventDefault());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
