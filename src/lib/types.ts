export type RelPoint = { x: number; y: number };
export type RelRect = { x: number; y: number; w: number; h: number };
export type PxPoint = { x: number; y: number };
export type PxRect = { x: number; y: number; w: number; h: number };
export type Rgb = { r: number; g: number; b: number };

export type WindowInfo = { client: PxRect; is_foreground: boolean; visible: boolean; dpi: number };

export type BotState =
  | "stopped"
  | "waiting_for_roblox"
  | "initial_setup"
  | "casting"
  | "waiting_for_bite"
  | "tracking"
  | "post_catch"
  | "purchasing"
  | "storing_fruit"
  | "recovering"
  | "paused";

export type Lifetime = {
  fish: number;
  failed: number;
  fruits: number;
  bait_purchased: number;
  runtime_s: number;
  sessions: number;
  last_fish: string | null;
  last_fruit: string | null;
  last_spawn: string | null;
  pity_fruit: number;
  pity_legendary: number;
  estimated_peli: number;
};

export type Stats = {
  fish: number;
  failed: number;
  fruits: number;
  bait_purchased: number;
  success_rate: number;
  runtime_s: number;
  restarts: number;
  last_fish: string | null;
  last_fruit: string | null;
  last_spawn: string | null;
  pity_fruit: number;
  pity_legendary: number;
  fish_per_hour: number;
  estimated_peli: number;
  since_purchase: number;
  since_progress: number;
  total: Lifetime;
};

export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogLine = { ts: number; level: LogLevel; msg: string };

export type Span = { start: number; end: number };
export type Bbox = { x0: number; y0: number; x1: number; y1: number };
export type Reading = {
  bar: Bbox;
  fish: Span;
  marker: Span;
  fish_center: number;
  marker_center: number;
  error: number;
  hold: boolean;
  predicted_error: number;
  fish_velocity: number;
  marker_velocity: number;
  origin_dx: number;
};
export type Confidence = { bar: number; fish: number; marker: number; score: number };

export type DropInfo = { text: string; is_legendary: boolean; name?: string | null; pity?: string | null };
export type SpawnInfo = { text: string; name: string | null; location: string | null };

export type CatchRecord = {
  timestamp: string;
  kind: "fish" | "fruit" | string;
  name: string;
  raw: string;
};

