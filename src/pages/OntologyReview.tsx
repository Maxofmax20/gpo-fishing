/**
 * ONTOLOGY REVIEW screen.
 *
 * The reviewer sees what the GAME appears to be naming, the evidence behind it,
 * and four decision shapes. Nothing is pre-selected.
 *
 * Why the screen shows evidence and refuses to record a decision:
 *
 * The current 8-class ontology is what is under review. If this screen also
 * wrote decisions, a reviewer could resolve a name by clicking the nearest
 * familiar-looking entity - which is precisely how the existing contamination
 * arose. So the screen's job is to make the decision EASY and well-evidenced, and
 * the decision itself is recorded deliberately, by a person, on the command line:
 *
 *     python -m ml.gpo_ontology_review decide \
 *       --concept-id concept:crimson-snapper \
 *       --decision NEW_ENTITY --proposed-name "Crimson Snapper" \
 *       --evidence-images <id1>,<id2>
 *
 * That is a deliberate friction, not an oversight.
 */
import { useEffect, useMemo, useState } from "react";
import { api } from "../lib/ipc";
import type { OntologyConcept, OntologyReviewPayload } from "../lib/types";
import { Button, Pill, Section } from "../components/primitives";

/** The four supported decisions. Display order is the order of increasing
 *  commitment: "I can't tell" first, "it's a new entity" last. */
const DECISIONS = [
  {
    id: "UNKNOWN",
    label: "Unknown / cannot determine",
    hint: "Evidence is insufficient. Legitimate and often correct.",
  },
  {
    id: "EXISTING_ENTITY",
    label: "Existing entity",
    hint: "The game is naming something the KB already has.",
  },
  {
    id: "VARIANT_OF_EXISTING",
    label: "Variant of an existing entity",
    hint: "Same entity, different appearance. Requires naming the variant.",
  },
  {
    id: "NEW_ENTITY",
    label: "New entity",
    hint: "The game names something the KB does not have.",
  },
] as const;

const BANDS = [
  { id: "P0", label: "P0 - strongest evidence", queue: "queueP0" as const },
  { id: "P1", label: "P1", queue: "queueP1" as const },
  { id: "P2", label: "P2", queue: "queueP2" as const },
  { id: "P3", label: "P3 - singletons", queue: "queueP3" as const },
];

/** Command line for recording a decision, pre-filled but never run. */
function decisionCommand(c: OntologyConcept, decision: string, entity: string, name: string) {
  const parts = [
    "python -m ml.gpo_ontology_review decide",
    `--concept-id ${c.conceptId}`,
    `--decision ${decision}`,
  ];
  if (decision === "EXISTING_ENTITY" || decision === "VARIANT_OF_EXISTING") {
    if (entity) parts.push(`--existing-entity ${entity}`);
    if (decision === "VARIANT_OF_EXISTING") parts.push(`--variant-descriptor "<describe the variant>"`);
  }
  if (decision === "NEW_ENTITY" && name) parts.push(`--proposed-name "${name}"`);
  const imgs = c.representativeImages.map((i) => i.imageId).slice(0, 3).join(",");
  if (imgs) parts.push(`--evidence-images ${imgs}`);
  return parts.join(" \\\n  ");
}

