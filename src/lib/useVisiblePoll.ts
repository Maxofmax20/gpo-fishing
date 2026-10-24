import { useEffect, useRef } from "react";

/**
 * Interval polling that pauses while the window is hidden and refreshes on
 * return to visibility. Saves CPU/wakeups for background tabs; never gates
 * time-critical bot behavior (bot runs in the Rust backend regardless).
 */
export function useVisiblePoll(fn: () => void | Promise<unknown>, ms: number, active = true) {
  const ref = useRef(fn);
  ref.current = fn;
  const flight = useRef(false);
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    const tick = async () => {
      if (disposed || document.hidden || flight.current) return;
      flight.current = true;
      try { await ref.current(); } catch (e) { console.error("Polling failed", e); }
      finally { flight.current = false; }
    };
    // Hidden windows have no interval at all, rather than waking just to skip.
    let t: number | undefined;
    const onVis = () => {
      window.clearInterval(t);
      t = undefined;
      if (!document.hidden) {
        void tick();
        t = window.setInterval(tick, ms);
      }
    };
    onVis();
    document.addEventListener("visibilitychange", onVis);
    return () => {
      disposed = true;
      window.clearInterval(t);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [ms, active]);
}
