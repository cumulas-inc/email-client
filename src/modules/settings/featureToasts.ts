// Says when a Microsoft account's calendar, contacts, tasks, mailbox time
// zone or sensitivity labels stop working, or come back: Microsoft's answer
// can change at any sync (an administrator approved the app, a license
// changed).

import { useEffect, useRef } from "react";

import { useAccounts } from "../../api/queries";
import type { Account, AccountFeature } from "../../api/types";
import { toast } from "../../components/toast";
import { FEATURE_COPY, ON_REQUEST_FEATURES, moduleFeatures } from "./microsoft";

/** A feature turned on from the app has its own toast for this long. */
const OWN_TOAST_MS = 60_000;

/** When each account's feature was turned on from the app, by `accountId:feature`. */
const turnedOn = new Map<string, number>();

/** A Turn on just succeeded: its toast says so, so the change isn't announced again. */
export function noteFeatureTurnedOn(accountId: string, feature: AccountFeature): void {
  turnedOn.set(`${accountId}:${feature}`, Date.now());
}

function turnedOnLately(accountId: string, feature: AccountFeature): boolean {
  const at = turnedOn.get(`${accountId}:${feature}`);
  return at !== undefined && Date.now() - at < OWN_TOAST_MS;
}

/**
 * The features an account can't use now, for whatever reason. A feature
 * turned on by itself (sensitivity labels) counts only when refused: until
 * it's turned on it lacks permission, which isn't news.
 */
function unavailable(account: Account): Set<AccountFeature> {
  return new Set([...moduleFeatures(account.needsPermission), ...account.refusedFeatures]);
}

/**
 * Toasts when an account's feature stops or starts working, from the second
 * look at the accounts on: neither the first one nor accounts added or
 * removed since say anything. Mount once (App).
 */
export function useFeatureChangeToasts(): void {
  const { data: accounts } = useAccounts();
  const known = useRef<Map<string, Set<AccountFeature>> | null>(null);

  useEffect(() => {
    if (!accounts) return;
    const previous = known.current;
    known.current = new Map(accounts.map((a) => [a.id, unavailable(a)]));
    if (!previous) return;
    for (const account of accounts) {
      const before = previous.get(account.id);
      if (!before) continue;
      const now = known.current.get(account.id)!;
      for (const feature of now) {
        if (before.has(feature)) continue;
        const noun = FEATURE_COPY[feature].noun;
        toast.info(
          ON_REQUEST_FEATURES.includes(feature)
            ? `Microsoft 365 refused access to the ${noun} of ${account.email}. See Settings → Accounts.`
            : `Cumulas Mail can no longer open the ${noun} of ${account.email}. See Settings → Accounts.`,
        );
      }
      for (const feature of before) {
        // Refused, then without permission: not back either.
        const off = now.has(feature) || account.needsPermission.includes(feature);
        if (off || turnedOnLately(account.id, feature)) continue;
        toast.success(`${FEATURE_COPY[feature].isOn} for ${account.email}.`);
      }
    }
  }, [accounts]);
}
