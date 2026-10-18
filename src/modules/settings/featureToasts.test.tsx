import { act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { MockBackend } from "../../api/mock";
import { qk } from "../../api/queries";
import type { Account } from "../../api/types";
import { useAppStore } from "../../app/store";
import { useBackendEvents } from "../../app/useBackendEvents";
import { useToasts } from "../../components/toast";
import { renderWithBackend } from "../../test/render";
import { AddAccountDialog } from "./AddAccountDialog";
import { openAddAccount, openTurnOn, useAddAccountRequest } from "./addAccountStore";
import { useFeatureChangeToasts } from "./featureToasts";

const EMAIL = "alex.morgan@acme.example";

function Watcher() {
  useBackendEvents();
  useFeatureChangeToasts();
  return null;
}

const messages = () => useToasts.getState().toasts.map((t) => t.message);

/** Renders the watcher and waits until it has seen the accounts once. */
async function watch(backend: MockBackend) {
  const utils = renderWithBackend(
    <>
      <Watcher />
      <AddAccountDialog />
    </>,
    { backend },
  );
  await waitFor(() => expect(utils.client.getQueryData(qk.accounts())).toBeDefined());
  const accounts = () => utils.client.getQueryData<Account[]>(qk.accounts()) ?? [];
  const work = () => accounts().find((a) => a.id === "acct-work")!;
  return { ...utils, accounts, work };
}

beforeEach(() => {
  useAppStore.setState({ addAccountOpen: false, settingsOpen: null });
  useAddAccountRequest.setState({ request: { step: "choose" } });
  useToasts.setState({ toasts: [] });
});

describe("useFeatureChangeToasts", () => {
  it("says once when a feature stops working, and when it's back", async () => {
    const backend = new MockBackend({ missing: ["tasks"] });
    const { work } = await watch(backend);
    // What the accounts were like at first isn't news.
    expect(messages()).toEqual([]);

    act(() => backend.revokeFeature("acct-work", "contacts"));
    await waitFor(() =>
      expect(messages()).toEqual([
        `Cumulas Mail can no longer open the contacts of ${EMAIL}. See Settings → Accounts.`,
      ]),
    );

    // Refused instead of missing: it still doesn't work, which was said.
    act(() => backend.revokeFeature("acct-work", "contacts", "refused"));
    await waitFor(() => expect(work().refusedFeatures).toEqual(["contacts"]));

    act(() => backend.grantFeature("acct-work", "contacts"));
    await waitFor(() => expect(messages()).toContain(`Contacts are on for ${EMAIL}.`));
    expect(messages()).toHaveLength(2);
  });

  it("doesn't announce a feature turned on in the app a second time", async () => {
    const backend = new MockBackend({ missing: ["calendar"] });
    const { work } = await watch(backend);

    act(() => openTurnOn(EMAIL, "calendar"));
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    await waitFor(() => expect(work().needsPermission).toEqual([]));
    // Time for the watcher to see it too.
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expect(messages()).toEqual([`Turned on calendar for ${EMAIL}.`]);
  });

  it("doesn't announce refused features as back when signing in again checks them", async () => {
    // Tasks: the other tests here turn the calendar on, which is remembered for a while.
    const backend = new MockBackend({ refused: ["tasks"] });
    const { work } = await watch(backend);

    act(() => openAddAccount({ step: "microsoft", email: EMAIL, autoStart: true }));
    await waitFor(() => expect(useAppStore.getState().addAccountOpen).toBe(false));
    await waitFor(() => expect(work().refusedFeatures).toEqual([]));
    // Time for the watcher to see it too.
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expect(messages()).toEqual([`You're signed in again as ${EMAIL}.`]);
  });

  it("says nothing when sensitivity labels lack permission, only when they're refused", async () => {
    const backend = new MockBackend();
    const { work } = await watch(backend);

    // Turned on by themselves: lacking their permission isn't news.
    act(() => backend.revokeFeature("acct-work", "sensitivityLabels"));
    await waitFor(() => expect(work().needsPermission).toEqual(["sensitivityLabels"]));
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expect(messages()).toEqual([]);

    act(() => backend.revokeFeature("acct-work", "sensitivityLabels", "refused"));
    await waitFor(() =>
      expect(messages()).toEqual([
        `Microsoft 365 refused access to the sensitivity labels of ${EMAIL}. See Settings → Accounts.`,
      ]),
    );

    // Refused, then without permission: not back.
    act(() => backend.revokeFeature("acct-work", "sensitivityLabels"));
    await waitFor(() => expect(work().needsPermission).toEqual(["sensitivityLabels"]));
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expect(messages()).toHaveLength(1);

    act(() => backend.revokeFeature("acct-work", "sensitivityLabels", "refused"));
    await waitFor(() => expect(messages()).toHaveLength(2));
    act(() => backend.grantFeature("acct-work", "sensitivityLabels"));
    await waitFor(() => expect(messages()).toContain(`Sensitivity labels are on for ${EMAIL}.`));
  });

  it("says nothing about accounts added or removed", async () => {
    const backend = new MockBackend();
    const { accounts } = await watch(backend);

    await act(() => backend.addMicrosoftAccount("sam@contoso.example", { kind: "mailOnly" }));
    await waitFor(() => expect(accounts()).toHaveLength(3));
    await act(() => backend.removeAccount("acct-work"));
    await waitFor(() => expect(accounts()).toHaveLength(2));
    expect(messages()).toEqual([]);
  });
});
