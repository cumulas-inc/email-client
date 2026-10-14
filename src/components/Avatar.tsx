import { UserRound } from "lucide-react";

import { hueFor, initials } from "../lib/format";

interface AvatarProps {
  name: string;
  /** Used for the color; defaults to the name. */
  colorKey?: string;
  size?: number;
}

export function Avatar({ name, colorKey, size = 32 }: AvatarProps) {
  const letters = initials(name);
  return (
    <span
      className={`avatar avatar--${hueFor(colorKey ?? name)}`}
      aria-hidden="true"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.37) }}
    >
      {letters === "?" ? (
        <UserRound size={Math.round(size * 0.5)} strokeWidth={2} aria-hidden="true" />
      ) : (
        letters
      )}
    </span>
  );
}
