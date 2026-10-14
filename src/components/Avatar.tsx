import { UserRound } from "lucide-react";
import type { CSSProperties } from "react";

import { hueFor, initials } from "../lib/format";

interface AvatarProps {
  name: string;
  /** Used for the color; defaults to the name. */
  colorKey?: string;
  /** An account color to tint with instead, for the signed-in person, so
      their avatar matches the account's badge. */
  color?: string;
  size?: number;
}

export function Avatar({ name, colorKey, color, size = 32 }: AvatarProps) {
  const letters = initials(name);
  return (
    <span
      className={color ? "avatar avatar--account" : `avatar avatar--${hueFor(colorKey ?? name)}`}
      aria-hidden="true"
      style={
        {
          width: size,
          height: size,
          fontSize: Math.round(size * 0.37),
          ...(color ? { "--account-color": color } : {}),
        } as CSSProperties
      }
    >
      {letters === "?" ? (
        <UserRound size={Math.round(size * 0.5)} strokeWidth={2} aria-hidden="true" />
      ) : (
        letters
      )}
    </span>
  );
}
