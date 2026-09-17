import { useState } from "react";
import { Send, Camera, PencilRuler, Sparkles } from "lucide-react";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { PointField } from "../components/PointField";
import { Button, cx, Kbd, KeyCapture, Pill, Row, Section, Segmented, Slider, Step, Steps, Stepper, TextField, Toggle } from "../components/primitives";

export default function Features() {
  const s = useStore((st) => st.settings);
  const roblox = useStore((st) => st.roblox);
  const update = useStore((st) => st.update);
  const ocrAvailable = useStore((st) => st.ocrAvailable);
  const [open, setOpen] = useState<string | null>(null);
  const [whTest, setWhTest] = useState<"idle" | "ok" | string>("idle");
  if (!s) return null;
  const toggle = (k: string) => setOpen((o) => (o === k ? null : k));
  const baitReady = !!s.points.bait[0];
  const shopReady = !!s.points.purchase[0] && !!s.points.purchase[1];
  const fruitReady = !!s.points.fruit[0];

  const [baitScanState, setBaitScanState] = useState<{
    loading: boolean;
    result: { legendary: number | null; rare: number | null; common: number | null } | null;
    error: string | null;
  }>({ loading: false, result: null, error: null });

  const [geminiTesting, setGeminiTesting] = useState(false);
  const [geminiTestMsg, setGeminiTestMsg] = useState<{ ok: boolean; text: string } | null>(null);

  const handleTestGemini = async () => {
    if (!s?.gemini?.api_key) {
      setGeminiTestMsg({ ok: false, text: "Please enter your Gemini API key first" });
      return;
    }
    setGeminiTesting(true);
    setGeminiTestMsg(null);
    try {
      const res = await api.testGemini(s.gemini.api_key, s.gemini.model || "gemini-3.5-flash-lite");
      setGeminiTestMsg({ ok: true, text: res });
    } catch (err) {
      setGeminiTestMsg({ ok: false, text: String(err) });
    } finally {
      setGeminiTesting(false);
    }
  };

  const handleScanBait = async () => {
    setBaitScanState({ loading: true, result: null, error: null });
    try {
      const stock = await api.scanBaitStock();
      setBaitScanState({ loading: false, result: stock, error: null });
    } catch (e) {
      setBaitScanState({ loading: false, result: null, error: String(e) });
    }
  };

  const testWebhook = async () => {
    try {
      await api.webhookTest();
      setWhTest("ok");
    } catch (e) {
      setWhTest(String(e));
    }
  };

  return (
    <div className="pb-4 pt-2">
      <Section title="Fishing">
        <Row
          title="Auto zoom"
          sub="Resets the camera to a known zoom before fishing so the bar lands in the same place."
          right={<Toggle value={s.features.auto_zoom} onChange={(v) => { update((x) => void (x.features.auto_zoom = v)); if (v) setOpen("zoom"); }} />}
          open={open === "zoom"}
          onToggle={() => toggle("zoom")}
        >
          <Field label="Zoom out steps">
            <Stepper value={s.zoom.out_steps} min={1} max={30} onChange={(v) => update((x) => void (x.zoom.out_steps = v))} />
          </Field>
          <Field label="Zoom in steps">
            <Stepper value={s.zoom.in_steps} min={0} max={30} onChange={(v) => update((x) => void (x.zoom.in_steps = v))} />
          </Field>
          <Field label="Step delay">
            <Slider value={s.zoom.step_delay_ms} min={20} max={400} step={10} format={(v) => `${v} ms`} onChange={(v) => update((x) => void (x.zoom.step_delay_ms = v))} />
          </Field>
        </Row>
        <Row
          title="Auto bait"
          sub="Re-selects your bait before every cast so the rod never fishes empty."
          right={
            <>
              {s.features.auto_bait && !s.features.smart_bait && !baitReady && <Pill tone="warn">point needed</Pill>}
              {s.features.auto_bait && s.features.smart_bait && (
                <Pill tone="accent">{s.gemini?.enabled ? "Gemini AI" : "Neural AI"}</Pill>
              )}
              <Toggle value={s.features.auto_bait} onChange={(v) => { update((x) => void (x.features.auto_bait = v)); if (v) setOpen("bait"); }} />
            </>
          }
          open={open === "bait"}
          onToggle={() => toggle("bait")}
        >
          <div className="mb-3 pb-3 border-b border-line flex flex-col gap-2.5">
            <div className="flex items-center justify-between">
              <div>
                <div className="text-[12px] font-medium text-fg">Smart Bait (Neural / Gemini Stock Tracking)</div>
                <div className="text-[11px] text-fg-mute">
                  Uses AI to read bait quantities directly from the bait menu and auto-selects your preferred tier.
                </div>
              </div>
              <Toggle
                value={s.features.smart_bait}
                onChange={(v) => update((x) => void (x.features.smart_bait = v))}
              />
            </div>

            {s.features.smart_bait && (
              <div className="p-2.5 rounded bg-bg-card/70 border border-line space-y-3">
                <div>
                  <div className="text-[11px] font-medium text-fg-dim mb-1">Target Bait Tier</div>
                  <Segmented
                    value={s.purchase.bait_tier ?? "common"}
                    onChange={(v) => update((x) => void (x.purchase.bait_tier = v as any))}
                    options={[
                      { value: "common", label: "Common" },
                      { value: "rare", label: "Rare" },
                      { value: "legendary", label: "Legendary" },
                      { value: "highest", label: "Highest" },
                    ]}
                  />
                  <div className="text-[10px] text-fg-mute mt-1">
                    {s.purchase.bait_tier === "legendary" && "Uses Legendary bait. Automatically falls back to Rare then Common when empty."}
                    {s.purchase.bait_tier === "rare" && "Uses Rare bait. Automatically falls back to Common when empty."}
                    {s.purchase.bait_tier === "common" && "Always uses standard Common bait."}
                    {s.purchase.bait_tier === "highest" && "Always uses the highest tier bait currently in inventory (Legendary > Rare > Common)."}
                  </div>
                </div>

                <div className="pt-2 border-t border-line/60">
                  <div className="flex items-center justify-between mb-1.5">
                    <div>
                      <div className="text-[11px] font-medium text-fg flex items-center gap-1.5">
                        <Sparkles size={13} className="text-accent" />
                        <span>Google Gemini Vision AI (Cloud)</span>
                      </div>
                      <div className="text-[10px] text-fg-mute">
                        Use Gemini 3.5 Flash Lite or 2.5 Flash for cloud vision with automatic local neural fallback.
                      </div>
                    </div>
                    <Toggle
                      value={s.gemini?.enabled ?? false}
                      onChange={(v) =>
                        update((x) => {
                          if (!x.gemini) {
                            x.gemini = { enabled: v, api_key: "", model: "gemini-3.5-flash-lite" };
                          } else {
                            x.gemini.enabled = v;
                          }
                        })
                      }
                    />
                  </div>

                  {s.gemini?.enabled && (
                    <div className="space-y-2 mt-2 pt-2 border-t border-line/40">
                      <div>
                        <div className="text-[10px] font-medium text-fg-dim mb-1">Gemini Model</div>
                        <Segmented
                          value={s.gemini.model || "gemini-3.5-flash-lite"}
                          onChange={(v) => update((x) => void (x.gemini.model = v))}
                          options={[
                            { value: "gemini-3.5-flash-lite", label: "3.5 Flash Lite" },
                            { value: "gemini-2.5-flash", label: "2.5 Flash" },
                            { value: "gemini-3.5-flash", label: "3.5 Flash" },
                            { value: "gemini-2.5-pro", label: "2.5 Pro" },
                          ]}
                        />
                      </div>

                      <div>
                        <div className="text-[10px] font-medium text-fg-dim mb-1">Gemini API Key</div>
                        <div className="flex items-center gap-2">
                          <TextField
                            type="password"
                            placeholder="AQ... or AIza..."
                            value={s.gemini.api_key || ""}
                            onChange={(v) => update((x) => void (x.gemini.api_key = v))}
                          />
                          <Button
                            size="sm"
                            onClick={handleTestGemini}
                            disabled={geminiTesting || !s.gemini.api_key}
                          >
                            {geminiTesting ? "Testing..." : "Test Key"}
                          </Button>
                        </div>
                        {geminiTestMsg && (
                          <div className={cx("text-[10px] mt-1 font-mono", geminiTestMsg.ok ? "text-accent" : "text-err")}>
                            {geminiTestMsg.text}
                          </div>
                        )}
                      </div>
                    </div>
                  )}
                </div>

                <div className="flex items-center justify-between pt-1 border-t border-line/60">
                  <div className="flex items-center gap-2">
                    <Button
                      size="sm"
                      onClick={handleScanBait}
                      disabled={baitScanState.loading || !roblox}
                      icon={<Camera size={13} />}
                    >
                      {baitScanState.loading ? "Scanning..." : (s.gemini?.enabled ? "Test Gemini scan" : "Test AI scan")}
                    </Button>
                    <Button
                      size="sm"
                      onClick={() => api.overlayOpenRegions()}
                      disabled={!roblox}
                      icon={<PencilRuler size={13} />}
                    >
                      Edit Area
                    </Button>
                  </div>
                  {baitScanState.result && (
                    <div className="text-[11px] font-mono text-fg-dim flex items-center gap-2">
                      <span>🌟 {baitScanState.result.legendary ?? "?"}</span>
                      <span>🔷 {baitScanState.result.rare ?? "?"}</span>
                      <span>⚪ {baitScanState.result.common ?? "?"}</span>
                    </div>
                  )}
                  {baitScanState.error && (
                    <div className="text-[11px] text-err">{baitScanState.error}</div>
                  )}
                </div>
              </div>
            )}
          </div>

          {!s.features.smart_bait && (
            <Steps>
              <Step n={1} title={<>Press <Kbd>{s.keys.rod.toUpperCase()}</Kbd> to open the rod menu</>} sub="The rod key is set in Setup › Inventory keys." done />
              <Step n={2} title="Click the bait" sub="Pick the top bait in the rod menu." done={baitReady}>
                <PointField target="bait1" value={s.points.bait[0]} />
              </Step>
              <Step n={3} title="Backup click" sub="Optional. If set, the bot clicks here, then the bait again." done={!!s.points.bait[1]} last>
                <PointField target="bait2" value={s.points.bait[1]} clearable onCleared={() => update((x) => void (x.points.bait[1] = null))} />
              </Step>
            </Steps>
          )}

          <div className="mt-3 pt-3 border-t border-line flex items-center justify-between">
            <div>
              <div className="text-[12px] font-medium text-fg">Zero-bait failsafe</div>
              <div className="text-[11px] text-fg-mute">Safely pause macro and send Telegram/Discord alert if bait runs out.</div>
            </div>
            <Toggle
              value={s.features.zero_bait_failsafe ?? true}
              onChange={(v) => update((x) => void (x.features.zero_bait_failsafe = v))}
            />
          </div>
        </Row>
        <Row
          title="Fast reset (Animation cancel)"
          sub="Swaps hotbar slots upon catching a fish to cancel the celebration animation, maximizing fishing time."
          right={
            <Toggle
              value={s.features.fast_reset ?? true}
              onChange={(v) => {
                update((x) => void (x.features.fast_reset = v));
                if (v) setOpen("fast_reset");
              }}
            />
          }
          open={open === "fast_reset"}
          onToggle={() => toggle("fast_reset")}
        >
          <div className="space-y-3">
            <div className="text-[12px] text-fg-dim">
              ⚡ When a fish is caught, the macro immediately taps your swap slot key and re-equips the rod. This skips 2–4 seconds of fish-holding animation delay so you recast instantly.
            </div>
            <div className="flex items-center justify-between pt-2 border-t border-line/60">
              <div>
                <div className="text-[12px] font-medium text-fg">Swap slot key</div>
                <div className="text-[11px] text-fg-mute">Hotbar slot to swap to (default: 2). Keep empty or with a harmless tool.</div>
              </div>
              <KeyCapture
                single
                value={s.keys.reset_slot ?? "2"}
                onChange={(v) => update((x) => void (x.keys.reset_slot = v))}
              />
            </div>
          </div>
        </Row>
        <Row
          title="Auto buy bait"
          sub="Stand next to the bait barrel on the dock. Runs once at start and again every N catches."
          right={
            <>
              {s.features.auto_purchase && !shopReady && <Pill tone="warn">points needed</Pill>}
              <Toggle value={s.features.auto_purchase} onChange={(v) => { update((x) => void (x.features.auto_purchase = v)); if (v) setOpen("buy"); }} />
            </>
          }
          open={open === "buy"}
          onToggle={() => toggle("buy")}
        >
          <Steps>
            <Step n={1} title="Hold the shop key" sub="Held down next to the barrel until its shop dialog opens, like holding it yourself." done>
              <KeyCapture single value={s.keys.shop} onChange={(v) => update((x) => void (x.keys.shop = v))} />
              <Slider value={s.purchase.hold_shop_key_ms} min={500} max={5000} step={100} width={150} format={(v) => `${(v / 1000).toFixed(1)} s`} onChange={(v) => update((x) => void (x.purchase.hold_shop_key_ms = v))} />
            </Step>
            <Step n={2} title="Click the Confirm button" sub="Pick the button that confirms the purchase." done={!!s.points.purchase[0]}>
              <PointField target="purchase1" value={s.points.purchase[0]} clearable onCleared={() => update((x) => void (x.points.purchase[0] = null))} />
            </Step>
            <Step n={3} title="Click the quantity box and type the amount" sub="Pick the number field in the shop." done={!!s.points.purchase[1]}>
              <PointField target="purchase2" value={s.points.purchase[1]} clearable onCleared={() => update((x) => void (x.points.purchase[1] = null))} />
              <Stepper value={s.purchase.amount} min={1} max={9999} step={10} suffix="bait" onChange={(v) => update((x) => void (x.purchase.amount = v))} />
            </Step>
            <Step n={4} title="Click the Buy button" sub="Pick the button that executes the purchase." done={!!s.points.purchase[2]}>
              <PointField target="purchase3" value={s.points.purchase[2]} clearable onCleared={() => update((x) => void (x.points.purchase[2] = null))} />
            </Step>
            <Step n={5} title="Click the Middle button (OK / Close)" sub="The final button in the middle clicked after buy to dismiss the prompt." done={!!s.points.purchase[3]}>
              <PointField target="purchase4" value={s.points.purchase[3]} clearable onCleared={() => update((x) => void (x.points.purchase[3] = null))} />
            </Step>
            <Step n={6} title="Back to fishing" sub="Closes dialog and returns to fishing." done last />
          </Steps>
          <div className="mt-4 pt-3 border-t border-line">
            <Field label="Dynamic buy threshold (Smart Bait)">
              <Stepper
                value={s.purchase.low_bait_threshold ?? 5}
                min={0}
                max={50}
                suffix="common bait"
                onChange={(v) => update((x) => void (x.purchase.low_bait_threshold = v))}
              />
            </Field>
            <div className="text-[11px] text-fg-mute pb-2">
              💡 Only Common bait can be bought from the shop. When Common bait drops to or below this amount, macro immediately purchases from shop.
            </div>
            <Field label="Max bait capacity">
              <Stepper
                value={s.purchase.max_bait ?? 300}
                min={1}
                max={300}
                suffix="max bait"
                onChange={(v) => update((x) => void (x.purchase.max_bait = v))}
              />
            </Field>
            <div className="text-[11px] text-fg-mute pb-2">
              📦 Macro calculates missing bait (Max - Current) and buys the exact amount up to this limit (max inventory is 300).
            </div>
            <Field label="Buy every">
              <Stepper value={s.purchase.every_n_catches} min={1} max={500} suffix="fish" onChange={(v) => update((x) => void (x.purchase.every_n_catches = v))} />
            </Field>
            <div className="text-[11px] text-fg-mute pb-2">
              🛒 Periodic restock backup: Restocks bait from merchant every {s.purchase.every_n_catches} fish.
            </div>
            <Field label="Pause between clicks">
              <Slider value={s.purchase.click_delay_ms} min={200} max={3000} step={50} format={(v) => `${v} ms`} onChange={(v) => update((x) => void (x.purchase.click_delay_ms = v))} />
            </Field>
          </div>
        </Row>
      </Section>

      <Section title="Devil fruits">
        <Row
          title="Store fruits"
          sub="After a fruit drop, moves it to your fruit slots and re-equips the rod."
          right={
            <>
              {s.features.fruit_storage && !fruitReady && <Pill tone="warn">point needed</Pill>}
              <Toggle disabled={!ocrAvailable} value={s.features.fruit_storage} onChange={(v) => { update((x) => void (x.features.fruit_storage = v)); if (v) setOpen("store"); }} />
            </>
          }
          open={open === "store"}
          onToggle={() => toggle("store")}
          disabled={!ocrAvailable}
        >
          <Steps>
            <Step n={1} title="Fruit slot keys" sub="The two hotbar slots the fruit is moved through. Keep them empty." done>
              <KeyCapture single value={s.keys.fruit_slot_1} onChange={(v) => update((x) => void (x.keys.fruit_slot_1 = v))} />
              <KeyCapture single value={s.keys.fruit_slot_2} onChange={(v) => update((x) => void (x.keys.fruit_slot_2 = v))} />
            </Step>
            <Step n={2} title="Click the Store button" sub="Pick where the Store button appears after switching to a fruit slot." done={fruitReady}>
              <PointField target="fruit1" value={s.points.fruit[0]} />
            </Step>
            <Step n={3} title="Backup click" sub="Optional. If set, the bot clicks here, then the Store button again." done={!!s.points.fruit[1]}>
              <PointField target="fruit2" value={s.points.fruit[1]} clearable onCleared={() => update((x) => void (x.points.fruit[1] = null))} />
            </Step>
            <Step n={4} title={<>Press <Kbd>{s.keys.rod.toUpperCase()}</Kbd> to re-equip the rod</>} sub="The rod key is set in Setup." done last />
          </Steps>
          <div className="mt-4 pt-3 border-t border-line space-y-1">
            <Field wide label="Never drop Legendary / Mythical">
              <Toggle
                value={s.fruit_storage.never_drop_legendary_or_mythical ?? true}
                onChange={(v) => update((x) => void (x.fruit_storage.never_drop_legendary_or_mythical = v))}
              />
            </Field>
            <div className="text-[11px] text-fg-dim pb-1.5 leading-normal">
              🛡️ Never presses Backspace on high-tier fruits (Tori, Mochi, Ope, Venom, Buddha, Dragon, Pika, Magu, Goro, etc.). Keeps them safely in your hotbar/inventory if storage fails or backpack is full.
            </div>
            <Field wide label="Pause macro on Legendary / Mythical">
              <Toggle
                value={s.fruit_storage.pause_on_protected_fruit ?? false}
                onChange={(v) => update((x) => void (x.fruit_storage.pause_on_protected_fruit = v))}
              />
            </Field>
            <div className="text-[11px] text-fg-dim pb-1.5 leading-normal">
              🚨 Automatically pauses fishing immediately after catching a protected fruit so you can safely inspect and store it.
            </div>
            <Field label="Dialog wait">
              <Slider value={s.fruit_storage.dialog_wait_ms} min={200} max={3000} step={50} format={(v) => `${v} ms`} onChange={(v) => update((x) => void (x.fruit_storage.dialog_wait_ms = v))} />
            </Field>
            <Field label="After drop">
              <Slider value={s.fruit_storage.after_drop_ms} min={200} max={4000} step={50} format={(v) => `${v} ms`} onChange={(v) => update((x) => void (x.fruit_storage.after_drop_ms = v))} />
            </Field>
          </div>
        </Row>
      </Section>

      <Section title="Notifications & Integrations">
        <Row
          title="Discord Rich Presence (RPC)"
          sub="Displays live GPO fishing activity, fish count, devil fruits, pity counter, and runtime on your Discord profile."
          right={
            <Toggle
              value={s.features.discord_rpc ?? true}
              onChange={(v) => update((x) => void (x.features.discord_rpc = v))}
            />
          }
        />
        <Row
          title="Alerts (Telegram & Discord)"
          sub="Sends real-time alerts to Telegram or Discord for caught fruits, world spawns, and progress."
          right={<Toggle value={s.webhook.enabled} onChange={(v) => { update((x) => void (x.webhook.enabled = v)); if (v) setOpen("wh"); }} />}
          open={open === "wh"}
          onToggle={() => toggle("wh")}
        >
          <div className="mb-3">
            <div className="text-[12px] text-fg-dim mb-1.5 font-medium">Notification Provider</div>
            <Segmented
              value={s.webhook.provider || "telegram"}
              options={[
                { value: "telegram", label: "Telegram" },
                { value: "discord", label: "Discord" },
                { value: "both", label: "Both" },
              ]}
              onChange={(v) => update((x) => void (x.webhook.provider = v as any))}
            />
          </div>

          {(s.webhook.provider === "telegram" || s.webhook.provider === "both" || !s.webhook.provider) && (
            <div className="mb-3 p-3 rounded-xl bg-black/20 border border-line flex flex-col gap-2.5">
              <div className="text-[12px] font-semibold text-fg flex items-center gap-1.5">
                Telegram Bot Settings
              </div>
              <div>
                <div className="text-[11px] text-fg-dim mb-1">Bot Token</div>
                <TextField
                  type="text"
                  mono
                  placeholder="123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ"
                  value={s.webhook.telegram_bot_token || ""}
                  onChange={(v) => update((x) => void (x.webhook.telegram_bot_token = v.trim()))}
                />
              </div>
              <div>
                <div className="text-[11px] text-fg-dim mb-1">Chat ID</div>
                <TextField
                  type="text"
                  mono
                  placeholder="123456789 or @channel"
                  value={s.webhook.telegram_chat_id || ""}
                  onChange={(v) => update((x) => void (x.webhook.telegram_chat_id = v.trim()))}
                />
              </div>
              <div className="text-[11px] text-fg-mute bg-white/[0.03] p-2.5 rounded-lg leading-relaxed">
                💡 <b>How to set up Telegram notifications:</b>
                <br />1. Message <b>@BotFather</b> on Telegram, send <span className="font-mono text-fg-dim">/newbot</span> and copy the <b>HTTP API Token</b>.
                <br />2. Message <b>@userinfobot</b> to get your numeric <b>Id</b>, then send <span className="font-mono text-fg-dim">/start</span> to your bot.
              </div>
              <div className="mt-2 pt-2 border-t border-line flex items-center justify-between">
                <div>
                  <div className="text-[12px] font-medium text-fg">Two-way remote control</div>
                  <div className="text-[11px] text-fg-mute">Control via Telegram (/status, /screenshot, /pity, /recast, /update, /stop, /start).</div>
                </div>
                <Toggle
                  value={s.features.telegram_remote ?? true}
                  onChange={(v) => update((x) => void (x.features.telegram_remote = v))}
                />
              </div>
            </div>
          )}

          {(s.webhook.provider === "discord" || s.webhook.provider === "both") && (
            <div className="mb-3 p-3 rounded-xl bg-black/20 border border-line flex flex-col gap-2.5">
              <div className="text-[12px] font-semibold text-fg">Discord Webhook</div>
              <TextField
                type="url"
                mono
                placeholder="https://discord.com/api/webhooks/…"
                value={s.webhook.url || ""}
                onChange={(v) => update((x) => void (x.webhook.url = v.trim()))}
              />
            </div>
          )}

          <div className="flex items-center gap-2 mb-3">
            <Button size="md" onClick={testWebhook} icon={<Send size={13} />}>
              Test delivery
            </Button>
            {whTest !== "idle" && (
              <div>{whTest === "ok" ? <Pill tone="ok">message delivered</Pill> : <Pill tone="bad">{whTest}</Pill>}</div>
            )}
          </div>

          <Field label="Progress every">
            <Stepper value={s.webhook.progress_every_n} min={1} max={500} suffix="fish" onChange={(v) => update((x) => void (x.webhook.progress_every_n = v))} />
          </Field>
          <div className="text-[11px] text-fg-mute pb-2">
            📱 Sends Telegram / Discord catch statistics update every {s.webhook.progress_every_n} fish. (Separated from bait restocking)
          </div>
          <div className="mt-2 border-t border-line">
            {(
              [
                ["fruit_drop", "Devil fruit caught"],
                ["send_screenshot", "📸 Send catch screenshot photo"],
                ["disconnect_alert", "⚠️ Roblox disconnected alert"],
                ["bait_alert", "🎣 Bait depleted alert"],
                ["progress", "Progress updates"],
                ["spawn", "World spawn (reads the drop message area)"],
                ["purchase", "Bait purchased"],
                ["recovery", "Recovery / stuck"],
              ] as const
            ).map(([k, label]) => (
              <div key={k} className="flex items-center h-10 border-b border-line">
                <div className="text-fg-dim">{label}</div>
                <div className="ml-auto">
                  <Toggle value={s.webhook[k]} disabled={(k === "fruit_drop" || k === "spawn") && !ocrAvailable} onChange={(v) => update((x) => void (x.webhook[k] = v))} />
                </div>
              </div>
            ))}
            {s.webhook.fruit_drop && ocrAvailable && (
              <div className="flex items-center h-10 border-b border-line pl-4">
                <div className="text-fg-dim">Legendary drops only</div>
                <div className="ml-auto">
                  <Toggle value={s.webhook.legendary_only} onChange={(v) => update((x) => void (x.webhook.legendary_only = v))} />
                </div>
              </div>
            )}
          </div>
          {s.webhook.spawn && ocrAvailable && (
            <div className="mt-3">
              <Field label="Spawn check every">
                <Slider value={s.ocr.spawn_check_interval_s} min={2} max={30} step={1} format={(v) => `${v} s`} onChange={(v) => update((x) => void (x.ocr.spawn_check_interval_s = v))} />
              </Field>
              <Field label="Spawn cooldown">
                <Slider value={s.ocr.spawn_cooldown_s / 60} min={1} max={30} step={1} format={(v) => `${v} min`} onChange={(v) => update((x) => void (x.ocr.spawn_cooldown_s = v * 60))} />
              </Field>
            </div>
          )}
        </Row>
      </Section>
    </div>
  );
}

function Field({ label, children, wide }: { label: string; children: React.ReactNode; wide?: boolean }) {
  return (
    <div className="flex items-center min-h-10 py-1">
      <div className={cx("text-fg-dim shrink-0", wide ? "flex-1 pr-3" : "w-32")}>{label}</div>
      <div className="ml-auto shrink-0">{children}</div>
    </div>
  );
}
