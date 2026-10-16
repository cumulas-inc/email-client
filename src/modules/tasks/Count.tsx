import { clsx } from "clsx";
import { memo } from "react";

import { formatCount } from "../../lib/format";
import { exactCount } from "./fullText";

/** A count as shown in the list ("1,204", "12K"), with the exact number as a tooltip when shortened. */
export const Count = memo(function Count({ n, className }: { n: number; className?: string }) {
  const shown = formatCount(n);
  const exact = exactCount(n);
  return (
    <span className={clsx("tnum", className)} title={shown === exact ? undefined : exact}>
      {shown}
    </span>
  );
});
