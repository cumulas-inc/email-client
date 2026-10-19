import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, onTestFinished, vi } from "vitest";

import { MockBackend, type MockOptions, mockAdminConsentUrl } from "../../api/mock";
import type { Account, CheckResult, LabelOption, ProtectionSetup } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useBackendEvents } from "../../app/useBackendEvents";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { checkDate } from "../mail/protection";
import { useAddAccountRequest } from "./addAccountStore";
import { PROTECTION_GUIDE_URL } from "./microsoft";
import { ProtectionField } from "./ProtectionField";
import { AccountsSection } from "./sections/AccountsSection";

const EMAIL = "alex.morgan@acme.example";

function Events() {
  useBackendEvents();
  return null;
}

async function renderField(backend = new MockBackend()) {
  const account = (await backend.listAccounts()).find((a) => a.id === "acct-work")!;
  const utils = renderWithBackend(
    <>
      <ProtectionField account={account} returnFocusId="sync-now" />
      <Events />
    </>,
    { backend },
  );
  await screen.findByRole("heading", { name: "Encryption" });
  return { ...utils, backend, account };
}

/** A setup read from the demo backend, changed as a test needs. */
async function stubSetup(
  backend: MockBackend,
  change: (setup: ProtectionSetup) => Partial<ProtectionSetup>,
) {
  const real = await backend.getProtectionSetup("acct-work");
  vi.spyOn(backend, "getProtectionSetup").mockResolvedValue({ ...real, ...change(real) });
}

/** The row named `name`, in the list of checks. */
function row(name: string | RegExp) {
  const rows = screen.getAllByRole("listitem");
  const found = rows.find((r) => {
    const label = r.querySelector(".protection-row__name")?.textContent ?? "";
    return typeof name === "string" ? label === name : name.test(label);
  });
  if (!found) throw new Error(`No row named ${String(name)}`);
  return found;
}

const statusOf = (name: string | RegExp) => within(row(name)).getByRole("status");

const result = (patch: Partial<CheckResult>): CheckResult => ({
  state: "notChecked",
  at: null,
  note: null,
  testMessage: false,
  // A pass counts from when it arrived.
  passedAt: patch.state === "applied" ? (patch.at ?? null) : null,
  otherLabelId: null,
  otherLabelOnControl: false,
  changedAt: null,
  ...patch,
});

beforeEach(() => {
  useAppStore.setState({ addAccountOpen: false, settingsOpen: null });
  useAddAccountRequest.setState({ request: { step: "choose" } });
  useToasts.setState({ toasts: [] });
});

