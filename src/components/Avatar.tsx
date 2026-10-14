import { colorFor, initials } from "../lib/format";

interface AvatarProps {
  name: string;
  /** Used for the colour; defaults to the name. */
  colorKey?: string;
  size?: number;
}

export function Avatar({ name, colorKey, size = 32 }: AvatarProps) {
  return (
    <span
      className="avatar"
      aria-hidden="true"
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.4),
        background: colorFor(colorKey ?? name),
      }}
    >
      {initials(name)}
    </span>
  );
}
