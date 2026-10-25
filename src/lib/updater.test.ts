import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ check: vi.fn(), toast: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("./store", () => ({ showToast: mocks.toast }));
import { useUpdater } from "./updater";

beforeEach(() => {
  mocks.check.mockReset(); mocks.toast.mockReset();
  useUpdater.setState({ available: null, busy: false, message: null, dismissedVersion: null });
});
it("checking never installs an available update", async () => {
  const install = vi.fn();
  mocks.check.mockResolvedValue({ version: "6.0.0", downloadAndInstall: install, close: vi.fn().mockResolvedValue(undefined) });
  await useUpdater.getState().check();
  expect(install).not.toHaveBeenCalled();
  expect(useUpdater.getState().available?.version).toBe("6.0.0");
});
it("failed installation allows retry and dismissal applies to one version", async () => {
  const install = vi.fn().mockRejectedValueOnce("offline").mockResolvedValueOnce(undefined);
  mocks.check.mockResolvedValue({ version: "6.0.0", downloadAndInstall: install, close: vi.fn().mockResolvedValue(undefined) });
  await useUpdater.getState().check();
  useUpdater.getState().dismiss();
  await useUpdater.getState().install();
  expect(useUpdater.getState().busy).toBe(false);
  await useUpdater.getState().install();
  expect(install).toHaveBeenCalledTimes(2);
  mocks.check.mockResolvedValue({ version: "6.0.1", downloadAndInstall: install, close: vi.fn().mockResolvedValue(undefined) });
  await useUpdater.getState().check();
  expect(useUpdater.getState().dismissedVersion).not.toBe(useUpdater.getState().available?.version);
});

describe("signed updater state", () => {
  beforeEach(() => { vi.clearAllMocks(); useUpdater.setState({ available: null, busy: false, message: null }); });
  it("reports actual byte progress and install phase", async () => {
    const messages: (string | null)[] = [];
    const update = { version: "5.9.0", close: vi.fn().mockResolvedValue(undefined), downloadAndInstall: vi.fn(async (progress) => {
      progress({ event: "Started", data: { contentLength: 100 } });
      messages.push(useUpdater.getState().message);
      progress({ event: "Progress", data: { chunkLength: 40 } });
      messages.push(useUpdater.getState().message);
      progress({ event: "Finished" }); messages.push(useUpdater.getState().message);
    }) };
    useUpdater.setState({ available: update as any });
    await useUpdater.getState().install();
    expect(messages).toEqual(["Downloading 0%", "Downloading 40%", "Verifying and installing…"]);
    expect(useUpdater.getState().busy).toBe(false);
  });
  it("surfaces download failure and allows retry", async () => {
    useUpdater.setState({ available: { version: "5.9.0", downloadAndInstall: vi.fn().mockRejectedValue(Error("offline")) } as any });
    await useUpdater.getState().install();
    expect(useUpdater.getState().message).toContain("offline");
    expect(useUpdater.getState().busy).toBe(false); expect(mocks.toast).toHaveBeenCalled();
  });
});


it("successful installation clears and releases the update so install cannot repeat", async () => {
  const install = vi.fn().mockResolvedValue(undefined);
  const close = vi.fn().mockResolvedValue(undefined);
  mocks.check.mockResolvedValue({ version: "6.0.6", downloadAndInstall: install, close });
  await useUpdater.getState().check();
  await useUpdater.getState().install();
  await useUpdater.getState().install();
  expect(install).toHaveBeenCalledOnce();
  expect(close).toHaveBeenCalledOnce();
  expect(useUpdater.getState().available).toBeNull();
  expect(useUpdater.getState().message).toContain("Restart");
});
