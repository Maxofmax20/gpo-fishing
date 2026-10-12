import { beforeEach, expect, it, vi } from "vitest";
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
  mocks.check.mockResolvedValue({ version: "6.0.0", downloadAndInstall: install });
  await useUpdater.getState().check();
  expect(install).not.toHaveBeenCalled();
  expect(useUpdater.getState().available?.version).toBe("6.0.0");
});
it("failed installation allows retry and dismissal applies to one version", async () => {
  const install = vi.fn().mockRejectedValueOnce("offline").mockResolvedValueOnce(undefined);
  mocks.check.mockResolvedValue({ version: "6.0.0", downloadAndInstall: install });
  await useUpdater.getState().check();
  useUpdater.getState().dismiss();
  await useUpdater.getState().install();
  expect(useUpdater.getState().busy).toBe(false);
  await useUpdater.getState().install();
  expect(install).toHaveBeenCalledTimes(2);
  mocks.check.mockResolvedValue({ version: "6.0.1", downloadAndInstall: install });
  await useUpdater.getState().check();
  expect(useUpdater.getState().dismissedVersion).not.toBe(useUpdater.getState().available?.version);
});
