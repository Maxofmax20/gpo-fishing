import React, { Suspense, lazy, useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import OntologyReviewBrowser from "./pages/OntologyReviewBrowser";
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

/**
 * Path-based routing, for the one page with a real browser URL.
 *
 * Re-read on `popstate`, so the back button and a refresh both land correctly. Any
 * route other than `/ontology-review` renders the desktop windows exactly as
 * before: this adds a branch, it does not restructure the app.
 */
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
  if (route === ROUTES.ontologyReview && viewParam === null) {
    return <OntologyReviewBrowser />;
  }

  const Window =
    view === "hud" ? Hud : view === "overlay" ? Overlay : view === "guide" ? Guide : Panel;
  return (
    <Suspense
      fallback={
        <div className="p-4 text-[12px] text-fg-mute">Loading window…</div>
      }
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