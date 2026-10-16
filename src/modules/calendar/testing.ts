// Helpers for the calendar tests (which run in Node, where changing TZ takes
// effect immediately). The app's tsconfig has no Node types, hence the cast.

const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } })
  .process.env;

/** Sets the process time zone; `undefined` restores the system default. */
export function setTimeZone(tz: string | undefined): void {
  if (tz === undefined) delete env.TZ;
  else env.TZ = tz;
}

export function currentTimeZone(): string | undefined {
  return env.TZ;
}

/** Runs `fn` with the process time zone set to `tz`. */
export function withTimeZone<T>(tz: string, fn: () => T): T {
  const previous = currentTimeZone();
  setTimeZone(tz);
  try {
    return fn();
  } finally {
    setTimeZone(previous);
  }
}
