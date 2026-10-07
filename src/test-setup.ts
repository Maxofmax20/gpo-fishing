/**
 * Vitest setup.
 *
 * jsdom does not implement these, and the review screen uses them. Without them a
 * test failure would be about `scrollIntoView` rather than about the review flow,
 * which is a bad trade: a noisy suite trains you to ignore it.
 */
import "@testing-library/jest-dom/vitest";

if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => undefined;
}

if (typeof window !== "undefined" && !window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => undefined,
    removeListener: () => undefined,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}