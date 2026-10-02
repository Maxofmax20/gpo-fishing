import { useEffect, useRef } from "react";

/**
 * Interval polling that pauses while the window is hidden and refreshes on
 * return to visibility. Saves CPU/wakeups for background tabs; never gates
 * time-critical bot behavior (bot runs in the Rust backend regardless).
 */
export function useVisiblePoll(fn: () => void, ms: number, active = true) {
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => {
    if (!active) return;
    ref.current();
    const tick = () => {
      if (!document.hidden) ref.current();
    };
    const t = window.setInterval(tick, ms);
    const onVis = () => {
      if (!document.hidden) ref.current();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.clearInterval(t);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [ms, active]);
}
