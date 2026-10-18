import { describe, expect, it } from "vitest";

import type {
  CheckResult,
  LabelOption,
  MessageLabel,
  ProtectionSetup,
  ReplyKind,
} from "../../api/types";
import {
  canChooseLabel,
  hasProtectionNotice,
  initialLabelId,
  isEncrypted,
  labelRequired,
  labelTitle,
  labelsToCheck,
  noticeName,
  offeredActions,
  offeredLabels,
  sendProblem,
} from "./protection";

const NOT_CHECKED: CheckResult = { state: "notChecked", at: null, note: null, testMessage: false };
const APPLIED: CheckResult = {
  state: "applied",
  at: "2026-10-16T09:00:00Z",
  note: null,
  testMessage: true,
};

function label(id: string, sensitivity: number | null, check = APPLIED, extra = {}): LabelOption {
  return {
    id,
    name: id[0]!.toUpperCase() + id.slice(1),
    parentName: null,
    tooltip: null,
    color: null,
    hasProtection: false,
    sensitivity,
    check,
    ...extra,
  };
}

function setup(patch: Partial<ProtectionSetup> = {}): ProtectionSetup {
  return {
    accountId: "acct-work",
    supported: true,
    encrypt: APPLIED,
    doNotForward: NOT_CHECKED,
    labels: [
      label("public", 0),
      label("general", 1),
      label("confidential", 2),
      label("secret", 3, { ...NOT_CHECKED }),
    ],
    policy: {
      required: true,
      defaultLabelId: "general",
      justifyDowngrade: true,
      moreInfoUrl: null,
    },
    labelsListedAt: "2026-10-18T08:00:00Z",
    labelsError: null,
    checking: false,
    ...patch,
  };
}

const named = (id: string, name: string | null = null): MessageLabel => ({
  id,
  name,
  color: null,
  fromCatalog: name !== null,
});

describe("what the composer offers", () => {
  it("offers only what a check found works, and nothing on unsupported accounts", () => {
    expect(offeredActions(setup())).toEqual(["encrypt"]);
    expect(offeredActions(setup({ doNotForward: APPLIED }))).toEqual(["encrypt", "doNotForward"]);
    expect(offeredActions(setup({ encrypt: { ...APPLIED, state: "expired" } }))).toEqual([]);
    expect(offeredLabels(setup()).map((l) => l.id)).toEqual(["public", "general", "confidential"]);
    expect(offeredActions(setup({ supported: false }))).toEqual([]);
    expect(offeredLabels(setup({ supported: false }))).toEqual([]);
    expect(offeredActions(undefined)).toEqual([]);
    expect(offeredLabels(undefined)).toEqual([]);
  });

  it("requires a label only when one can be applied", () => {
    expect(labelRequired(setup())).toBe(true);
    const none = setup({ labels: [label("secret", 3, NOT_CHECKED)] });
    expect(labelRequired(none)).toBe(false);
    expect(labelRequired(setup({ policy: { ...setup().policy, required: false } }))).toBe(false);
    expect(labelRequired(undefined)).toBe(false);
  });

  it("titles child labels with their parent", () => {
    expect(labelTitle({ name: "All Employees", parentName: "Confidential" })).toBe(
      "Confidential › All Employees",
    );
    expect(labelTitle({ name: "General", parentName: null })).toBe("General");
  });

  it("starts with the original's label when offered, else the default for unlabeled mail", () => {
    expect(initialLabelId(setup(), null)).toBe("general");
    expect(initialLabelId(setup(), named("confidential"))).toBe("confidential");
    // Labeled with one Cumulas Mail can't apply: no default over it.
    expect(initialLabelId(setup(), named("secret"))).toBeNull();
    const defaultUnchecked = setup({
      policy: { ...setup().policy, defaultLabelId: "secret" },
    });
    expect(initialLabelId(defaultUnchecked, null)).toBeNull();
    expect(
      initialLabelId(setup({ policy: { ...setup().policy, defaultLabelId: null } }), null),
    ).toBeNull();
  });

  it("won't lower a reply's label when that needs a reason", () => {
    const s = setup();
    const [publicLabel, generalLabel, confidential] = s.labels;
    const original = named("general");
    expect(canChooseLabel(s, original, publicLabel!)).toBe(false);
    expect(canChooseLabel(s, original, null)).toBe(false);
    expect(canChooseLabel(s, original, generalLabel!)).toBe(true);
    expect(canChooseLabel(s, original, confidential!)).toBe(true);
    // Unknown sensitivities don't restrict, nor does a new message.
    expect(canChooseLabel(s, original, label("other", null))).toBe(true);
    const unknownOriginal = setup({ labels: [label("general", null), label("public", 0)] });
    expect(canChooseLabel(unknownOriginal, original, unknownOriginal.labels[1]!)).toBe(true);
    expect(canChooseLabel(s, named("not-listed"), publicLabel!)).toBe(true);
    expect(canChooseLabel(s, null, null)).toBe(true);
    const free = setup({ policy: { ...s.policy, justifyDowngrade: false } });
    expect(canChooseLabel(free, original, null)).toBe(true);
  });

  it("checks labels never checked first, then expired ones, then the oldest, 19 at most", () => {
    const at = (day: number) => `2026-10-${String(day).padStart(2, "0")}T09:00:00Z`;
    const s = setup({
      labels: [
        label("recent", 1, { ...APPLIED, at: at(17) }),
        label("old", 1, { ...APPLIED, at: at(2) }),
        label("expired", 1, { ...APPLIED, state: "expired", at: at(1) }),
        label("new", 1, NOT_CHECKED),
        label("failed", 1, { ...APPLIED, state: "notApplied", at: at(10) }),
        label("waiting", 1, { ...APPLIED, state: "waiting", at: at(18) }),
        label("new2", 1, NOT_CHECKED),
      ],
    });
    expect(labelsToCheck(s).map((l) => l.id)).toEqual([
      "new",
      "new2",
      "expired",
      "old",
      "failed",
      "recent",
    ]);
    const many = setup({
      labels: Array.from({ length: 25 }, (_, i) => label(`label${i}`, 1, NOT_CHECKED)),
    });
    expect(labelsToCheck(many)).toHaveLength(19);
    expect(labelsToCheck(many)[0]!.id).toBe("label0");
  });
});