describe("Sensitivity labels and encryption", () => {
  it("explains the checks, and lists the labels with nothing checked yet", async () => {
    const user = userEvent.setup();
    const { backend } = await renderField();
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);

    expect(screen.getByText("Sensitivity labels and encryption")).toBeInTheDocument();
    expect(screen.getByText(/once your IT team has set that up/)).toHaveTextContent(
      "Your organization's mail server can encrypt or label the mail you send from Cumulas Mail, once your IT team has set that up. Each option appears in new messages for 90 days after a test message here arrives encrypted or labeled. New rules can take 30 minutes or more to start working, and auto-labeling policies up to a day.",
    );
    // Nothing checked yet: nothing for IT.
    expect(screen.queryByRole("button", { name: "Copy details for IT" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "How IT sets this up" }));
    expect(openLink).toHaveBeenCalledWith(PROTECTION_GUIDE_URL);

    expect(statusOf("Encrypt")).toHaveTextContent("Not checked yet");
    expect(statusOf("Do Not Forward")).toHaveTextContent("Not checked yet");
    expect(
      within(row("Do Not Forward")).getByText(/can't tell Do Not Forward from Encrypt/),
    ).toHaveTextContent(
      "Cumulas Mail can't tell Do Not Forward from Encrypt in a test message. Open the test message in Outlook on the web: it should say that it can't be forwarded.",
    );
    expect(screen.getByRole("button", { name: `Check Encrypt for ${EMAIL}` })).toHaveTextContent(
      "Check",
    );

    expect(await screen.findByRole("heading", { name: "Sensitivity labels" })).toBeInTheDocument();
    expect(screen.getByText(/5 labels/)).toHaveTextContent(
      `5 labels · A label is required · Default: General · Updated ${checkDate(new Date().toISOString())}`,
    );
    expect(statusOf("Confidential › All Employees")).toHaveTextContent("Not checked yet");
    // Labels that encrypt say so, in words too.
    expect(screen.getAllByRole("img", { name: "This label encrypts" })).toHaveLength(2);
    expect(
      within(row("Confidential › All Employees")).getByRole("img", {
        name: "This label encrypts",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: `Check labels for ${EMAIL}` })).toHaveTextContent(
      "Check all labels",
    );
    expect(screen.queryByRole("button", { name: /Open the test message/ })).toBeNull();
  });

  it("says how each check went", async () => {
    const backend = new MockBackend({ protection: "setup" });
    const at = (await backend.getProtectionSetup("acct-work")).encrypt.at!;
    await renderField(backend);
    const on = checkDate(at);

    await waitFor(() =>
      expect(statusOf("Encrypt")).toHaveTextContent(`Arrived encrypted · checked ${on}`),
    );
    expect(statusOf("General")).toHaveTextContent(`Arrived labeled · checked ${on}`);
    expect(statusOf("Highly Confidential › Project Atlas")).toHaveTextContent(
      `Arrived without it · checked ${on}`,
    );
    // Why, as far as the check could tell, and who fixes it.
    expect(
      row("Highly Confidential › Project Atlas").querySelector(".protection-row__note"),
    ).toHaveTextContent(
      /Your IT team can fix this\. Copy details for IT gives them what the check found\.$/,
    );
    expect(statusOf("Public")).toHaveTextContent(`Couldn't tell · ${on}`);
    expect(
      within(row("Public")).getByText(
        "Another rule in your organization already protects or labels this mail, so Cumulas Mail can't tell whether its own rule works.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `Check Encrypt again for ${EMAIL}` }),
    ).toHaveTextContent("Check again");
    expect(
      screen.getByRole("button", { name: `Check the label General again for ${EMAIL}` }),
    ).toBeInTheDocument();
  });

  it("says when a check is too old to use, and when the app couldn't tell", async () => {
    const backend = new MockBackend({ protection: "expired" });
    const at = "2026-03-02T10:00:00Z";
    await stubSetup(backend, (setup) => ({
      encrypt: result({ state: "expired", at, testMessage: true }),
      doNotForward: result({ state: "unknown", note: "headerKept", at }),
      labels: setup.labels.map((l, i) =>
        i === 0
          ? { ...l, check: result({ state: "unknown", note: "notArrived", at }) }
          : i === 1
            ? { ...l, check: result({ state: "notApplied", note: "headerNotKept", at }) }
            : l,
      ),
    }));
    await renderField(backend);
    const on = checkDate(at);

    expect(statusOf("Encrypt")).toHaveTextContent(
      `Checked over 90 days ago (${on}). Check again to use it.`,
    );
    expect(statusOf("Do Not Forward")).toHaveTextContent(`Couldn't tell · ${on}`);
    expect(
      within(row("Do Not Forward")).getByText(/didn't remove the x-cumulas-protect header/),
    ).toHaveTextContent(
      "The test message arrived encrypted, but your organization's rule didn't remove the x-cumulas-protect header, so another rule may have encrypted it. Ask your IT team to check the rule.",
    );
    expect(statusOf("Public")).toHaveTextContent(`Couldn't check · ${on}`);
    expect(
      within(row("Public")).getByText(
        "The test message didn't arrive in your Inbox within 15 minutes.",
      ),
    ).toBeInTheDocument();
    expect(statusOf("General")).toHaveTextContent(`Couldn't send the test · ${on}`);
    expect(
      within(row("General")).getByText(
        "Microsoft 365 didn't keep the request on the test message, so Cumulas Mail can't ask for this.",
      ),
    ).toBeInTheDocument();
  });

  it("checks, waits for the test message, and says what arrived", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    onTestFinished(() => void vi.useRealTimers());
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const backend = new MockBackend();
    const check = vi.spyOn(backend, "checkProtection");
    const look = vi.spyOn(backend, "refreshProtectionCheck");
    await renderField(backend);

    await user.click(screen.getByRole("button", { name: `Check Encrypt for ${EMAIL}` }));
    expect(check).toHaveBeenCalledWith("acct-work", [{ kind: "action", action: "encrypt" }]);
    await waitFor(() => expect(statusOf("Encrypt")).toHaveTextContent("Checking…"));
    expect(screen.getByText(/is waiting for them to arrive/)).toHaveTextContent(
      `Cumulas Mail sent test messages to ${EMAIL} and is waiting for them to arrive. This usually takes a minute or two.`,
    );
    // Nothing else can be checked meanwhile; the buttons keep their place
    // for the keyboard, and the one pressed says it's busy.
    const pressed = screen.getByRole("button", { name: `Check Encrypt again for ${EMAIL}` });
    expect(pressed).toHaveAttribute("aria-disabled", "true");
    expect(pressed).toHaveClass("btn--busy");
    for (const name of [`Check Do Not Forward for ${EMAIL}`, `Check labels for ${EMAIL}`]) {
      const other = screen.getByRole("button", { name });
      expect(other).toHaveAttribute("aria-disabled", "true");
      expect(other).not.toHaveClass("btn--busy");
      await user.click(other);
    }
    expect(check).toHaveBeenCalledTimes(1);

    // Every 10 seconds it looks; the demo's arrive by the second look.
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(look).toHaveBeenCalledTimes(1);
    expect(statusOf("Encrypt")).toHaveTextContent("Checking…");
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    await waitFor(() => expect(statusOf("Encrypt")).toHaveTextContent(/^Arrived encrypted/));
    expect(
      screen.getByText("Done. Cumulas Mail moves the test messages it reads to Deleted Items."),
    ).toBeInTheDocument();
    await act(() => vi.advanceTimersByTimeAsync(30_000));
    expect(look).toHaveBeenCalledTimes(2);

    const open = vi.spyOn(backend, "openProtectionCheckMessage");
    await user.click(
      screen.getByRole("button", {
        name: "Open the test message for Encrypt in Outlook on the web",
      }),
    );
    expect(open).toHaveBeenCalledWith("acct-work", { kind: "action", action: "encrypt" });
    expect(backend.calls.openedInOutlook).toEqual(["check:acct-work:encrypt"]);
  });

  it("checks both kinds of encryption, and all the labels, in order", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const check = vi.spyOn(backend, "checkProtection");
    await renderField(backend);

    await user.click(
      screen.getByRole("button", { name: `Check Encrypt and Do Not Forward for ${EMAIL}` }),
    );
    expect(check).toHaveBeenLastCalledWith("acct-work", [
      { kind: "action", action: "encrypt" },
      { kind: "action", action: "doNotForward" },
    ]);
    await waitFor(() => expect(statusOf("Do Not Forward")).toHaveTextContent("Checking…"));
    // A check is waiting: the backend would refuse another.
    const labels = screen.getByRole("button", { name: `Check labels for ${EMAIL}` });
    expect(labels).toHaveAttribute("aria-disabled", "true");
    await user.click(labels);
    expect(check).toHaveBeenCalledTimes(1);
  });

  it("keeps the keyboard on the button that started a check", async () => {
    const user = userEvent.setup();
    await renderField();
    const button = screen.getByRole("button", { name: `Check Encrypt for ${EMAIL}` });
    button.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(statusOf("Encrypt")).toHaveTextContent("Checking…"));
    // The same button, now "Check again" and busy, still has the focus.
    expect(button).toHaveAccessibleName(`Check Encrypt again for ${EMAIL}`);
    expect(button).toHaveAttribute("aria-disabled", "true");
    expect(button).not.toBeDisabled();
    expect(button).toHaveFocus();
  });

  it("keeps the keyboard on Check all labels while every label is checked", async () => {
    const user = userEvent.setup();
    await renderField();
    const button = await screen.findByRole("button", { name: `Check labels for ${EMAIL}` });
    button.focus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(statusOf("General")).toHaveTextContent("Checking…"));
    // All of them waiting leaves nothing more to check, but the button stays.
    expect(button).toHaveAttribute("aria-disabled", "true");
    expect(button).toHaveClass("btn--busy");
    expect(button).not.toBeDisabled();
    expect(button).toHaveFocus();
  });

  it("says that a pass still applies while it's checked again, and when its test didn't arrive", async () => {
    const at = "2026-10-04T09:00:00Z";
    const on = checkDate(at);
    const backend = new MockBackend();
    await stubSetup(backend, () => ({
      encrypt: result({ state: "waiting", at: "2026-10-18T09:00:00Z", passedAt: at }),
      doNotForward: result({ state: "applied", at, note: "notArrived", testMessage: true }),
    }));
    await renderField(backend);
    expect(statusOf("Encrypt")).toHaveTextContent("Checking…");
    expect(
      within(row("Encrypt")).getByText(
        `Until this check is done, the one from ${on} still applies.`,
      ),
    ).toBeInTheDocument();
    expect(statusOf("Do Not Forward")).toHaveTextContent(`Arrived encrypted · checked ${on}`);
    expect(
      within(row("Do Not Forward")).getByText(
        "The latest test message didn't arrive in your Inbox within 15 minutes, so this earlier check still applies.",
      ),
    ).toBeInTheDocument();
  });

  it("checks 19 labels at a time: never checked first, then the oldest checks", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const labels: LabelOption[] = Array.from({ length: 22 }, (_, i) => ({
      id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
      name: `Label ${i}`,
      parentName: null,
      tooltip: null,
      color: null,
      hasProtection: false,
      sensitivity: i,
      check:
        i < 4
          ? result({ state: "applied", at: `2026-10-0${4 - i}T09:00:00Z` })
          : result({ state: "notChecked" }),
    }));
    await stubSetup(backend, () => ({ labels }));
    const check = vi.spyOn(backend, "checkProtection").mockImplementation(async () => {
      throw { code: "invalidInput", message: "Not in the demo." };
    });
    await renderField(backend);

    expect(
      screen.getByText("Cumulas Mail checks 19 labels at a time. Check again for the rest."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: `Check labels for ${EMAIL}` }));
    const items = check.mock.lastCall![1];
    expect(items).toHaveLength(19);
    expect(items.slice(0, 2)).toEqual([
      { kind: "label", labelId: labels[4]!.id },
      { kind: "label", labelId: labels[5]!.id },
    ]);
    // The 18 never checked, then the oldest check.
    expect(items[18]).toEqual({ kind: "label", labelId: labels[3]!.id });
    expect(screen.getByRole("button", { name: `Check labels for ${EMAIL}` })).toHaveTextContent(
      "Check 19 labels",
    );
    await waitFor(() =>
      expect(useToasts.getState().toasts.map((t) => t.message)).toEqual(["Not in the demo."]),
    );
  });

  it("turns labels on, or asks IT, when they need permission", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ missing: ["sensitivityLabels"] });
    await renderField(backend);

    expect(
      screen.getByText("Cumulas Mail needs permission to read the sensitivity labels you can use."),
    ).toBeInTheDocument();
    // The encryption checks don't need it.
    expect(statusOf("Encrypt")).toHaveTextContent("Not checked yet");

    const ask = screen.getByRole("button", {
      name: `Ask IT about sensitivity labels for ${EMAIL}`,
    });
    await user.click(ask);
    expect(ask).toHaveAttribute("aria-expanded", "true");
    expect(await screen.findByLabelText("Approval link for your IT administrator")).toHaveValue(
      mockAdminConsentUrl("demo-tenant"),
    );

    await user.click(
      screen.getByRole("button", { name: `Turn on sensitivity labels for ${EMAIL}` }),
    );
    expect(useAppStore.getState().addAccountOpen).toBe(true);
    expect(useAddAccountRequest.getState().request).toEqual({
      step: "microsoft",
      email: EMAIL,
      autoStart: true,
      permissions: { kind: "feature", feature: "sensitivityLabels" },
      returnFocusTo: "sync-now",
    });
  });

  it("says when Microsoft 365 refused the labels, with Check again and Email IT", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ refused: ["sensitivityLabels"] });
    const retry = vi.spyOn(backend, "retryMicrosoftFeatures");
    const openLink = vi.spyOn(backend, "openLink").mockResolvedValue(undefined);
    await renderField(backend);

    expect(
      screen.getByText("Microsoft 365 refused access to your sensitivity labels"),
    ).toBeInTheDocument();
    expect(screen.getByText(/Ask your IT team to check/)).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: `Check sensitivity labels again for ${EMAIL}` }),
    );
    expect(retry).toHaveBeenCalledWith("acct-work");
    await user.click(
      screen.getByRole("button", { name: `Email IT about sensitivity labels for ${EMAIL}` }),
    );
    expect(decodeURIComponent(String(openLink.mock.lastCall?.[0]))).toMatch(
      /^mailto:\?subject=Cumulas Mail can't open my sensitivity labels&body=/,
    );
  });

  it("says when the labels couldn't be listed, or there are none", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    const refresh = vi.spyOn(backend, "refreshSensitivityLabels");
    await stubSetup(backend, () => ({
      labels: [],
      labelsListedAt: null,
      labelsError: "Microsoft Graph returned 503.",
    }));
    const { unmount } = await renderField(backend);
    expect(
      screen.getByText(
        "Couldn't get your labels from Microsoft 365: Microsoft Graph returned 503.",
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(refresh).toHaveBeenCalledWith("acct-work");
    unmount();

    vi.restoreAllMocks();
    const listed = "2026-10-10T08:00:00Z";
    await stubSetup(backend, () => ({ labelsListedAt: listed, labelsError: "Timed out." }));
    const stale = await renderField(backend);
    expect(
      screen.getByText(
        `Couldn't update your labels (Timed out.). These are from ${checkDate(listed)}.`,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: `Refresh sensitivity labels for ${EMAIL}` }),
    ).toBeInTheDocument();
    stale.unmount();

    vi.restoreAllMocks();
    await stubSetup(backend, () => ({ labels: [], labelsListedAt: listed }));
    await renderField(backend);
    expect(
      screen.getByText(/Microsoft 365 lists no sensitivity labels for email for you\./),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: `Check labels for ${EMAIL}` })).toBeNull();
  });

  it("says it's getting the labels until Microsoft 365 has listed them", async () => {
    const backend = new MockBackend();
    await stubSetup(backend, () => ({ labels: [], labelsListedAt: null, labelsError: null }));
    await renderField(backend);
    expect(screen.getByText("Getting your labels from Microsoft 365…")).toBeInTheDocument();
  });
});

