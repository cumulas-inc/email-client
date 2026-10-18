import { Fragment, useMemo } from "react";

import type { TextRun } from "../../../api/types";
import { type HitPattern, findHits, leadToFirstHit, toRuns } from "../../../lib/search/highlight";

/**
 * Text with what a search found in it marked; the text itself without a
 * hit. With `lead`, what comes before the first hit is cut to about that
 * many characters, so a one-line row shows the hit.
 */
export function Highlighted({
  text,
  patterns,
  lead,
}: {
  text: string;
  patterns: readonly HitPattern[] | null | undefined;
  lead?: number;
}) {
  const runs = useMemo(() => {
    if (!patterns || patterns.length === 0) return null;
    const runs = toRuns(text, findHits(text, patterns));
    return lead === undefined ? runs : leadToFirstHit(runs, lead);
  }, [text, patterns, lead]);
  if (!runs || !runs.some((r) => r.hit)) return text;
  return <Runs runs={runs} />;
}

/** Runs of text, the hits marked. */
export function Runs({ runs }: { runs: readonly TextRun[] }) {
  return runs.map((run, i) =>
    run.hit ? (
      <mark key={i} className="search-hit">
        {run.text}
      </mark>
    ) : (
      <Fragment key={i}>{run.text}</Fragment>
    ),
  );
}
