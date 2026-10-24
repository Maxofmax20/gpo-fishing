/**
 * FISH LABEL REVIEW
 *
 * The shortest path from "the OCR probably read this correctly" to "a human said
 * yes, this is a Crimson Snapper".
 *
 * # Why one fish at a time
 *
 * The question a reviewer answers is not "is row 4,117 a Crimson Snapper?" - it is
 * "are these all Crimson Snapper?". Showing 261 screenshots of the same fish
 * together turns 261 judgements into one. So batches are grouped by fish, the
 * whole batch is one click, and the only per-tile action is flagging a screenshot
 * that clearly does not belong.
 *
 * # Keys, and nothing else
 *
 *   Enter   confirm this fish
 *   N       skip to the next fish
 *   1-9     jump to that fish
 *   X       flag the highlighted screenshot as wrong
 *
 * # What a confirmation does, and does not do
 *
 * It writes a real human review through the same store every other review path
 * uses. It does not make a row training-eligible on its own: 15 of these 16 fish do
 * not exist in the knowledge base yet, and the screen says so before you click,
 * because a reviewer who believed they had unlocked training would be wrong.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LABELS_API_BASE, fetchImageDataUrl } from "../lib/ontologyBrowser";
import { Button, Pill } from "../components/primitives";
import { navigate } from "../lib/router";

type BatchSummary = {
  entityId: string;
  name: string;
  rowCount: number;
  sessionCount: number;
  avgConfidence: number;
  minConfidence: number;
  inKnowledgeBase: boolean;
  storedLabelConflicts: number;
};

type BatchIndex = {
  available: boolean;
  detail: string;
  batchCount: number;
  totalRows: number;
  rowsMissingFromKb: string[];
  batches: BatchSummary[];
};

type BatchRow = {
  imageId: string;
  confidence: number;
  sessionId: string;
  ocrText: string;
  storedEntityId: string | null;
  storedConflicts: boolean;
};

type BatchDetail = BatchSummary & { rows: BatchRow[] };

/**
 * Tiles per row at the widest breakpoint (`lg:grid-cols-8`).
 *
 * PageUp/PageDown move by this many so one keypress crosses exactly one screenful,
 * which is what a reviewer moving through 261 screenshots actually wants.
 */
const TILES_PER_PAGE = 24;

/** Nudge the scroller so the highlighted tile stays visible after a keypress. */
function scrollCursorIntoView(dir: 1 | -1) {
  window.requestAnimationFrame(() => {
    const el = document.querySelector<HTMLElement>("[data-cursor='1']");
    el?.scrollIntoView({ block: dir === 1 ? "end" : "start", behavior: "smooth" });
  });
}

type ConfirmResult = {
  ok: boolean;
  confirmed: number;
  skipped: number;
  recordedPendingEntity: number;
  entityId: string;
  inKnowledgeBase: boolean;
  message: string;
};

const thumbCache = new Map<string, string>();
const thumbRequests = new Map<string, Promise<string | null>>();
const THUMB_CACHE_LIMIT = 96;
function loadThumb(imageId: string): Promise<string | null> {
  const cached = thumbCache.get(imageId);
  if (cached) { thumbCache.delete(imageId); thumbCache.set(imageId, cached); return Promise.resolve(cached); }
  const active = thumbRequests.get(imageId);
  if (active) return active;
  const pending = fetchImageDataUrl(`${LABELS_API_BASE}/thumb/${encodeURIComponent(imageId)}`)
    .then((url) => {
      if (url) thumbCache.set(imageId, url);
      while (thumbCache.size > THUMB_CACHE_LIMIT) thumbCache.delete(thumbCache.keys().next().value!);
      return url;
    }).finally(() => { thumbRequests.delete(imageId); });
  thumbRequests.set(imageId, pending);
  return pending;
}

function useThumb(imageId: string) {
  const [src, setSrc] = useState<string | null>(() => thumbCache.get(imageId) ?? null);
  useEffect(() => {
    let live = true;
    if (thumbCache.has(imageId)) {
      setSrc(thumbCache.get(imageId) ?? null);
      return;
    }
    loadThumb(imageId)
      .then((u) => {
        if (live) setSrc(u);
      })
      .catch(() => {
        if (live) setSrc(null);
      });
    return () => {
      live = false;
    };
  }, [imageId]);
  return src;
}