describe("what the checks found", () => {
  const at = "2026-10-12T09:30:00Z";
  const on = checkDate(at);
  const IT = " Your IT team can fix this. Copy details for IT gives them what the check found.";

  /** The demo's setup with Encrypt, Do Not Forward and labels (by name) checked as given. */
  async function withChecks(
    encrypt: Partial<CheckResult>,
    doNotForward: Partial<CheckResult>,
    labels: Record<string, Partial<CheckResult>> = {},
  ) {
    const backend = new MockBackend();
    await stubSetup(backend, (setup) => ({
      encrypt: result({ at, ...encrypt }),
      doNotForward: result({ at, ...doNotForward }),
      labels: setup.labels.map((l) =>
        labels[l.name] ? { ...l, check: result({ at, ...labels[l.name] }) } : l,
      ),
    }));
    await renderField(backend);
    return backend;
  }

  const noteOf = (name: string) =>
    row(name).querySelector(".protection-row__note")?.textContent ?? null;

  it("says when the test arrived without it, and nothing more is known", async () => {
    await withChecks({ state: "notApplied" }, {});
    expect(statusOf("Encrypt")).toHaveTextContent(`Arrived without it · checked ${on}`);
    expect(noteOf("Encrypt")).toBe(
      "The test message arrived without it, as far as Cumulas Mail can tell. New mail flow rules can take 30 minutes or more to start working, and auto-labeling policies up to a day." +
        IT,
    );
  });

  it("says when the server got the request but nothing acted on it, or it was removed", async () => {
    await withChecks(
      { state: "notApplied", note: "requestIgnored" },
      { state: "notApplied", note: "requestRemoved" },
      {
        General: { state: "notApplied", note: "requestIgnored" },
        Public: { state: "notApplied", note: "requestRemoved" },
      },
    );
    expect(statusOf("Encrypt")).toHaveTextContent(`Arrived without it · checked ${on}`);
    expect(noteOf("Encrypt")).toBe(
      "Your organization's mail server got the request (x-cumulas-protect was still on the test message), but no rule acted on it. The rule “Cumulas Mail: Encrypt” may be missing, turned off, in test mode, or new: rules can take 30 minutes or more to start working." +
        IT,
    );
    expect(noteOf("Do Not Forward")).toBe(
      "The request was removed on the way, but the message wasn't encrypted. Usually “Cumulas Mail: Encrypt” or “Cumulas Mail: Do Not Forward” removed it while its template couldn't be applied, or another rule removed x-cumulas-protect." +
        IT,
    );
    expect(noteOf("General")).toBe(
      "Your organization's mail server got the request (x-cumulas-label was still on the test message), but no auto-labeling policy applied the label. The policy may be missing, still in simulation, not include you, not be in effect yet (allow up to a day), or need a license your plan doesn't have." +
        IT,
    );
    expect(noteOf("Public")).toBe(
      "The request was removed on the way, but the message wasn't labeled. Usually a rule removed it, such as “Cumulas Mail: ignore label requests from outside” when the message didn't count as from inside your organization." +
        IT,
    );
  });

  it("says when the Cumulas Signatures relay hands mail back as outside mail", async () => {
    await withChecks(
      { state: "notApplied", note: "relayNotInternal" },
      { state: "applied", passedAt: at },
      { General: { state: "notApplied", note: "relayNotInternal" } },
    );
    expect(noteOf("Encrypt")).toBe(
      "Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back. Cumulas Mail's encryption rules then need to come before the relay's rule: Set up or check for your organization does that." +
        IT,
    );
    expect(statusOf("Do Not Forward")).toHaveTextContent(`Arrived encrypted · checked ${on}`);
    expect(noteOf("General")).toBe(
      "Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back, so labels can't work yet. They need Cumulas Signatures to count mail coming back from its server as mail from inside your organization: your IT team turns that on in Cumulas Signatures where it's offered (Finish setting up); otherwise ask Cumulas." +
        IT,
    );
  });

  it("says when the check ran too soon after a setup change, and how long after", async () => {
    await withChecks(
      { state: "notApplied", note: "tooSoon", changedAt: "2026-10-12T09:20:00Z" },
      { state: "notApplied", note: "relayNotInternal", changedAt: "2026-10-12T07:30:00Z" },
    );
    expect(statusOf("Encrypt")).toHaveTextContent(`Arrived without it · checked ${on}`);
    expect(noteOf("Encrypt")).toBe(
      "Your organization's setup changed 10 minutes before this check. New rules can take 30 minutes or more to start working, and auto-labeling policies up to a day. Check again later.",
    );
    // Another note says what it found, then when the setup changed.
    expect(noteOf("Do Not Forward")).toBe(
      "Your mail goes through the Cumulas Signatures relay, and Microsoft 365 treats it as outside mail when it comes back. Cumulas Mail's encryption rules then need to come before the relay's rule: Set up or check for your organization does that. Your organization's setup changed 2 hours before this check; if that's under an hour (a day for labels), check again later." +
        IT,
    );
  });

  it("names the label that arrived instead, and says when the organization labels all mail", async () => {
    const backend = new MockBackend();
    const labels = (await backend.getProtectionSetup("acct-work")).labels;
    const confidential = labels.find((l) => l.name === "All Employees")!;
    await withChecks(
      {},
      {},
      {
        General: { state: "notApplied", note: "otherLabel", otherLabelId: confidential.id },
        Public: {
          state: "notApplied",
          note: "otherLabel",
          otherLabelId: "00000000-0000-4000-8000-00000000abcd",
          otherLabelOnControl: true,
        },
      },
    );
    expect(statusOf("General")).toHaveTextContent(`Arrived with another label · checked ${on}`);
    expect(noteOf("General")).toBe("It arrived labeled Confidential › All Employees instead.");
    expect(noteOf("Public")).toBe(
      "It arrived labeled a different label of your organization instead. Your organization puts that label on all mail.",
    );
  });

  it("says when the test arrived protected another way, or its label couldn't be read", async () => {
    await withChecks(
      { state: "unknown", note: "otherProtection" },
      {},
      { General: { state: "unknown", note: "labelUnreadable" } },
    );
    expect(statusOf("Encrypt")).toHaveTextContent(`Couldn't tell · ${on}`);
    expect(noteOf("Encrypt")).toBe(
      "The test message arrived protected another way (S/MIME, or as a link to Microsoft's encrypted message portal), so Cumulas Mail can't tell whether your organization's rule did it. Open the test message to see.",
    );
    expect(noteOf("General")).toBe(
      "The test message arrived encrypted, but Cumulas Mail couldn't read its label. Open the test message to see which label it has.",
    );
  });

  it("asks for a check again when the mail now goes through the relay", async () => {
    await withChecks({ state: "expired", note: "relayChanged" }, { state: "expired" });
    expect(statusOf("Encrypt")).toHaveTextContent(/^Check again to use it$/);
    expect(noteOf("Encrypt")).toBe(
      "Your organization's mail now goes through the Cumulas Signatures relay, which it didn't when this was checked.",
    );
    // Only a pass that's too old says so.
    expect(statusOf("Do Not Forward")).toHaveTextContent(
      `Checked over 90 days ago (${on}). Check again to use it.`,
    );
  });

  it("copies what the checks found for IT", async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async (_text: string) => undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    onTestFinished(() => void vi.unstubAllGlobals());
    const backend = await withChecks({ state: "notApplied" }, {});
    const details = vi
      .spyOn(backend, "getProtectionCheckDetails")
      .mockResolvedValue("Cumulas Mail 0.5.0: setup check details for alex.morgan@acme.example");

    await user.click(screen.getByRole("button", { name: "Copy details for IT" }));
    expect(details).toHaveBeenCalledWith("acct-work");
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        "Cumulas Mail 0.5.0: setup check details for alex.morgan@acme.example",
      ),
    );
    await waitFor(() =>
      expect(useToasts.getState().toasts.map((t) => t.message)).toEqual([
        "Copied the check's details",
      ]),
    );
  });

  it("says when the details couldn't be copied", async () => {
    const user = userEvent.setup();
    vi.stubGlobal("navigator", { ...navigator, clipboard: undefined });
    onTestFinished(() => void vi.unstubAllGlobals());
    await withChecks({ state: "applied", passedAt: at }, {});
    await user.click(screen.getByRole("button", { name: "Copy details for IT" }));
    await waitFor(() =>
      expect(useToasts.getState().toasts.map((t) => t.message)).toEqual([
        "Copying isn't available here.",
      ]),
    );
  });

  it("opens Set up or check for your organization from its button", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend();
    await renderField(backend);
    const entry = screen.getByRole("button", { name: "Set up or check for your organization…" });
    expect(entry).toHaveAttribute("aria-haspopup", "dialog");
    await user.click(entry);
    const dialog = await screen.findByRole("dialog", {
      name: "Set up or check for your organization",
    });
    // Re-approving Cumulas Mail's own permissions needs no setup sign-in.
    expect(
      await within(dialog).findByRole("region", { name: "Cumulas Mail's permissions" }),
    ).toBeInTheDocument();
    await user.click(within(dialog).getAllByRole("button", { name: "Close" })[0]!);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(backend.calls.orgSetupEnded).toBe(1);
    await waitFor(() => expect(entry).toHaveFocus());
  });
});

