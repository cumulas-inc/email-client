import type { BackendError, ErrorCode } from "./types";

export function isBackendError(e: unknown): e is BackendError {
  return (
    typeof e === "object" &&
    e !== null &&
    "code" in e &&
    "message" in e &&
    typeof (e as { message: unknown }).message === "string"
  );
}

export function hasErrorCode(e: unknown, code: ErrorCode): e is BackendError {
  return isBackendError(e) && e.code === code;
}

/** A message suitable for showing to the user. */
export function errorMessage(e: unknown): string {
  if (isBackendError(e)) return e.message;
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  return "Something went wrong.";
}