function Tile({
  row,
  flagged,
  onFlag,
  selected,
}: {
  row: BatchRow;
  flagged: boolean;
  onFlag: () => void;
  selected: boolean;
}) {
  const src = useThumb(row.imageId);
  return (
    <button
      type="button"
      onClick={onFlag}
      // Lets the key handler find the highlighted tile and keep it on screen.
      data-cursor={selected ? "1" : "0"}
      title={`${row.ocrText} — ${(row.confidence * 100).toFixed(0)}%${flagged ? " — flagged wrong" : ""}`}
      className={`relative rounded border text-left focus:outline-none focus:ring-1 focus:ring-accent ${
        flagged
          ? "border-bad opacity-50"
          : selected
            ? "border-accent"
            : "border-line/60 hover:border-accent/60"
      }`}
    >
      {src ? (
        <img src={src} alt={row.ocrText} loading="lazy" className="aspect-[589/199] w-full rounded object-contain" />
      ) : (
        <div className="aspect-[589/199] rounded bg-black/40 animate-pulse" />
      )}
      <div className="flex items-center justify-between px-1 py-0.5 text-[9px]">
        <span className={row.storedConflicts ? "text-warn" : "text-fg-mute"}>
          {flagged ? "WRONG" : `${Math.round(row.confidence * 100)}%`}
        </span>
        {row.storedEntityId && (
          <span className="text-fg-mute truncate max-w-[50%]">
            {row.storedEntityId.replace("fish:", "")}
          </span>
        )}
      </div>
    </button>
  );
}

