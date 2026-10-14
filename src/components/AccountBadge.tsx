import type { CSSProperties } from "react";

import { accountMark } from "../lib/format";

/**
 * The account's initials on a rounded square in the account's color, the
 * same everywhere an account heads a list. Decorative: the account's name is
 * always shown next to it.
 */
export function AccountBadge({
  name,
  color,
  size = 22,
  className,
}: {
  name: string;
  color: string;
  size?: number;
  className?: string;
}) {
  return (
    <span
      className={className ? `account-badge ${className}` : "account-badge"}
      style={
        {
          "--account-color": color,
          width: size,
          height: size,
          borderRadius: Math.round(size * 0.32),
          fontSize: Math.round(size * 0.44 * 10) / 10,
        } as CSSProperties
      }
      aria-hidden="true"
    >
      {accountMark(name)}
    </span>
  );
}
