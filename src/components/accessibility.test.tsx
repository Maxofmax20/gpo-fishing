import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";
import ConfirmModal from "./ConfirmModal";
import { CustomSelect, Row, Toggle } from "./primitives";

afterEach(cleanup);

it("names row controls and navigates the select using the keyboard", async () => {
  const change = vi.fn();
  render(<><Row title="Record training data" right={<Toggle value={false} onChange={() => {}} />} />
    <Row title="Controller mode" right={<CustomSelect value="a" options={[{ value: "a", label: "A" }, { value: "b", label: "B" }]} onChange={change} />} /></>);
  expect(screen.getByRole("switch", { name: "Record training data" })).toBeInTheDocument();
  const select = screen.getByRole("button", { name: "Controller mode" });
  select.focus();
  await userEvent.keyboard("{ArrowDown}{ArrowDown}{Enter}");
  expect(change).toHaveBeenCalledWith("b");
  expect(select).toHaveFocus();
});

it("traps modal focus, supports Escape and restores the invoking control", async () => {
  const cancel = vi.fn();
  const { rerender } = render(<><button>Open</button><ConfirmModal open={false} title="Reset" body="Reset settings" onConfirm={() => {}} onCancel={cancel} /></>);
  screen.getByText("Open").focus();
  rerender(<><button>Open</button><ConfirmModal open title="Reset" body="Reset settings" onConfirm={() => {}} onCancel={cancel} /></>);
  expect(screen.getByRole("dialog", { name: "Reset" })).toHaveAttribute("aria-modal", "true");
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
  expect(screen.getByRole("button", { name: "Confirm" })).toHaveFocus();
  await userEvent.keyboard("{Escape}");
  expect(cancel).toHaveBeenCalledOnce();
  rerender(<><button>Open</button><ConfirmModal open={false} title="Reset" body="Reset settings" onConfirm={() => {}} onCancel={cancel} /></>);
  expect(screen.getByText("Open")).toHaveFocus();
});

it("cannot dismiss a busy operation with Escape or the backdrop", async () => {
  const cancel = vi.fn();
  const { container } = render(<ConfirmModal open busy title="Reset" body="Reset settings" onConfirm={() => {}} onCancel={cancel} />);
  fireEvent.click(container.firstChild!);
  await userEvent.keyboard("{Escape}");
  expect(cancel).not.toHaveBeenCalled();
});
