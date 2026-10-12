import { beforeEach, expect, it, vi } from "vitest";
import type { Settings } from "./types";

const mock = vi.hoisted(() => ({ get: vi.fn(), save: vi.fn() }));
vi.mock("./ipc", () => ({ api: { settingsGet: mock.get, settingsSet: mock.save }, on: vi.fn() }));
import { useStore } from "./store";

let backend: Settings;
beforeEach(() => {
  backend = { fishing: { trace: false }, auto_update: false } as Settings;
  mock.get.mockReset().mockImplementation(async () => structuredClone(backend));
  mock.save.mockReset().mockImplementation(async (next: Settings) => { backend = next; return structuredClone(next); });
  useStore.setState({ settings: structuredClone(backend), saving: false, error: null, toasts: [] });
});

it("serializes edits against acknowledged settings and retains both changes", async () => {
  const first = useStore.getState().update((s) => { s.fishing.trace = true; });
  const second = useStore.getState().update((s) => { s.auto_update = true; });
  expect(useStore.getState().saving).toBe(true);
  await Promise.all([first, second]);
  expect(backend.fishing.trace).toBe(true);
  expect(backend.auto_update).toBe(true);
  expect(useStore.getState().settings).toEqual(backend);
  expect(useStore.getState().saving).toBe(false);
});

it("reports rejection, keeps acknowledged settings and permits the next save", async () => {
  mock.save.mockRejectedValueOnce("disk full");
  await expect(useStore.getState().update((s) => { s.fishing.trace = true; })).rejects.toBe("disk full");
  expect(useStore.getState().settings?.fishing.trace).toBe(false);
  expect(useStore.getState().toasts[0].msg).toContain("disk full");
  await useStore.getState().update((s) => { s.auto_update = true; });
  expect(backend.auto_update).toBe(true);
});

it("uses normalized backend values and reports read failures", async () => {
  mock.save.mockImplementationOnce(async (next: Settings) => ({ ...next, auto_update: false }));
  await useStore.getState().update((s) => { s.auto_update = true; });
  expect(useStore.getState().settings?.auto_update).toBe(false);
  mock.get.mockRejectedValueOnce("offline");
  await expect(useStore.getState().update(() => {})).rejects.toBe("offline");
  expect(useStore.getState().toasts.at(-1)?.msg).toContain("offline");
});
