import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useVisiblePoll } from "./useVisiblePoll";

afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); });

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


it("removes interval wakeups while hidden and refreshes once on return", async () => {
  vi.useFakeTimers();
  let hidden = false;
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  const poll = vi.fn(async () => {});
  const { unmount } = renderHook(() => useVisiblePoll(poll, 100));
  await act(async () => {});
  expect(poll).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(1);
  hidden = true;
  await act(async () => { document.dispatchEvent(new Event("visibilitychange")); });
  expect(vi.getTimerCount()).toBe(0);
  await act(async () => { vi.advanceTimersByTime(10_000); });
  expect(poll).toHaveBeenCalledOnce();
  hidden = false;
  await act(async () => { document.dispatchEvent(new Event("visibilitychange")); });
  expect(poll).toHaveBeenCalledTimes(2);
  expect(vi.getTimerCount()).toBe(1);
  unmount();
  expect(vi.getTimerCount()).toBe(0);
});