export function LabelReview() {
  const [index, setIndex] = useState<BatchIndex | null>(null);
  const [pos, setPos] = useState(0);
  const [detail, setDetail] = useState<BatchDetail | null>(null);
  const [flagged, setFlagged] = useState<Set<string>>(new Set());
  const [cursor, setCursor] = useState(0);
  const [done, setDone] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** Focus returns here after every confirm, so the keyboard flow never breaks. */
  const confirmRef = useRef<HTMLDivElement>(null);

  /** Latest index, readable from a key handler without re-subscribing. */
  const indexRef = useRef<BatchIndex | null>(null);

  const load = useCallback(async () => {
    try {
      const r = await fetch(`${LABELS_API_BASE}/batches`, { credentials: "omit" });
      const j: BatchIndex = await r.json();
      indexRef.current = j;
      setIndex(j);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const current = index?.batches[pos] ?? null;

  useEffect(() => {
    if (!current) {
      setDetail(null);
      return;
    }
    let live = true;
    setFlagged(new Set());
    setCursor(0);
    setMessage(null);
    fetch(`${LABELS_API_BASE}/batch/${encodeURIComponent(current.entityId)}`, {
      credentials: "omit",
    })
      .then((r) => r.json())
      .then((d) => live && setDetail(d))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [current?.entityId]);

  const confirmedRows = useMemo(
    () => Object.values(done).reduce((a, b) => a + b, 0),
    [done],
  );

  const confirm = useCallback(async () => {
    if (!current || !detail || busy) return;
    setBusy(true);
    setError(null);
    // Flagged tiles are excluded: a reviewer who flagged a screenshot is saying
    // "this one is not that fish", and confirming it anyway would be the tool
    // overruling the human.
    const ids = detail.rows
      .filter((r) => !flagged.has(r.imageId))
      .map((r) => r.imageId);
    if (ids.length === 0) {
      setMessage("Nothing left to confirm - every screenshot is flagged.");
      setBusy(false);
      return;
    }
    try {
      const res = await fetch(`${LABELS_API_BASE}/confirm`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        credentials: "omit",
        body: JSON.stringify({ entityId: current.entityId, imageIds: ids }),
      });
      const r: ConfirmResult = await res.json();
      if (!r.ok) {
        setError(r.message);
      } else {
        setDone((d) => ({ ...d, [current.entityId]: (d[current.entityId] ?? 0) + r.confirmed }));
        setMessage(r.message);
        const next = Math.min(pos + 1, (index?.batches.length ?? 1) - 1);
        setPos(next);
        void load();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      window.setTimeout(() => confirmRef.current?.focus(), 0);
    }
  }, [current, detail, busy, flagged, pos, index, load]);

  const skip = useCallback(() => {
    if (!index) return;
    setPos((p) => Math.min(p + 1, index.batches.length - 1));
    setMessage(null);
  }, [index]);

  const flagAt = useCallback(
    (i: number) => {
      setFlagged((f) => {
        const n = new Set(f);
        const row = detail?.rows[i];
        if (!row) return n;
        if (n.has(row.imageId)) n.delete(row.imageId);
        else n.add(row.imageId);
        return n;
      });
      setCursor((prev) => Math.min(prev + 1, (detail?.rows.length ?? 1) - 1));
      scrollCursorIntoView(1);
    },
    [detail],
  );

  // Keyboard only. No modifier combo may fire a confirmation: Ctrl+Enter would
  // otherwise be an accidental one.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      const t = ev.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
      if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
      if (ev.key === "Enter") {
        ev.preventDefault();
        void confirm();
      } else if (ev.key === "n" || ev.key === "N") {
        ev.preventDefault();
        skip();
      } else if (ev.key === " " || ev.key === "x" || ev.key === "X") {
        // Space is the natural "mark this one" key and works without reaching for
        // X. Since it is claimed here it no longer scrolls, so PageUp/PageDown and
        // the mouse wheel are provided below - both of which the reviewer needs
        // anyway to move through a 261-tile grid.
        ev.preventDefault();
        flagAt(cursor);
      } else if (ev.key === "ArrowRight") {
        ev.preventDefault();
        setCursor((prev) => Math.min(prev + 1, (detail?.rows.length ?? 1) - 1));
        scrollCursorIntoView(1);
      } else if (ev.key === "ArrowLeft") {
        ev.preventDefault();
        setCursor((prev) => Math.max(prev - 1, 0));
        scrollCursorIntoView(-1);
      } else if (ev.key === "PageDown") {
        ev.preventDefault();
        setCursor((prev) => Math.min(prev + TILES_PER_PAGE, (detail?.rows.length ?? 1) - 1));
        scrollCursorIntoView(+1);
      } else if (ev.key === "PageUp") {
        ev.preventDefault();
        setCursor((prev) => Math.max(prev - TILES_PER_PAGE, 0));
        scrollCursorIntoView(-1);
      } else if (/^[1-9]$/.test(ev.key)) {
        ev.preventDefault();
        setPos((p) => {
          const i = Number(ev.key) - 1;
          const idx = indexRef.current;
          if (idx && i < idx.batches.length) return i;
          return p;
        });
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        navigate("/ontology-review");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [confirm, skip, flagAt, cursor, detail]);

  if (error && !index) {
    return (
      <div className="p-6 text-[12px] text-bad">
        Could not load the review batches: {error}
      </div>
    );
  }
  if (!index) {
    return <div className="p-6 text-[12px] text-fg-mute">Loading…</div>;
  }
  if (!index.available) {
    return (
      <div className="p-6 text-[12px] text-fg-dim flex flex-col gap-2">
        <div>{index.detail}</div>
        <pre className="bg-black/50 border border-line/60 rounded p-2">
          python -m ml.gpo_train candidates
        </pre>
      </div>
    );
  }

  const goodRows = detail ? detail.rows.length - flagged.size : 0;

  return (
    <div className="flex flex-col gap-2.5">
      {/* ---- header ---- */}
      <div className="flex items-center gap-2 flex-wrap text-[11px]">
        <span className="text-[15px] font-semibold">Fish label review</span>
        <Pill tone="mute">{confirmedRows} / {index.totalRows} confirmed</Pill>
        <Pill tone={index.batches.length - Object.keys(done).length > 0 ? "warn" : "ok"}>
          {index.batches.length - Object.keys(done).length} fish left
        </Pill>
        <span className="flex-1" />
        <span className="text-[10px] text-fg-mute">
          <K>Enter</K> confirm · <K>N</K> skip · <K>1</K>–<K>9</K> jump ·{" "}
          <K>Space</K>/<K>X</K> flag · <K>↑↓←→</K>/<K>PgUp</K> scroll
        </span>
      </div>

      {/* ---- the KB caveat, before anything is clicked ---- */}
      {index.rowsMissingFromKb.length > 0 && (
        <div className="text-[10px] text-fg-dim border border-line/60 rounded px-2 py-1.5">
          <span className="text-warn">{index.rowsMissingFromKb.length} of these fish do not exist in the
          knowledge base yet</span> (only <code className="font-mono">fish:swordfish</code> does).
          Confirming them still records your judgement, but the rows stay
          <span className="font-semibold"> ineligible for training</span> until the proposed KB patch is
          applied by hand. Nothing on this page edits the knowledge base.
        </div>
      )}

      {/* ---- fish selector ---- */}
      <div className="flex gap-1 flex-wrap">
        {index.batches.map((b, i) => (
          <button
            key={b.entityId}
            type="button"
            onClick={() => setPos(i)}
            className={`px-2 py-1 rounded text-[10px] border focus:outline-none focus:ring-1 focus:ring-accent ${
              i === pos ? "border-accent bg-accent/15" : "border-line/60 hover:border-accent/50"
            } ${done[b.entityId] ? "opacity-50" : ""}`}
            title={`${b.name} — ${b.rowCount} rows, ${b.sessionCount} sessions, avg ${(b.avgConfidence * 100).toFixed(0)}%`}
          >
            {i < 9 ? <span className="text-fg-mute">{i + 1} </span> : null}
            {b.name}
            <span className="text-fg-mute"> {b.rowCount}</span>
            {done[b.entityId] ? <span className="text-ok"> ✓{done[b.entityId]}</span> : null}
          </button>
        ))}
      </div>

      {current && (
        <>
          {/* ---- the one judgement ---- */}
          <div className="flex items-baseline gap-3 flex-wrap">
            <span className="text-[18px] font-semibold">{current.name}</span>
            <span className="text-[11px] text-fg-dim">
              {goodRows} of {detail?.rows.length ?? current.rowCount} screenshots
              {flagged.size > 0 && (
                <span className="text-bad"> · {flagged.size} flagged wrong</span>
              )}
            </span>
            <span className="text-[10px] text-fg-mute">
              {current.sessionCount} sessions · confidence {Math.round(current.minConfidence * 100)}–
              {Math.round(current.avgConfidence * 100)}%
              {current.storedLabelConflicts > 0 && (
                <span className="text-warn">
                  {" "}
                  · {current.storedLabelConflicts} differ from the stored label
                </span>
              )}
            </span>
            <span className="flex-1" />
            {!current.inKnowledgeBase && (
              <Pill tone="warn">NOT IN KB — review kept, row stays ineligible</Pill>
            )}
          </div>

          <div className="grid grid-cols-3 sm:grid-cols-4 md:grid-cols-6 lg:grid-cols-8 gap-1">
            {detail?.rows.map((r, i) => (
              <Tile
                key={r.imageId}
                row={r}
                flagged={flagged.has(r.imageId)}
                selected={i === cursor}
                onFlag={() => flagAt(i)}
              />
            ))}
          </div>

          {/* ---- two actions ---- */}
          <div ref={confirmRef} tabIndex={-1} className="flex items-center gap-2 pt-1 focus:outline-none">
            <Button
              kind="primary"
              onClick={() => void confirm()}
              disabled={busy || goodRows === 0}
              title="Record a human review for every screenshot in this batch (Enter)"
            >
              {busy ? "Saving…" : `Confirm these ${goodRows} as ${current.name}`}
            </Button>
            <Button kind="ghost" onClick={skip} title="Move to the next fish (N)">
              Skip (N)
            </Button>
            {flagged.size > 0 && (
              <Button kind="ghost" onClick={() => setFlagged(new Set())}>
                Unflag all
              </Button>
            )}
            <span className="flex-1" />
            <Button kind="ghost" onClick={() => navigate("/ontology-review")}>
              Ontology review
            </Button>
          </div>
        </>
      )}

      {message && (
        <div className="text-[11px] text-ok border border-ok/40 rounded px-2 py-1">{message}</div>
      )}
      {error && (
        <div className="text-[11px] text-bad border border-bad/40 rounded px-2 py-1">
          {error}
          <Button size="sm" kind="ghost" onClick={() => setError(null)}>
            Dismiss
          </Button>
        </div>
      )}
    </div>
  );
}

function K({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="border border-line/70 rounded px-1 text-[9px] font-mono bg-black/40">
      {children}
    </kbd>
  );
}

export default LabelReview;