function OntologyConceptCard({ c }: { c: OntologyConcept }) {
  const [decision, setDecision] = useState<string | null>(null);
  const [entity, setEntity] = useState("");
  const [name, setName] = useState("");

  const cmd = useMemo(() => {
    if (!decision) return null;
    return decisionCommand(c, decision, entity, name || c.displayName);
  }, [c, decision, entity, name]);

  return (
    <div className="rounded-lg border border-line/70 p-3 flex flex-col gap-2">
      <div className="flex items-baseline justify-between gap-3 flex-wrap">
        <div className="text-[13px] font-semibold tracking-wide uppercase">
          {c.displayName}
        </div>
        <div className="flex items-center gap-2">
          <Pill tone={c.priority === "P0" ? "bad" : c.priority === "P1" ? "warn" : "mute"}>
            {c.priority}
          </Pill>
          {c.humanDecision ? (
            <Pill tone="ok">{c.humanDecision}</Pill>
          ) : (
            <Pill tone="accent">PENDING</Pill>
          )}
        </div>
      </div>

      <div className="flex flex-wrap gap-x-5 gap-y-1 text-[11px] text-fg-dim">
        <span>occurrences: {c.occurrences}</span>
        <span>sessions: {c.sessionCount}</span>
        <span>days: {c.dayCount}</span>
        <span className="font-mono text-fg-mute">{c.conceptId}</span>
      </div>

      {c.modifiers.length > 0 && (
        <div className="text-[11px] text-fg-mute">
          Decomposition (textual, not an ontology claim):{" "}
          {c.modifiers.join(" + ")} + <span className="font-semibold">{c.baseName}</span>
        </div>
      )}

      {c.representativeImages.length > 0 && (
        <div className="grid grid-cols-3 gap-1.5 mt-1">
          {c.representativeImages.map((e) => (
            <div
              key={e.imageId}
              className="rounded border border-line/60 bg-black/30 px-1.5 py-1 flex flex-col gap-0.5"
              title={`${e.imageId} - ${e.selectedBecause}`}
            >
              <div className="text-[9px] font-mono text-fg-mute truncate">{e.imageId}</div>
              <div className="text-[10px] text-fg-dim truncate">{(e.rawOcr || "").slice(0, 34)}</div>
              <div className="text-[9px] text-fg-mute truncate">
                {e.day} · {e.selectedBecause.slice(0, 20)}
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="text-[11px] flex flex-col gap-1 mt-1">
        <div>
          <span className="text-fg-dim">Current KB candidates: </span>
          {c.currentKbCandidates.length === 0 ? (
            <span className="text-fg-mute">
              none — the KB has no entity for this name
            </span>
          ) : (
            c.currentKbCandidates.map((e) => (
              <span key={e} className="font-mono text-fg-dim mr-2">{e}</span>
            ))
          )}
        </div>
        <div>
          <span className="text-fg-dim">Ontology status: </span>
          <span className="font-mono">{c.ontologyStatus}</span>
        </div>
        {c.storedLabels.length > 0 && (
          <div>
            <span className="text-fg-dim">Stored labels (evidence, not truth): </span>
            {c.storedLabels.map(([e, n]) => (
              <span key={e} className="font-mono text-fg-dim mr-2">{e} ×{n}</span>
            ))}
          </div>
        )}
        {c.storedAgreesWithCandidate > 0 && (
          <div className="text-fg-mute">
            {c.storedAgreesWithCandidate} row(s) carry a stored label equal to one
            candidate. That means the label agrees with one COMPONENT of this name —
            it is not a confirmation.
          </div>
        )}
        {c.ocrVariants.length > 1 && (
          <div className="text-fg-mute truncate">
            OCR variants: {c.ocrVariants.slice(0, 4).join(" | ")}
          </div>
        )}
      </div>

      {c.humanDecision ? (
        <div className="text-[11px] text-fg-mute border-t border-line/60 pt-2">
          Decided: <span className="font-semibold">{c.humanDecision}</span>
          {c.decisionNote ? ` — ${c.decisionNote}` : ""}
        </div>
      ) : (
        <div className="border-t border-line/60 pt-2 flex flex-col gap-2">
          <div className="text-[10px] uppercase tracking-wide text-fg-dim">
            What is this? (nothing is pre-selected)
          </div>
          <div className="flex flex-wrap gap-1.5">
            {DECISIONS.map((d) => (
              <span key={d.id} title={d.hint}>
                <Button
                  size="sm"
                  kind={decision === d.id ? "primary" : "ghost"}
                  onClick={() => setDecision(decision === d.id ? null : d.id)}
                >
                  {d.label}
                </Button>
              </span>
            ))}
          </div>
          {decision && (
            <div className="flex flex-col gap-1.5">
              {(decision === "EXISTING_ENTITY" || decision === "VARIANT_OF_EXISTING") && (
                <label className="text-[11px] flex items-center gap-2">
                  <span className="text-fg-dim w-40">Existing entity id</span>
                  <input
                    className="flex-1 bg-black/40 border border-line/70 rounded px-2 py-1 font-mono text-[11px]"
                    value={entity}
                    placeholder="fish:…"
                    onChange={(e) => setEntity(e.target.value)}
                  />
                </label>
              )}
              {decision === "NEW_ENTITY" && (
                <label className="text-[11px] flex items-center gap-2">
                  <span className="text-fg-dim w-40">Proposed canonical name</span>
                  <input
                    className="flex-1 bg-black/40 border border-line/70 rounded px-2 py-1 text-[11px]"
                    value={name}
                    placeholder={c.displayName}
                    onChange={(e) => setName(e.target.value)}
                  />
                </label>
              )}
              {cmd && (
                <div className="text-[10px] text-fg-mute">
                  <div className="mb-1">
                    This screen cannot record a decision on your behalf. Run:
                  </div>
                  <pre className="bg-black/50 border border-line/60 rounded p-2 overflow-x-auto whitespace-pre-wrap">
                    {cmd}
                  </pre>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function OntologyReview() {
  const [data, setData] = useState<OntologyReviewPayload | null>(null);
  const [band, setBand] = useState("P0");
  const [showDecided, setShowDecided] = useState(false);

  useEffect(() => {
    api
      .ontologyReviewRead()
      .then(setData)
      .catch(() =>
        setData({
          available: false,
          detail: "Could not read the ontology review dataset.",
          conceptsTotal: 0,
          decidedTotal: 0,
          pendingTotal: 0,
          queueP0: [],
          queueP1: [],
          queueP2: [],
          queueP3: [],
          concepts: [],
        }),
      );
  }, []);

  if (!data) return <div className="text-[11px] text-fg-mute">Loading…</div>;

  if (!data.available) {
    return (
      <Section title="Ontology review">
        <div className="text-[11px] text-fg-dim flex flex-col gap-2">
          <div>{data.detail}</div>
          <pre className="bg-black/50 border border-line/60 rounded p-2 overflow-x-auto">
            python -m ml.gpo_ocr_audit{`\n`}python -m ml.gpo_ontology_review build
          </pre>
        </div>
      </Section>
    );
  }

  const bandCfg = BANDS.find((b) => b.id === band)!;
  const byId = new Map(data.concepts.map((c) => [c.conceptId, c]));
  const queue = data[bandCfg.queue]
    .map((id) => byId.get(id))
    .filter((c): c is OntologyConcept => !!c);
  const decided = data.concepts.filter((c) => c.humanDecision !== null);

  return (
    <Section
      title="Ontology review"
      action={
        <div className="flex items-center gap-2">
          <Pill tone="bad">MODEL_NOT_READY</Pill>
          <Pill tone={data.pendingTotal > 0 ? "warn" : "ok"}>
            {data.decidedTotal}/{data.conceptsTotal} decided
          </Pill>
        </div>
      }
    >
      <div className="flex flex-col gap-2">
        <div className="text-[11px] text-fg-dim flex flex-col gap-1">
          <div>
            These are the names the <span className="font-semibold">game appears to be
            writing</span>, recovered from the OCR audit. The current knowledge
            base&apos;s candidates are shown as evidence, not as an answer — the
            ontology is what is under review, so it must not bias the decision.
          </div>
          <div className="text-fg-mute">{data.detail}</div>
        </div>

        <div className="flex items-center gap-1.5 flex-wrap">
          {BANDS.map((b) => (
            <Button
              key={b.id}
              size="sm"
              kind={band === b.id ? "primary" : "ghost"}
              onClick={() => setBand(b.id)}
            >
              {b.label} ({data[b.queue].length})
            </Button>
          ))}
          <span className="w-2" />
          <Button
            size="sm"
            kind={showDecided ? "primary" : "ghost"}
            onClick={() => setShowDecided(!showDecided)}
          >
            Decided ({decided.length})
          </Button>
        </div>

        {showDecided ? (
          decided.length === 0 ? (
            <div className="text-[11px] text-fg-mute">No decisions recorded yet.</div>
          ) : (
            <div className="grid grid-cols-1 xl:grid-cols-2 gap-2">
              {decided.map((c) => (
                <OntologyConceptCard key={c.conceptId} c={c} />
              ))}
            </div>
          )
        ) : queue.length === 0 ? (
          <div className="text-[11px] text-fg-mute">
            Nothing pending in {band}.
          </div>
        ) : (
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-2">
            {queue.map((c) => (
              <OntologyConceptCard key={c.conceptId} c={c} />
            ))}
          </div>
        )}
      </div>
    </Section>
  );
}