export type Settings = {
  version: number;
  regions: { bar: RelRect; drop: RelRect; server_time: RelRect; bait_menu: RelRect };
  points: {
    fishing: RelPoint;
    purchase: [RelPoint | null, RelPoint | null, RelPoint | null, RelPoint | null];
    fruit: [RelPoint | null, RelPoint | null];
    bait: [RelPoint | null, RelPoint | null];
    rod_slot: RelPoint | null;
  };
  keys: { rod: string; fruit_slot_1: string; fruit_slot_2: string; shop: string; reset_slot: string };
  fishing: {
    control: {
      mode: "lookahead" | "physics";
      lookahead_ms: number;
      hysteresis: number;
      velocity_smoothing: number;
      invert: boolean;
      physics: { accel_hold: number; accel_release: number; max_speed: number; latency_ms: number; calibrated_at: number };
    };
    palette: { bar: Rgb; fish: Rgb; marker: Rgb; tolerance: number; min_bar_height_px: number; min_bar_aspect: number; min_bar_fill: number; min_row_fraction: number };
    scan_timeout_s: number;
    track_timeout_s: number;
    min_track_s: number;
    bite_confirm_frames: number;
    lost_frames: number;
    wait_after_catch_s: number;
    cast_hold_ms: number;
    scan_hz: number;
    track_hz: number;
    trace: boolean;
  };
  features: {
    auto_zoom: boolean;
    auto_mouse_position: boolean;
    auto_bait: boolean;
    smart_bait: boolean;
    fruit_storage: boolean;
    auto_purchase: boolean;
    zero_bait_failsafe: boolean;
    fast_reset: boolean;
    telegram_remote: boolean;
    discord_rpc: boolean;
    multi_roblox: boolean;
  };
  purchase: {
    amount: number;
    every_n_catches: number;
    hold_shop_key_ms: number;
    after_key_ms: number;
    click_delay_ms: number;
    after_type_ms: number;
    bait_tier: BaitTier;
    low_bait_threshold: number;
    max_bait: number;
    legendary_reserve: number;
  };
  zoom: { out_steps: number; in_steps: number; step_delay_ms: number; sequence_delay_ms: number };
  fruit_storage: { key_settle_ms: number; click_settle_ms: number; dialog_wait_ms: number; after_drop_ms: number; never_drop_legendary_or_mythical: boolean; pause_on_protected_fruit: boolean; keep_pity_zero_fruit: boolean; all_seeing_eye: boolean; pity_cap: number };
  ocr: { spawn_check_interval_s: number; spawn_cooldown_s: number; post_catch_reads: number; post_catch_read_gap_ms: number };
  lexicon: { fruits: string[]; drop_phrases: string[]; drop_keywords: string[]; spawn_keywords: string[]; catch_phrases: string[]; fail_phrases: string[]; fuzzy_threshold: number };
  webhook: {
    provider: "telegram" | "discord" | "both";
    url: string;
    telegram_bot_token: string;
    telegram_chat_id: string;
    enabled: boolean;
    progress_every_n: number;
    progress: boolean;
    fruit_drop: boolean;
    spawn: boolean;
    purchase: boolean;
    recovery: boolean;
    legendary_only: boolean;
    send_screenshot: boolean;
    send_catch_screenshot: boolean;
    send_drop_screenshot: boolean;
    crop_fruit_screenshot: boolean;
    disconnect_alert: boolean;
    bait_alert: boolean;
  };
  boss_tracker: {
    enabled: boolean;
    notify_5m: boolean;
    notify_spawn: boolean;
    notify_hawkeye: boolean;
    notify_roger: boolean;
    notify_soulking: boolean;
    notify_radiant_admiral: boolean;
    notify_merchant: boolean;
    hawkeye_offset: number | null;
    roger_offset: number | null;
    soulking_offset: number | null;
    radiant_admiral_offset: number | null;
    merchant_offset: number | null;
  };
  hotkeys: { toggle: string; overlay: string; quit: string; hide_hud: string; record_toggle: string };
  ui: { theme: string; hud_offset: RelPoint; hud_visible: boolean; panel_offset: RelPoint; panel_size: [number, number]; log_level: string };
  watchdog: { enabled: boolean; heartbeat_timeout_s: number; max_restarts: number; restart_backoff_s: number };
  auto_update: boolean;
  gemini: {
    enabled: boolean;
    api_key: string;
    model: string;
  };
  web: { allow_lan: boolean; token: string };
  game: { spawn_banner: RelRect; disconnect_region: RelRect; reconnect_point: RelPoint; gpo_place_id: number };
};

export type Snapshot = {
  state: BotState;
  paused: boolean;
  stats: Stats;
  roblox: WindowInfo | null;
  ocr_available: boolean;
  settings: Settings;
  version: string;
};

export type BaitTier = "common" | "rare" | "legendary" | "highest";

export type BaitStock = {
  legendary: number | null;
  rare: number | null;
  common: number | null;
};

export type OverlayTarget =
  | "bar_region"
  | "drop_region"
  | "server_time_region"
  | "bait_menu_region"
  | "fishing_point"
  | "purchase1"
  | "purchase2"
  | "purchase3"
  | "purchase4"
  | "fruit1"
  | "fruit2"
  | "bait1"
  | "bait2"
  | "rod_slot";

export type OverlaySession = {
  target: OverlayTarget;
  roblox: PxRect;
  overlay_origin: PxPoint;
  region: RelRect | null;
  point: RelPoint | null;
};

export type RegionsSession = { roblox: PxRect; bar: RelRect; drop: RelRect; server_time: RelRect; bait_menu: RelRect };

export type Geometry = { bar: Bbox; fish: Span; marker: Span; fish_center: number; marker_center: number; error: number };

export type RegionPreview = {
  width: number;
  height: number;
  png_base64: string;
  confidence: Confidence;
  reading: Geometry | null;
};

export type OcrTest = {
  text: string;
  ocr_variant: string;
  drop: DropInfo | null;
  spawn: SpawnInfo | null;
  observation: Observation;
};

export type Evidence = { kind: string; detail: string; weight: number };

export type EntityMatch = {
  entity_id: string;
  canonical_name: string;
  category: string;
  confidence: number;
  evidence: Evidence[];
};

