import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useVisiblePoll } from "./useVisiblePoll";

afterEach(() => { cleanup(); vi.useRealTimers(); });

it("never starts a second poll while the first request is pending", async () => {
  vi.useFakeTimers();
  let finish!: () => void;
  const poll = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
  renderHook(() => useVisiblePoll(poll, 100));
  await act(async () => { vi.advanceTimersByTime(500); });
  expect(poll).toHaveBeenCalledOnce();
  await act(async () => { finish(); });
  await act(async () => { vi.advanceTimersByTime(100); });
  expect(poll).toHaveBeenCalledTimes(2);
});

it("stops polling after unmount", async () => {
  vi.useFakeTimers();
  const poll = vi.fn(async () => {});
  const { unmount } = renderHook(() => useVisiblePoll(poll, 100));
  await act(async () => {});
  unmount();
  await act(async () => { vi.advanceTimersByTime(500); });
  expect(poll).toHaveBeenCalledOnce();
});
