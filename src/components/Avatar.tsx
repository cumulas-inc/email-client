import { hueFor, initials } from "../lib/format";

interface AvatarProps {
  name: string;
  /** Used for the color; defaults to the name. */
  colorKey?: string;
  size?: number;
}

export function Avatar({ name, colorKey, size = 32 }: AvatarProps) {
  return (
    <span
      className={`avatar avatar--${hueFor(colorKey ?? name)}`}
      aria-hidden="true"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.37) }}
    >
      {initials(name)}
    </span>
  );
}