export type Observation = {
  timestamp_ms: number;
  source: string;
  region: string;
  screen: string;
  ocr: { text: string; region: string } | null;
  vision_hint: { label: string; confidence: number } | null;
  candidates: EntityMatch[];
  confidence: number;
  entity: EntityMatch | null;
  unknown_reason: string | null;
};

export type DatasetSample = {
  id: string;
  timestamp_ms: number;
  event_type: string;
  region: string;
  game_state: string;
  ocr_text: string;
  observation: Observation;
  needs_label: boolean;
  label: string | null;
  label_correct: boolean | null;
  png_file: string;
};

export type KnowledgeStats = {
  version: number;
  entities: number;
  fruits: number;
  fish: number;
  bait: number;
  ui_terms: number;
  overlay: number;
};

export type KnowledgeEntity = { id: string; name: string; category: string };

export type MlAnnotation = {
  image_id: string;
  dataset_version: number;
  session_id: string;
  task: string;
  ocr_text: string;
  region_name: string;
  ui_label: string | null;
  bbox: { x: number; y: number; w: number; h: number } | null;
  game_state: string | null;
  entity_id: string | null;
  annotator: string;
  timestamp_ms: number;
  source: string;
  confidence: number | null;
  hard_example: boolean;
  hard_reason: string | null;
  corrections: { at_ms: number; prev_label: string | null; note: string }[];
};

export type DatasetReport = {
  dataset: string;
  images: number;
  labeled: number;
  unlabeled: number;
  classes: Record<string, number>;
  sessions_train: number;
  sessions_validation: number;
  sessions_test: number;
  leakage_sessions: string[];
  same_file_cross_split: string[];
  near_similarity_groups: number;
  corrupt_files: string[];
  missing_labels: number;
  invalid_entity_ids: string[];
  duplicate_groups: number;
  near_duplicate_pairs: number;
  invalid_bboxes: string[];
  orphan_annotations: string[];
  orphan_images: string[];
  min_max_class_ratio: number;
  ok: boolean;
};

export type StageMetrics = {
  n: number;
  accuracy: number;
  precision: number;
  recall: number;
  f1: number;
  unknown_rate: number;
  mean_latency_ms: number;
};

export type BaselineReport = {
  dataset: string;
  labeled_samples: number;
  insufficient_data: boolean;
  stages: Record<string, StageMetrics>;
  confusion: Record<string, Record<string, number>>;
  notes: string[];
};

export type MlSessionState = {
  collecting: boolean;
  session_id: string | null;
  samples: number;
  reels: number;
  hard_examples: number;
  dropped: number;
  quality_ok: boolean | null;
  quality_warnings: string[];
  pending_annotation: number;
  total_samples: number;
};

export type StateEligibility = {
  total: number;
  eligible: number;
  excluded: number;
  transition: number;
  sessions: number;
};

export type LeakageDetail = {
  exact_duplicate_files: string[];
  near_similarity_groups: number;
  total_groups: number;
};

export type TrainingReadiness = {
  ready: boolean;
  training: string;
  verified: number;
  required_verified: number;
  entity_linked_result: number;
  required_entity_linked: number;
  state_coverage: Record<string, StateEligibility>;
  min_state_eligible: number;
  sessions: number;
  required_sessions: number;
  test_sessions: number;
  class_coverage_ok: boolean;
  leakage_ok: boolean;
  validation_ok: boolean;
  leakage_detail: LeakageDetail;
  hard_examples: number;
  entities: number;
  blocking_requirement: string;
  reasons: string[];
};

export type MlModelStatus = {
  available: boolean;
  name: string | null;
  version: string | null;
  dataset: string | null;
  runtime: string | null;
  classes: string[];
  reason: string | null;
};

export type PreflightCheck = {
  name: string;
  ok: boolean;
  detail: string;
};

export type MlPreflight = {
  trace_on: boolean;
  ok: boolean;
  checks: PreflightCheck[];
};

export type MlCollectionStatus = {
  trace_on: boolean;
  session_active: boolean;
  recording: boolean;
  session_id: string | null;
  samples_written: number;
  reels: number;
  hard_examples: number;
  dropped: number;
  write_errors: number;
  queue_depth: number;
  last_capture_ms: number | null;
  dataset: string;
  dataset_samples: number;
  verified: number;
};

