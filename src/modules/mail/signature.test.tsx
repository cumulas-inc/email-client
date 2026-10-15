import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BackendProvider } from "../../api/BackendProvider";
import { MockBackend } from "../../api/mock";
import { newDraft } from "./draft";
import type { ComposeDraft } from "./mailStore";
import { SIGNATURE_WAIT_MS, useSentSignature } from "./signature";

function setup(backend: MockBackend) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <BackendProvider backend={backend}>
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    </BackendProvider>
  );
  return renderHook(() => useSentSignature(), { wrapper }).result.current;
}

function draft(patch: Partial<ComposeDraft> = {}): ComposeDraft {
  return { ...newDraft("acct-work"), id: 1, dirty: false, ...patch };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("the signature a message goes out with", () => {
  it("is the person's edit, none once removed, or the account's", async () => {
    const signatureFor = setup(new MockBackend());
    expect(
      await signatureFor(draft({ signature: { edited: "<div>Ada</div>", own: true } })),
    ).toEqual({ html: "<div>Ada</div>", own: true });
    expect(await signatureFor(draft({ signature: "removed" }))).toBeNull();
    expect(await signatureFor(draft({ signature: "off" }))).toBeNull();
    expect(await signatureFor(draft())).toMatchObject({ own: false });
    expect(await signatureFor(draft({ accountId: "acct-home", replyKind: "reply" }))).toEqual({
      html: "<div>Alex Morgan</div><div>+1 (206) 555-0199</div>",
      own: true,
    });
  });

  it("isn't waited for more than a few seconds", async () => {
    vi.useFakeTimers();
    const backend = new MockBackend();
    backend.getComposeSignature = () => new Promise(() => undefined);
    const signatureFor = setup(backend);
    let result: unknown = "pending";
    void signatureFor(draft()).then((r) => (result = r));
    await vi.advanceTimersByTimeAsync(SIGNATURE_WAIT_MS - 100);
    expect(result).toBe("pending");
    await vi.advanceTimersByTimeAsync(100);
    expect(result).toBeNull();
  });
});