describe("where it shows", () => {
  async function renderAccounts(options: MockOptions = {}, change?: (a: Account[]) => void) {
    const backend = new MockBackend(options);
    if (change) change((backend as unknown as { state: { accounts: Account[] } }).state.accounts);
    renderWithBackend(<AccountsSection onRemove={() => undefined} />, { backend });
    return userEvent.setup();
  }

  it("shows for work accounts only, not for IMAP ones", async () => {
    const user = await renderAccounts();
    await user.click(await screen.findByRole("button", { name: "Manage Alex Morgan" }));
    expect(await screen.findByText("Sensitivity labels and encryption")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Manage Alex (Personal)" }));
    await screen.findByText("Password and servers");
    expect(screen.queryByText("Sensitivity labels and encryption")).not.toBeInTheDocument();
  });

  it("doesn't show for personal Microsoft accounts", async () => {
    const user = await renderAccounts({}, (accounts) => {
      accounts[0]!.settings = {
        kind: "microsoft",
        tenantId: "9188040d-6c67-4c5b-b112-36a304b66dad",
        userId: "u",
      };
    });
    await user.click(await screen.findByRole("button", { name: "Manage Alex Morgan" }));
    await screen.findByText("Sync");
    expect(screen.queryByText("Sensitivity labels and encryption")).not.toBeInTheDocument();
  });
});
