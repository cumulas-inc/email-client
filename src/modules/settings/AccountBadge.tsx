import type { CSSProperties } from "react";

/** The letter on an account's badge: "W" for Work. Its color tells accounts apart. */
function accountInitial(name: string): string {
  const letter = name.match(/[\p{L}\p{N}]/u)?.[0] ?? "?";
  return letter.toLocaleUpperCase();
}

/**
 * The account's initial on a rounded square in the account color, the same
 * mark the folder list uses. Decorative: the name is always shown next to it.
 */
export function AccountBadge({ name, color }: { name: string; color: string }) {
  return (
    <span
      className="account-badge"
      style={{ "--account-color": color } as CSSProperties}
      aria-hidden="true"
    >
      {accountInitial(name)}
    </span>
  );
}