export type WikiSyncResult = {
  category_titles: number;
  fetched_pages: number;
  truncated: boolean;
  parsed: number;
  added: number;
  updated: number;
  skipped: number;
  errors: string[];
};

export type HealthItem = {
  id: string;
  label: string;
  status: "pass" | "warn" | "fail" | string;
  detail: string;
  score: number | null;
  configured: boolean;
  detected: boolean;
};

export type HealthCheck = { roblox: boolean; knowledge_entities: number; items: HealthItem[] };

export const STATE_LABEL: Record<BotState, string> = {
  stopped: "Idle",
  waiting_for_roblox: "Waiting for Roblox",
  initial_setup: "Setting up",
  casting: "Casting",
  waiting_for_bite: "Waiting for bite",
  tracking: "Reeling",
  post_catch: "Checking catch",
  purchasing: "Buying bait",
  storing_fruit: "Storing fruit",
  recovering: "Recovering",
  paused: "Paused",
};

export const TARGET_LABEL: Record<OverlayTarget, string> = {
  bar_region: "Fishing bar area",
  drop_region: "Drop message area",
  server_time_region: "Server timer area",
  bait_menu_region: "Bait menu area",
  fishing_point: "Cast point",
  purchase1: "Shop confirm button",
  purchase2: "Shop quantity box",
  purchase3: "Shop buy button",
  purchase4: "Shop middle button (OK / Close)",
  fruit1: "Fruit slot",
  fruit2: "Fruit slot (backup)",
  bait1: "Bait slot",
  bait2: "Bait slot (backup)",
  rod_slot: "Rod slot indicator (optional)",
};

export type VpnMacroAction = "connect" | "disconnect" | "wait_connected" | "wait_disconnected";

export type MacroStep =
  | { type: "Click"; rx: number; ry: number; button: string; delay_ms: number }
  | { type: "Drag"; start_rx: number; start_ry: number; end_rx: number; end_ry: number; duration_ms: number; delay_ms: number }
  | { type: "KeyTap"; key: string; delay_ms: number }
  | { type: "KeyHold"; key: string; duration_ms: number; delay_ms: number }
  | { type: "MouseMove"; rx: number; ry: number; delay_ms: number }
  | { type: "Sleep"; ms: number }
  | { type: "VpnConnect"; engine: string; timeout_s: number; required: boolean }
  | { type: "VpnDisconnect"; timeout_s: number }
  | { type: "VpnWaitConnected"; timeout_s: number; required: boolean }
  | { type: "VpnWaitDisconnected"; timeout_s: number };

export type CustomMacro = {
  id: string;
  name: string;
  created_at: string;
  steps: MacroStep[];
};

export type RecordMode = "pc" | "web";

export type RecorderStatus = {
  is_recording: boolean;
  record_mode: "WebScreen" | "PcWindow" | null;
  recorded_steps_count: number;
  is_playing: boolean;
  playing_macro_name: string | null;
  current_loop: number;
  is_looping: boolean;
  message: string;
};

export type VpnEngine = "auto" | "dedicated" | "warp" | "psiphon" | "wireguard" | "proton" | "windscribe" | "openvpn" | "tailscale" | "mullvad" | "nord" | "clash" | "nekobox" | "v2ray" | "generic" | "none" | (string & {});

export type VpnState =
  | "disconnected"
  | "connecting"
  | "verifying"
  | "connected"
  | "disconnecting"
  | "error"
  | "unknown";

export type VpnStatus = {
  connected: boolean;
  /** Evidence-based lifecycle state. `connected` is true ONLY when verified. */
  state: VpnState;
  /** True only when this app established the verified connection. */
  managed: boolean;
  engine: string;
  engine_name?: string;
  ip: string;
  country: string;
  city: string;
  latency_ms: number | null;
  uptime_secs: number;
  auto_reconnect: boolean;
  last_error: string | null;
  auto_detected?: boolean;
  process_running: boolean;
  tunnel_detected: boolean;
  verification_detail: string;
};

export type PingResult = {
  success: boolean;
  latency_ms: number;
  target: string;
  error: string | null;
};

