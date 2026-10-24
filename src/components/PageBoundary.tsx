import { Component, type ReactNode } from "react";

/** Keep navigation usable if an optional page crashes. */
export default class PageBoundary extends Component<{ children: ReactNode }, { error: boolean }> {
  state = { error: false };
  static getDerivedStateFromError() { return { error: true }; }
  render() {
    if (this.state.error) return <section role="alert" className="p-5 space-y-3">
      <h2 className="text-base font-semibold">This page could not load</h2>
      <p className="text-fg-dim">Fishing runs separately. You can switch pages or retry this view.</p>
      <button className="px-3 py-2 rounded-lg bg-accent-soft text-accent" onClick={() => this.setState({ error: false })}>Retry page</button>
    </section>;
    return this.props.children;
  }
}
