/**
 * WIKI REFERENCE PANEL
 *
 * # The question this answers
 *
 * "Is this name the thing, or not the thing?"
 *
 * Everything else on the review screen is internal evidence: OCR text from this
 * project's own dataset, and a knowledge base this project wrote. Both are circular
 * - the KB encodes assumptions that are exactly what is under review. The game's own
 * wiki is the only independent reference available, so it is the one piece of
 * evidence that can actually settle a question rather than restate it.
 *
 * It settles the hard ones. The `Fishing/Bestiary` page lists 16 fish. Checking all
 * 77 observed names against it:
 *
 *   * all 11 P0 concepts are documented fish, with rarity and seasonal status;
 *   * 42 more are OCR garbles of documented fish (`sgolden tigerfin` ->
 *     `Golden Tigerfin`, `blve lipgrouper` -> `Blue-Lip Grouper`);
 *   * the rest are mostly banner text that is not a fish at all.
 *
 * It also disproves the current ontology: the wiki has no bare `angelfish`,
 * `puffer`, `snapper`, `squid` or `shark`. Those KB entities are fictional.
 *
 * # Why the panel states a match rather than making one
 *
 * The wiki is community-editable. Someone can add a fish that does not exist. So:
 *
 *   * a MATCH is strong evidence the name is real, and this component says so;
 *   * NO MATCH is evidence only that this list omits it, and the panel says THAT,
 *     because the opposite reading - "the fish does not exist" - is how a reference
 *     tool starts manufacturing conclusions.
 *
 * Nothing here ever selects a decision. `NEW_ENTITY` is still the human's call, even
 * for a name the wiki lists, because "the wiki lists it" and "it should become an
 * entity" are two different claims.
 */

import { useCallback, useState } from "react";
import { api } from "../lib/ipc";
import type { WikiMatch, WikiRefreshResult } from "../lib/types";
import { Button, Pill } from "../components/primitives";

type Props = {
  match: WikiMatch | undefined;
  /** Shown when the index predates the reference, so the absence is explained. */
  referenceAvailable: boolean;
  retrievedAtUnix: number | null;
  entryCount: number;
  exactMatches: number;
  nearMisses: number;
  onRefreshed: () => void;
};

const RARITY_TONE: Record<string, "mute" | "accent" | "warn" | "bad"> = {
  Common: "mute",
  Rare: "accent",
  Epic: "warn",
  Legendary: "bad",
  Unknown: "mute",
};

export function WikiReferencePanel({
  match,
  referenceAvailable,
  retrievedAtUnix,
  entryCount,
  exactMatches,
  nearMisses,
  onRefreshed,
}: Props) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<WikiRefreshResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.ontologyWikiRefresh();
      if (r.ok) {
        setResult(r);
        onRefreshed();
      } else {
        setError(r.message);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [onRefreshed]);

  const retrieved = retrievedAtUnix
    ? new Date(retrievedAtUnix * 1000).toISOString().slice(0, 10)
    : null;

  return (
    <div className="rounded-lg border border-line/70 px-2.5 py-2 flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <span className="text-[9px] uppercase tracking-wide text-fg-dim">
          Game wiki reference
        </span>
        <Button
          size="sm"
          kind="ghost"
          onClick={() => void refresh()}
          disabled={busy}
          title="Re-fetch the game's fish list from the wiki. This is the only network call in this tool, and it happens only when you press it."
        >
          {busy ? "Refreshing…" : "Refresh"}
        </Button>
      </div>

      {error && (
        <div className="text-[10px] text-bad border border-bad/40 rounded px-1.5 py-1">
          {error}
          <div className="text-fg-mute mt-0.5">
            The previous reference is still in use - a failed refresh changes nothing.
          </div>
        </div>
      )}
      {result?.ok && (
        <div className="text-[10px] text-ok border border-ok/40 rounded px-1.5 py-1">
          {result.unchanged ? "Up to date - the wiki has not changed." : "Updated."}{" "}
          {result.message}
        </div>
      )}

      {!referenceAvailable ? (
        <div className="text-[10px] text-fg-mute">
          No reference snapshot on disk. Refresh to fetch it, or run{" "}
          <span className="font-mono">python -m ml.gpo_ontology_review wiki-refresh</span>.
          <div className="mt-1 text-fg-dim">
            Until then this panel says nothing - a missing reference is not evidence
            that a name is not a fish.
          </div>
        </div>
      ) : !match ? (
        <div className="text-[10px] text-fg-mute">No reference data for this concept.</div>
      ) : match.matched ? (
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-1.5 flex-wrap">
            <Pill tone="ok">DOCUMENTED FISH</Pill>
            {match.rarity && <Pill tone={RARITY_TONE[match.rarity] ?? "mute"}>{match.rarity}</Pill>}
            {match.seasonal && <Pill tone="warn">SEASONAL</Pill>}
          </div>
          <div className="text-[13px] font-semibold leading-tight">
            {match.canonicalName}
          </div>
          {match.description && (
            <div className="text-[10px] text-fg-dim leading-tight">{match.description}</div>
          )}
          <div className="text-[9px] text-fg-mute">
            {match.basis}. Reference only - it is not a decision, and it does not
            pre-select one.
          </div>
          {match.url && (
            <a
              href={match.url}
              target="_blank"
              rel="noreferrer noopener"
              className="text-[10px] text-accent underline"
            >
              Open the wiki page
            </a>
          )}
        </div>
      ) : match.suggestion ? (
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-1.5 flex-wrap">
            <Pill tone="warn">NEAR MATCH</Pill>
            {match.suggestionRarity && (
              <Pill tone={RARITY_TONE[match.suggestionRarity] ?? "mute"}>
                {match.suggestionRarity}
              </Pill>
            )}
          </div>
          <div className="text-[11px] leading-tight">
            Closest documented fish:{" "}
            <span className="font-semibold">{match.suggestion}</span>
          </div>
          <div className="text-[9px] text-fg-mute">
            {match.basis}. Likely the same fish read imperfectly - check the evidence
            images before deciding.
          </div>
          {match.url && (
            <a
              href={match.url}
              target="_blank"
              rel="noreferrer noopener"
              className="text-[10px] text-accent underline"
            >
              Open the wiki page
            </a>
          )}
        </div>
      ) : (
        <div className="flex flex-col gap-1">
          <Pill tone="mute">NOT IN THIS LIST</Pill>
          <div className="text-[9px] text-fg-mute leading-tight">
            This name is absent from the wiki's {entryCount}-fish bestiary. That is{" "}
            <span className="text-fg-dim">not</span> evidence the fish does not exist -
            the wiki is community-editable and can lag the game. It often means the text
            is not a fish at all, so check whether the screenshot is a banner or a
            location message.
          </div>
        </div>
      )}

      <div className="text-[9px] text-fg-mute border-t border-line/60 pt-1.5 leading-tight">
        <span className="text-fg-dim">Reference coverage:</span> {exactMatches} of the
        observed names match exactly, {nearMisses} are near matches.
        {retrieved && (
          <>
            {" "}
            Fetched <span className="font-mono">{retrieved}</span>.
          </>
        )}
      </div>
    </div>
  );
}

export default WikiReferencePanel;