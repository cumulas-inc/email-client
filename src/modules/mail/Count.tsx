import { formatCount } from "../../lib/format";
import { exactCount } from "./counts";

/** A count as badges show it ("1,204", "12K"), with the exact number on hover when shortened. */
export function Count({ n, className }: { n: number; className?: string }) {
  const shown = formatCount(n);
  const exact = exactCount(n);
  return (
    <span className={className} title={shown === exact ? undefined : exact}>
      {shown}
    </span>
  );
}