describe("why Send waits", () => {
  const reply = (patch: {
    labelId?: string | null;
    labelChosen?: boolean;
    originalLabel?: MessageLabel | null;
    replyKind?: ReplyKind | null;
  }) => ({
    labelId: null,
    labelChosen: false,
    originalLabel: null,
    replyKind: "reply" as ReplyKind | null,
    ...patch,
  });
  const notRequired = setup({ policy: { ...setup().policy, required: false } });

  it("needs a label the organization requires", () => {
    expect(sendProblem(reply({ replyKind: null }), setup())).toEqual({ kind: "labelRequired" });
    expect(sendProblem(reply({ replyKind: null, labelId: "general" }), setup())).toBeNull();
    // Nothing can be applied yet: nothing to choose.
    expect(sendProblem(reply({}), setup({ labels: [] }))).toBeNull();
  });

  it("makes the person choose when the original's label can't be kept and one is required", () => {
    const original = named("secret", "Secret");
    expect(sendProblem(reply({ originalLabel: original }), setup())).toEqual({
      kind: "chooseLabel",
      name: "Secret",
    });
    expect(
      sendProblem(
        reply({ originalLabel: original, labelId: "general", labelChosen: true }),
        setup(),
      ),
    ).toBeNull();
    // The original's own label, offered: nothing to say.
    expect(
      sendProblem(reply({ originalLabel: named("general"), labelId: "general" }), setup()),
    ).toBeNull();
  });

  it("asks before dropping the original's label when none is required", () => {
    const original = named("secret");
    expect(sendProblem(reply({ originalLabel: original }), notRequired)).toEqual({
      kind: "originalLabelDropped",
      name: null,
    });
    expect(
      sendProblem(reply({ originalLabel: original, labelId: "confidential" }), notRequired),
    ).toEqual({ kind: "originalLabelDropped", name: null });
    // New messages have no original.
    expect(
      sendProblem(reply({ originalLabel: original, replyKind: null }), notRequired),
    ).toBeNull();
  });

  it("says nothing to people who don't use labels from Cumulas Mail", () => {
    const none = setup({ labels: [label("secret", 3, NOT_CHECKED)] });
    expect(sendProblem(reply({ originalLabel: named("general", "General") }), none)).toBeNull();
    expect(sendProblem(reply({ originalLabel: named("general") }), undefined)).toBeNull();
  });
});

describe("protection notices", () => {
  it("strips bidi controls from names, and drops names that could pass for an address", () => {
    expect(noticeName("IT Service Desk\u202e", "it@acme.example")).toBe("IT Service Desk");
    expect(noticeName("\u2067Dana\u2069 \u200eWhitfield", "dana@acme.example")).toBe(
      "Dana Whitfield",
    );
    expect(noticeName("IT Service Desk <it@acme.example>", "alerts@notacme.example")).toBeNull();
    expect(noticeName(" DANA@acme.example ", "dana@acme.example")).toBeNull();
    expect(noticeName("Dana@Acme", "x@y.example")).toBeNull();
    expect(noticeName("dana", "dana")).toBeNull();
    expect(noticeName("  \u202e ", "dana@acme.example")).toBeNull();
    expect(noticeName(null, "dana@acme.example")).toBeNull();
  });

  it("knows encrypted mail, and when signed mail needs a notice", () => {
    const protection = (
      kind: "rightsManaged" | "smimeEncrypted" | "smimeSigned" | "portalLink",
    ) => ({
      kind,
      containerIds: [],
      portalUrl: null,
      contentHidden: false,
    });
    expect(isEncrypted(protection("rightsManaged"))).toBe(true);
    expect(isEncrypted(protection("smimeEncrypted"))).toBe(true);
    expect(isEncrypted(protection("smimeSigned"))).toBe(false);
    expect(isEncrypted(protection("portalLink"))).toBe(false);
    expect(isEncrypted(null)).toBe(false);

    const body = { bodyHtml: "<p>Hi</p>", bodyText: "Hi" };
    expect(hasProtectionNotice({ ...body, protection: protection("smimeSigned") })).toBe(false);
    expect(
      hasProtectionNotice({
        ...body,
        protection: { ...protection("smimeSigned"), contentHidden: true },
      }),
    ).toBe(true);
    expect(
      hasProtectionNotice({ bodyHtml: " ", bodyText: null, protection: protection("smimeSigned") }),
    ).toBe(true);
    expect(hasProtectionNotice({ ...body, protection: protection("portalLink") })).toBe(true);
    expect(hasProtectionNotice({ ...body, protection: null })).toBe(false);
  });
});