export type RobloxInstanceInfo = {
  pid: number;
  hwnd: number | null;
  user_id: string | null;
  username: string | null;
  display_name: string | null;
  avatar_url: string | null;
  universe_id: string | null;
  game_name: string | null;
  is_target: boolean;
};

export type MultiRobloxStatus = {
  enabled: boolean;
  mutex_locked: boolean;
  cookie_locked: boolean;
  instances_count: number;
  instances: RobloxInstanceInfo[];
  target_pid: number | null;
};

export type SavedRobloxAccount = {
  id: string;
  user_id: number;
  username: string;
  display_name: string;
  avatar_url: string | null;
  created_at: string;
  note: string | null;
  is_running: boolean;
  running_pid: number | null;
};




export type JobStatus =
  | "QUEUED" | "RUNNING" | "EVALUATING" | "PASSED"
  | "REJECTED" | "FAILED" | "CANCELLED" | "INTERRUPTED";

export type JobProgress = {
  stage: string;
  epoch: number;
  total_epochs: number;
  train_loss: number | null;
  val_metric: number | null;
  learning_rate: number | null;
  sec_per_epoch: number | null;
  eta_s: number | null;
  elapsed_s: number;
  log_tail: string[];
};

export type TrainingJob = {
  job_id: string;
  model_family: string;
  dataset_version: number;
  dataset_fingerprint: string;
  frozen_rows: number;
  frozen_sessions: number;
  frozen_test_sessions: string[];
  review_fingerprint: string;
  snapshot_dir: string;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
  status: JobStatus;
  requested_by: string;
  trainer_module: string;
  run_id: string;
  epochs: number;
  seed: number;
  code_version: string;
  trainer_dir: string;
  work_dir: string;
  candidate_id: string | null;
  evaluation: Record<string, unknown> | null;
  error: string | null;
  progress: JobProgress;
  deferred_reason: string | null;
  macro_running_at_start: boolean;
};

export type GateCheck = { ok: boolean; text: string };

export type FamilyEligibility = { family: string; eligible: boolean; checks: GateCheck[] };

export type DeployedModelInfo = {
  family: string;
  name: string;
  version: string;
  test_accuracy: number | null;
  macro_f1: number | null;
};

export type TrainingOverview = {
  dataset_version: number;
  rows: number;
  sessions: number;
  last_collection_ms: number;
  last_training_ms: number | null;
  new_since_training: number;
  models: DeployedModelInfo[];
  shadow_enabled: boolean;
  shadow_events: number;
  eligibility: FamilyEligibility[];
  auto_enabled: boolean;
  backend_available: boolean;
  backend_detail: string;
  production_control: string;
};

export type BackendStatus = {
  available: boolean;
  python_path: string;
  python_ok: boolean;
  torch_ok: boolean;
  torch_version: string | null;
  trainer_dir_ok: boolean;
  detail: string;
};

export type Lifecycle =
  | "TRAINED" | "EVALUATED" | "SHADOW" | "SHADOW_VALIDATED"
  | "ACCEPTED" | "REJECTED" | "ROLLED_BACK";

export type ModelMetrics = {
  accuracy: number;
  macro_f1: number;
  ece: number | null;
  test_sessions: number;
  test_n: number;
  per_class_f1: Record<string, number>;
  kept_accuracy: number | null;
};

export type CandidateRecord = {
  candidate_id: string;
  model_family: string;
  model_version: number;
  parent_name: string;
  parent_sha: string | null;
  dataset_fingerprint: string;
  dataset_version: number;
  training_config_hash: string;
  code_version: string;
  job_id: string;
  created_at: number;
  artifact_sha: string;
  manifest_sha: string | null;
  eval_sha: string;
  metrics: ModelMetrics;
  classes: string[];
  temperature: number;
  mean: [number, number, number];
  std: [number, number, number];
  status: Lifecycle;
  decision_reason: string | null;
};

export type ComparisonVerdict = "Pass" | "Reject" | "Inconclusive";

export type Comparison = {
  acc_delta: number;
  f1_delta: number;
  ece_delta: number | null;
  regressions: [string, number, number][];
  improvements: [string, number, number][];
  verdict: ComparisonVerdict;
  reasons: string[];
};

export type CompareView = {
  candidate: CandidateRecord;
  current_metrics: ModelMetrics;
  comparison: Comparison;
};

