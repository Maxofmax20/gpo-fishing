import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import PageBoundary from "./PageBoundary";
describe("page recovery", () => {
  it("keeps navigation alive and retries a failed page", () => {
    let fails = true;
    function Page() { if (fails) throw Error("page failed"); return <p>Recovered page</p>; }
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      render(<div><button>Navigation</button><PageBoundary><Page /></PageBoundary></div>);
      expect(screen.getByRole("alert")).toHaveTextContent("could not load");
      expect(screen.getByRole("button", { name: "Navigation" })).toBeEnabled();
      fails = false; fireEvent.click(screen.getByRole("button", { name: "Retry page" }));
      expect(screen.getByText("Recovered page")).toBeInTheDocument();
    } finally { errors.mockRestore(); }
  });
});
