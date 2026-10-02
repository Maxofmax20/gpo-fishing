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
  if (!open) return null;
  return (
    <div
      className="absolute inset-0 z-50 grid place-items-center bg-black/60 backdrop-blur-sm px-6"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-[340px] rounded-2xl border border-line bg-[#0c101a] p-4 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="font-semibold text-[14px]">{title}</div>
        <div className="mt-1.5 text-[12px] text-fg-dim leading-relaxed">{body}</div>
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
