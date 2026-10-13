import { useRef, useState } from "react";
import { Pause, Play } from "lucide-react";
import { api } from "../lib/ipc";
import { useVisiblePoll } from "../lib/useVisiblePoll";
import { showToast, useStore } from "../lib/store";
import { Button } from "./primitives";

/** Recording can stop independently of fishing and local model inference. */
export function CollectionControl() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const working = useRef(false);
  const generation = useRef(0);
  useVisiblePoll(async () => {
    if (working.current) return;
    const current = generation.current;
    try {
      const status = await api.mlCollectionStatus();
      if (!working.current && current === generation.current) setEnabled(status.trace_on);
    } catch { if (!working.current && current === generation.current) setEnabled(null); }
  }, 3000);
  const change = async () => {
    if (enabled === null || working.current) return;
    working.current = true;
    generation.current++;
    setBusy(true);
    try {
      const settings = await api.mlCollectionSet(!enabled);
      setEnabled(!enabled);
      useStore.setState({ settings });
      showToast("info", enabled ? "Collection stopped. Fishing continues; samples kept." : "Collection enabled. Start fishing to save samples.");
    } catch (error) { showToast("error", `Collection: ${String(error)}`); }
    finally { working.current = false; setBusy(false); }
  };
  return <Button size="sm" kind={enabled ? "danger" : "primary"} disabled={busy || enabled === null}
    icon={enabled ? <Pause size={14} /> : <Play size={14} />} onClick={() => void change()}>
    {busy ? "Saving…" : enabled === null ? "Checking collection…" : enabled ? "Stop collecting data" : "Enable data collection"}
  </Button>;
}
