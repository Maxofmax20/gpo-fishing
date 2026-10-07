/**
 * MINIMAL PATH ROUTER
 *
 * # Why this exists rather than a router library
 *
 * The desktop app has no router. Navigation is `useState<Tab>` in `Panel.tsx`, and
 * the extra windows (HUD, overlay, guide) are selected by a `?view=` query
 * parameter in `main.tsx`. So there is no existing routing convention to extend,
 * and the ontology review page simply had no URL of its own — which is why it could
 * not be opened in a browser.
 *
 * Adding `react-router` for one route would be the wrong trade here: it would add a
 * dependency, a provider tree, and a nesting model to an app that does not want any
 * of them, to serve a single additional URL. What is actually needed is:
 *
 *   * read the current path,
 *   * render one of two roots,
 *   * navigate without a reload,
 *   * survive a browser refresh.
 *
 * That is this file, and nothing more. If the app ever grows a second browser route
 * worth nesting or parameters, that is the moment to reach for a real router - not
 * before.
 *
 * # Refresh behaviour
 *
 * Refresh is what a static host cannot fix on its own: the browser asks the server
 * for `/ontology-review`, and a server that only knows about files returns 404. The
 * serving side (`src-tauri/src/ontology_web.rs`) answers any non-asset path with
 * `index.html`, which is the SPA half of this arrangement. The two must agree: this
 * file reads the path, that file makes the path exist.
 */

/** The routes this app serves over HTTP. */
export const ROUTES = {
  /** The ontology review workstation. The one real browser route. */
  ontologyReview: "/ontology-review",
  /** The desktop panel, at the site root. */
  root: "/",
} as const;

export type Route = (typeof ROUTES)[keyof typeof ROUTES];

/**
 * Normalise a path to a known route, or null.
 *
 * Trailing slashes and a trailing `.html` both resolve to the same route, because
 * both are things a human types or a browser produces from a link. Unknown paths are
 * NOT coerced to the root: a typo should render the app shell, which then says the
 * page does not exist, rather than silently showing something else.
 */
export function resolveRoute(pathname: string): Route | null {
  let p = pathname || "/";
  // Strip a query string if one was passed in the path argument by mistake.
  const q = p.indexOf("?");
  if (q >= 0) p = p.slice(0, q);
  if (p.endsWith("/index.html")) p = "/";
  if (p.length > 1 && p.endsWith("/")) p = p.slice(0, -1);
  if (p.endsWith(".html")) p = p.slice(0, -5) || "/";

  if (p === ROUTES.ontologyReview) return ROUTES.ontologyReview;
  if (p === "/" || p === "") return ROUTES.root;
  return null;
}

/** The route for the current browser location, or null. */
export function currentRoute(): Route | null {
  if (typeof window === "undefined") return ROUTES.root;
  return resolveRoute(window.location.pathname);
}

/**
 * Navigate without a full page load.
 *
 * `pushState` alone would change the URL bar while the app kept rendering the old
 * route, so the `popstate` event is dispatched to keep any listener in step.
 *
 * Falls back to a full load when the History API is unavailable, which is what a
 * `file://` window in the packaged desktop app effectively has.
 */
export function navigate(to: string): void {
  if (typeof window === "undefined") return;
  if (window.location.pathname === to) return;
  if (!window.history?.pushState) {
    window.location.href = to;
    return;
  }
  window.history.pushState({}, "", to);
  window.dispatchEvent(new PopStateEvent("popstate"));
}

/**
 * Subscribe to route changes (back button, and `navigate`).
 *
 * Returns an unsubscribe function, so a caller cannot leak a listener.
 */
export function onRouteChange(cb: () => void): () => void {
  if (typeof window === "undefined") return () => undefined;
  window.addEventListener("popstate", cb);
  return () => window.removeEventListener("popstate", cb);
}