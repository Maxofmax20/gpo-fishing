import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CollectionControl } from "./CollectionControl";

const rpc = vi.hoisted(() => ({ mlCollectionStatus: vi.fn(), mlCollectionSet: vi.fn(), settingsGet: vi.fn() }));
vi.mock("../lib/ipc", () => ({ api: rpc }));
vi.mock("../lib/store", () => ({ showToast: vi.fn(), useStore: { setState: vi.fn() } }));
beforeEach(() => {
  vi.resetAllMocks();
  rpc.mlCollectionStatus.mockResolvedValue({ trace_on: true });
  rpc.mlCollectionSet.mockResolvedValue({ fishing: { trace: false } });
  rpc.settingsGet.mockResolvedValue({ fishing: { trace: false } });
});
afterEach(cleanup);

it("stops collection independently and makes enabling it available", async () => {
  render(<CollectionControl />);
  await userEvent.click(await screen.findByRole("button", { name: "Stop collecting data" }));
  expect(rpc.mlCollectionSet).toHaveBeenCalledExactlyOnceWith(false);
  expect(await screen.findByRole("button", { name: "Enable data collection" })).toBeEnabled();
});

it("retains the enabled state when saving fails", async () => {
  rpc.mlCollectionSet.mockRejectedValue("disk unavailable");
  render(<CollectionControl />);
  await userEvent.click(await screen.findByRole("button", { name: "Stop collecting data" }));
  expect(await screen.findByRole("button", { name: "Stop collecting data" })).toBeEnabled();
  expect(rpc.settingsGet).not.toHaveBeenCalled();
});

it("does not queue repeated changes while finalizing", async () => {
  let finish!: (value: unknown) => void;
  rpc.mlCollectionSet.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  render(<CollectionControl />);
  await userEvent.dblClick(await screen.findByRole("button", { name: "Stop collecting data" }));
  expect(rpc.mlCollectionSet).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "Saving…" })).toBeDisabled();
  finish({ fishing: { trace: false } });
  expect(await screen.findByRole("button", { name: "Enable data collection" })).toBeEnabled();
});
