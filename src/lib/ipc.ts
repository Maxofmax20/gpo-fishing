import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  BaitStock,
  BotState,
  CatchRecord,
  DropInfo, SpawnInfo,
  LogLine,
  OcrTest,
  OverlaySession,
  OverlayTarget,
  RegionsSession,
  PxRect,
  Reading,
  RegionPreview,
  RelPoint,
  RelRect,
  Settings,
  Snapshot,
  Stats,
  WindowInfo,
  CustomMacro,
  RecorderStatus,
  RecordMode,
  HealthCheck,
  VpnEngine,
  VpnMacroAction,
  DatasetSample,
  KnowledgeStats,
  KnowledgeEntity,
  WikiSyncResult,
  MlAnnotation,
  MlSessionState,
  TrainingReadiness,
  DatasetReport,
  BaselineReport,
  MlModelStatus,
  VpnStatus,
  PingResult,
  MultiRobloxStatus,
  SavedRobloxAccount,
} from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  // NOTE: bot_start/bot_pause/settings_get/app_quit/hud_toggle have no UI
  // callers (snapshot + bot_toggle cover them) and are intentionally not
  // wrapped here. The backend commands stay registered.
  botStop: () => invoke<void>("bot_stop"),
  botToggle: () => invoke<void>("bot_toggle"),
  settingsSet: (settings: Settings) => invoke<void>("settings_set", { settings }),
  settingsReset: () => invoke<Settings>("settings_reset"),
  presetList: () => invoke<string[]>("preset_list"),
  presetSave: (name: string) => invoke<void>("preset_save", { name }),
  presetLoad: (name: string) => invoke<Settings>("preset_load", { name }),
  presetDelete: (name: string) => invoke<void>("preset_delete", { name }),
  legacyImport: (json: string) => invoke<Settings>("legacy_import", { json }),
  overlayOpen: (target: OverlayTarget) => invoke<OverlaySession>("overlay_open", { target }),
  overlayCommit: (commit: { target: OverlayTarget; region?: RelRect | null; point?: RelPoint | null }) =>
    invoke<Settings>("overlay_commit", { commit }),
  overlayCancel: () => invoke<void>("overlay_cancel"),
  overlayOpenRegions: () => invoke<RegionsSession>("overlay_open_regions"),
  overlayPending: () => invoke<{ kind: "single"; session: OverlaySession } | { kind: "regions"; session: RegionsSession } | null>("overlay_pending"),
  overlayCommitRegions: (commit: { bar: RelRect; drop: RelRect }) => invoke<Settings>("overlay_commit_regions", { commit }),
  panelPlacementChanged: () => invoke<void>("panel_placement_changed"),
  regionPreview: (region: RelRect, maxDim = 320) => invoke<RegionPreview>("region_preview", { region, maxDim }),
  panelVisible: () => invoke<boolean>("panel_visible"),
  ocrTest: () => invoke<OcrTest>("ocr_test"),
  webhookTest: () => invoke<void>("webhook_test"),
  detectBarRegion: () => invoke<RelRect>("detect_bar_region"),
  hudSetOffset: (offset: RelPoint) => invoke<void>("hud_set_offset", { offset }),
  panelShow: () => invoke<void>("panel_show"),
  guideOpen: () => invoke<void>("guide_open"),
  guideHide: () => invoke<void>("guide_hide"),
  panelHide: () => invoke<void>("panel_hide"),
  panelToggle: () => invoke<void>("panel_toggle"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  dataDir: () => invoke<string>("data_dir"),
  getCatches: () => invoke<CatchRecord[]>("catches_list"),
  clearCatches: () => invoke<void>("catches_clear"),
  openCatches: () => invoke<void>("catches_open"),
  settingsLoadError: () => invoke<string | null>("settings_load_error"),
  webDashboardUrl: () => invoke<string>("web_dashboard_url"),
  webSetAllowLan: (allow: boolean) => invoke<Settings>("web_set_allow_lan", { allow }),
  webRegenerateToken: () => invoke<Settings>("web_regenerate_token"),
  healthCheck: () => invoke<HealthCheck>("health_check"),
  // Boss tracker has no Panel UI by design; it runs headless (auto-sync from
  // server age + Telegram /bosses). Kept wrapped for future UI work.
  bossTrackerScanServerAge: () =>
    invoke<{ success: boolean; uptime_sec: number; time_str: string; remaining_sec: number; is_spawned: boolean; message: string }>("boss_tracker_scan_server_age"),
  bossTrackerSyncServerAge: (timeStr: string) =>
    invoke<{ success: boolean; uptime_sec: number; time_str: string; remaining_sec: number; is_spawned: boolean; message: string }>("boss_tracker_sync_server_age", { timeStr }),
  scanBaitStock: () => invoke<BaitStock>("scan_bait_stock"),
  testGemini: (apiKey: string, model: string) => invoke<string>("test_gemini", { apiKey, model }),
  macroList: () => invoke<CustomMacro[]>("macro_list"),
  macroStatus: () => invoke<RecorderStatus>("macro_status"),
  macroRecord: (action: string, name?: string, mode?: RecordMode) =>
    invoke<{ ok: boolean; message: string; macro?: CustomMacro }>("macro_record", { action, name, mode }),
  macroPlay: (action: string, name?: string, loopMode?: boolean, speed?: number, maxLoops?: number) =>
    invoke<{ ok: boolean; message: string }>("macro_play", { action, name, loopMode, speed, maxLoops }),
  macroRename: (idOrName: string, newName: string) =>
    invoke<{ ok: boolean; message: string }>("macro_rename", { idOrName, newName }),
  macroAppendVpnStep: (nameOrId: string, action: VpnMacroAction, engine?: string, timeoutS?: number, required?: boolean) =>
    invoke<CustomMacro>("macro_append_vpn_step", { nameOrId, action, engine, timeoutS, required }),
  datasetList: () => invoke<DatasetSample[]>("dataset_list"),
  datasetLabel: (id: string, label: string, correct: boolean) =>
    invoke<DatasetSample>("dataset_label", { id, label, correct }),
  datasetOpen: () => invoke<void>("dataset_open"),
  knowledgeStats: () => invoke<KnowledgeStats>("knowledge_stats"),
  knowledgeList: () => invoke<KnowledgeEntity[]>("knowledge_list"),
  knowledgeSyncWiki: () => invoke<WikiSyncResult>("knowledge_sync_wiki"),
  mlSamples: () => invoke<MlAnnotation[]>("ml_samples"),
  mlAnnotate: (imageId: string, uiLabel?: string, gameState?: string, entityId?: string, hardExample?: boolean, hardReason?: string) =>
    invoke<MlAnnotation>("ml_annotate", { imageId, uiLabel, gameState, entityId, hardExample, hardReason }),
  mlValidate: () => invoke<DatasetReport>("ml_validate"),
  mlBaseline: () => invoke<BaselineReport>("ml_baseline"),
  mlModelStatus: () => invoke<MlModelStatus>("ml_model_status"),
  mlReadiness: () => invoke<TrainingReadiness>("ml_readiness"),
  macroDelete: (name: string) => invoke<{ ok: boolean; message: string }>("macro_delete", { name }),
  vpnGetStatus: () => invoke<VpnStatus>("vpn_get_status"),
  vpnConnect: (engine: VpnEngine) => invoke<VpnStatus>("vpn_connect", { engine }),
  vpnDisconnect: () => invoke<VpnStatus>("vpn_disconnect"),
  vpnTestPing: () => invoke<PingResult>("vpn_test_ping"),
  vpnResetNetwork: () => invoke<string>("vpn_reset_network"),
  vpnGetLogs: (maxLines?: number) => invoke<string[]>("vpn_get_logs", { maxLines }),
  vpnSetAutoReconnect: (enabled: boolean) => invoke<void>("vpn_set_auto_reconnect", { enabled }),
  multiRobloxGetStatus: () => invoke<MultiRobloxStatus>("multi_roblox_get_status"),
  multiRobloxSetEnabled: (enabled: boolean) => invoke<MultiRobloxStatus>("multi_roblox_set_enabled", { enabled }),
  // NOTE: multi_roblox_list_instances/launch have no UI callers (status
  // already includes instances; launches go through accounts). Not wrapped.
  multiRobloxFocusInstance: (pid: number) => invoke<void>("multi_roblox_focus_instance", { pid }),
  multiRobloxKillInstance: (pid: number) => invoke<void>("multi_roblox_kill_instance", { pid }),
  multiRobloxKillAll: () => invoke<number>("multi_roblox_kill_all"),
  multiRobloxSetTarget: (pid: number | null) => invoke<void>("multi_roblox_set_target", { pid }),
  multiRobloxListAccounts: () => invoke<SavedRobloxAccount[]>("multi_roblox_list_accounts"),
  multiRobloxAddAccount: (cookie: string, note?: string) =>
    invoke<SavedRobloxAccount>("multi_roblox_add_account", { cookie, note }),
  multiRobloxRemoveAccount: (id: string) => invoke<void>("multi_roblox_remove_account", { id }),
  multiRobloxLaunchAccount: (id: string, placeId?: number) =>
    invoke<void>("multi_roblox_launch_account", { id, placeId }),
};

type Events = {
  "bot:state": { kind: "state"; state: BotState; detail: string | null };
  "bot:stats": { kind: "stats" } & Stats;
  "bot:log": { kind: "log" } & LogLine;
  "bot:reading": Reading | null;
  "bot:fruit_drop": { kind: "fruit_drop" } & DropInfo;
  "bot:fruit_spawn": { kind: "fruit_spawn" } & SpawnInfo;
  "bot:purchase": { kind: "purchase"; amount: number };
  "bot:recovery": { kind: "recovery"; attempt: number; reason: string };
  "bot:ml_session": MlSessionState;
  "roblox:changed": WindowInfo | null;
  "overlay:roblox": PxRect;
  "overlay:session": OverlaySession;
  "overlay:regions": RegionsSession;
  "overlay:close": null;
  "overlay:view": null;
  "ui:visibility": boolean;
  "guide:open": null;
  "settings:changed": Settings;
  "panel:visible": boolean;
  "macro:status_changed": null;
};

export function on<K extends keyof Events>(name: K, cb: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return listen<Events[K]>(name, (e) => cb(e.payload));
}