export type ExplorerEntity = {
  entity: string;
  examples: number;
  sessions: number;
  train: number;
  validation: number;
  test: number;
  ocr_agree: number;
  ocr_disagree: number;
  qualified: boolean;
  reason: string;
};

export type DatasetExplorer = {
  rows: number;
  sessions: number;
  states: Record<string, number>;
  entities: number;
  hard_examples: number;
  ocr_bearing: number;
  ocr_empty_result: number;
  null_entity_result: number;
  fish: ExplorerEntity[];
  fruits: ExplorerEntity[];
};

export type ReviewItem = {
  image_id: string;
  session_id: string;
  timestamp_ms: number;
  ocr_text: string;
  entity_id: string | null;
  reasons: string[];
};

export type TrainingSettings = {
  auto_enabled: boolean;
  min_new_samples: number;
  min_new_sessions: number;
  require_held_out_test: boolean;
  auto_promote_to_shadow: boolean;
  auto_rollback: boolean;
  max_concurrent_jobs: number;
  allow_fish: boolean;
  allow_fruit: boolean;
  python_path: string;
  trainer_dir: string;
  trigger_cooldown_hours: number;
  defer_while_fishing: boolean;
};

export type HistoryEntry = { ts: number; kind: string; detail: unknown };


export type ReviewStatus =
  | "UNREVIEWED" | "REVIEWED_CORRECT" | "REVIEWED_CORRECTED"
  | "REVIEWED_UNKNOWN" | "REVIEWED_SKIPPED" | "CONFLICT";

export type ReviewRecord = {
  image_id: string;
  event_id: string | null;
  session_id: string;
  review_status: ReviewStatus;
  model_prediction: string | null;
  model_confidence: number | null;
  human_entity_id: string | null;
  human_canonical_name: string | null;
  ocr_text: string | null;
  ocr_prediction: string | null;
  vision_prediction: string | null;
  vision_ocr_agreement: boolean | null;
  is_hard_example: boolean;
  reviewed_at: number | null;
  reviewer_version: string;
  correction_reason: string | null;
  training_eligible: boolean;
  excluded_reason: string | null;
  dataset_version: number;
};

export type CoverageReport = {
  total: number;
  reviewed: number;
  correct: number;
  corrected: number;
  unknown: number;
  skipped: number;
  conflicts: number;
  sessions: number;
  eligible: number;
  excluded: number;
};

export type PerEntityReview = {
  entity: string;
  collected: number;
  reviewed: number;
  confirmed: number;
  corrected: number;
  unknown: number;
  sessions: number;
  eligible: number;
};

export type ReviewCoverageView = {
  total_rows: number;
  coverage: CoverageReport;
  per_entity: PerEntityReview[];
};

export type PriorityItem = {
  image_id: string;
  session_id: string;
  timestamp_ms: number;
  ocr_text: string;
  entity_id: string | null;
  review_status: string;
  score: number;
  reason: string;
};

export type EntityHit = {
  entity_id: string;
  canonical_name: string;
  category: string;
  kind: string;
};

export type DropEntry = {
  entity_id: string;
  canonical_name: string;
  category: string;
  fishing_drop: boolean;
  rarity: string | null;
  aliases: string[];
  wiki_source: string;
  wiki_url: string | null;
  collected: number;
  reviewed: number;
  eligible: number;
  sessions: number;
  model_status: string;
};

export type ReadinessStatus =
  | "TRAINING_READY" | "CANDIDATE_READY" | "SHADOW_READY"
  | "PRODUCTION_READY" | "NOT_READY";

export type ReadinessCheck = { name: string; passed: boolean; detail: string };

export type ModelReadiness = {
  family: string;
  status: ReadinessStatus;
  checks: ReadinessCheck[];
  blockers: string[];
  evidence: Record<string, unknown>;
};

export type ReviewImage = {
  image_id: string;
  width: number;
  height: number;
  png_base64: string;
};

export type ReviewApplyResult = { record: ReviewRecord; dataset_updated: boolean };

export type TriggerEvent = {
  trigger_type: string;
  reason: string;
  evidence: unknown;
  dataset_fingerprint: string;
  timestamp_ms: number;
};

export type HermesTasks = {
  triggers: TriggerEvent[];
  readiness: ModelReadiness[];
  history_tail: HistoryEntry[];
};
