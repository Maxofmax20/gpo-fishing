import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { showToast } from "./store";

type Updater = {
  available: Update | null;
  busy: boolean;
  message: string | null;
  dismissedVersion: string | null;
  check: (quiet?: boolean) => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
};

export const useUpdater = create<Updater>((set, get) => ({
  available: null, busy: false, message: null, dismissedVersion: null,
  check: async (quiet = false) => {
    if (get().busy) return;
    set({ busy: true, message: quiet ? get().message : "Checking…" });
    try {
      const update = await check({ timeout: 15000 });
      const previous = get().available;
      if (previous && previous !== update) await previous.close().catch(() => {});
      set({ available: update, message: update ? `Version ${update.version} is available.` : "You're on the latest version." });
    } catch (e) {
      set({ message: `Update check failed: ${String(e)}` });
      if (!quiet) showToast("warn", `Update check failed: ${String(e)}`);
    } finally { set({ busy: false }); }
  },
  install: async () => {
    const update = get().available;
    if (!update || get().busy) return;
    set({ busy: true, message: `Downloading ${update.version}…` });
    let handoff: string | null = null;
    try {
      let downloaded = 0;
      let total: number | undefined;
      let shown = "";
      await update.download((progress) => {
        if (progress.event === "Started") { total = progress.data.contentLength; downloaded = 0; }
        if (progress.event === "Progress") downloaded += progress.data.chunkLength;
        const message = progress.event === "Finished" ? "Download complete; verifying…"
          : total ? `Downloading ${Math.min(100, Math.floor(downloaded * 100 / total))}%`
          : `Downloading ${(downloaded / 1048576).toFixed(1)} MB`;
        if (message !== shown) { shown = message; set({ message }); }
      }, { timeout: 120000 });
      set({ message: "Waiting for the current reel or fruit workflow to finish…" });
      handoff = await invoke<string>("update_prepare", { version: update.version });
      set({ message: "Installing; fishing will resume after restart if it was running." });
      await update.install();
      set({ message: "Installed. Reopening with your saved fishing state…" });
    } catch (e) {
      if (handoff) {
        await invoke("update_cancel", { requestId: handoff }).catch((recovery) => {
          showToast("error", "Update recovery needs attention: " + String(recovery));
        });
      }
      set({ message: `Update failed: ${String(e)}. You can retry.` });
      showToast("error", `Update failed: ${String(e)}`);
    } finally { set({ busy: false }); }
  },
  dismiss: () => set({ dismissedVersion: get().available?.version ?? null }),
}));
