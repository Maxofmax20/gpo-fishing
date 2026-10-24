import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ check: vi.fn(), toast: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("./store", () => ({ showToast: mocks.toast }));
import { useUpdater } from "./updater";
describe("signed updater state", () => {
  beforeEach(() => { vi.clearAllMocks(); useUpdater.setState({ available: null, busy: false, message: null }); });
  it("reports actual byte progress and install phase", async () => {
    const messages: (string | null)[] = [];
    const update = { version: "5.9.0", downloadAndInstall: vi.fn(async (progress) => {
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
