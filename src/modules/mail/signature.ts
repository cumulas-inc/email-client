// The signature a draft shows and goes out with: the From account's (its
// organization's or the person's own, as the core decides) while the draft
// follows it, or the person's edit.

import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

import { useBackend } from "../../api/context";
import { composeSignatureQuery, useComposeSignature } from "../../api/queries";
import type { ComposeSignature } from "../../api/types";
import type { SentSignature } from "./draft";
import type { ComposeDraft } from "./mailStore";

/** How long Send waits for an account's signature it doesn't know yet. */
export const SIGNATURE_WAIT_MS = 4000;

/** What the composer shows for the signature. */
export type SignatureView =
  | { kind: "none" }
  /** The signature, editable; `own` when it's the person's own. */
  | { kind: "editor"; html: string; own: boolean; edited: boolean }
  /** Removed: it can be added back. */
  | { kind: "removed" }
  /** The organization's mail server adds it on send. */
  | { kind: "addedOnSend" };

/** The signature the account offers, if it offers one the app adds. */
function offered(signature: ComposeSignature | undefined): SentSignature | null {
  if (!signature?.html || (signature.kind !== "organization" && signature.kind !== "own")) {
    return null;
  }
  return { html: signature.html, own: signature.kind === "own" };
}

/** The composer's signature, following the account's while the draft's is "auto". */
export function useSignatureView(draft: ComposeDraft | null): SignatureView {
  const signature = draft?.signature ?? "off";
  const { data } = useComposeSignature(
    draft?.accountId ?? "",
    draft?.replyKind ?? null,
    draft !== null && signature !== "off",
  );
  if (signature === "off") return { kind: "none" };
  if (typeof signature === "object") {
    return { kind: "editor", html: signature.edited, own: signature.own, edited: true };
  }
  const account = offered(data);
  if (signature === "removed") return account ? { kind: "removed" } : { kind: "none" };
  if (data?.kind === "addedOnSend") return { kind: "addedOnSend" };
  return account ? { kind: "editor", ...account, edited: false } : { kind: "none" };
}

/**
 * Returns `signatureFor(draft)`: the signature the draft goes out with. One
 * the account hasn't answered for yet is waited for, a few seconds at
 * most; then the message goes without.
 */
export function useSentSignature() {
  const backend = useBackend();
  const client = useQueryClient();
  return useCallback(
    async (draft: ComposeDraft): Promise<SentSignature | null> => {
      const signature = draft.signature;
      if (signature === "off" || signature === "removed") return null;
      if (typeof signature === "object") return { html: signature.edited, own: signature.own };
      const query = composeSignatureQuery(backend, draft.accountId, draft.replyKind);
      const known = client.getQueryData<ComposeSignature>(query.queryKey);
      if (known) return offered(known);
      let timer: ReturnType<typeof setTimeout> | undefined;
      const late = new Promise<undefined>((resolve) => {
        timer = setTimeout(() => resolve(undefined), SIGNATURE_WAIT_MS);
      });
      const answer = await Promise.race([client.fetchQuery(query).catch(() => undefined), late]);
      clearTimeout(timer);
      return offered(answer);
    },
    [backend, client],
  );
}
