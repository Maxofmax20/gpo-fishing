import { X } from "lucide-react";
import { useStore } from "../lib/store";
import { cx } from "./primitives";

/** Global toast stack for backend/UI errors. Render once inside Panel. */
export default function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none absolute inset-x-3 bottom-3 z-50 flex flex-col gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={cx(
            "pointer-events-auto flex items-start gap-2 rounded-xl border px-3 py-2 text-[12px] shadow-lg backdrop-blur",
            t.kind === "error" && "border-bad/50 bg-bad-soft text-bad",
            t.kind === "warn" && "border-warn/50 bg-warn-soft text-warn",
            t.kind === "info" && "border-line bg-black/70 text-fg-dim",
          )}
        >
          <div className="flex-1 min-w-0 break-words select-text">{t.msg}</div>
          <button
            className="shrink-0 opacity-70 hover:opacity-100"
            onClick={() => dismiss(t.id)}
            title="Dismiss"
          >
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
