import { useEffect, useId, useRef } from "react";
import { Button } from "./primitives";

export default function ConfirmModal({
  open,
  title,
  body,
  backupNote,
  confirmLabel = "Confirm",
  danger = true,
  busy = false,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  body: string;
  backupNote?: string;
  confirmLabel?: string;
  danger?: boolean;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const dialog = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const bodyId = useId();
  const cancel = useRef(onCancel);
  const isBusy = useRef(busy);
  cancel.current = onCancel;
  isBusy.current = busy;
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const root = dialog.current;
    root?.querySelector<HTMLButtonElement>("button")?.focus();
    const keydown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        if (!isBusy.current) cancel.current();
      }
      if (e.key === "Tab") {
        const items = Array.from(root?.querySelectorAll<HTMLElement>('button:not(:disabled), [tabindex="0"]') ?? []);
        if (!items.length) { e.preventDefault(); root?.focus(); return; }
        const first = items[0], last = items[items.length - 1];
        if (e.shiftKey && (document.activeElement === first || !root?.contains(document.activeElement))) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && (document.activeElement === last || !root?.contains(document.activeElement))) { e.preventDefault(); first.focus(); }
      }
    };
    document.addEventListener("keydown", keydown, true);
    return () => { document.removeEventListener("keydown", keydown, true); previous?.focus(); };
  }, [open]);
  if (!open) return null;
  return (
    <div
      className="absolute inset-0 z-50 grid place-items-center bg-black/60 backdrop-blur-sm px-6"
      onClick={() => { if (!busy) onCancel(); }}
    >
      <div
        ref={dialog}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={bodyId}
        aria-busy={busy}
        tabIndex={-1}
        className="w-full max-w-[340px] rounded-2xl border border-line bg-[#0c101a] p-4 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div id={titleId} className="font-semibold text-[14px]">{title}</div>
        <div id={bodyId} className="mt-1.5 text-[12px] text-fg-dim leading-relaxed">{body}</div>
        {backupNote && <div className="mt-1.5 text-[11px] text-ok">{backupNote}</div>}
        <div className="mt-4 flex justify-end gap-2">
          <Button size="sm" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button size="sm" kind={danger ? "danger" : "primary"} onClick={onConfirm} disabled={busy}>
            {busy ? "Working…" : confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}
