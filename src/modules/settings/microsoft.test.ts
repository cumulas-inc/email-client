import { describe, expect, it } from "vitest";

import type { Account } from "../../api/types";
import {
  MICROSOFT_PERMISSIONS,
  adminConsentEmail,
  blockedSignInEmail,
  classifyAadsts,
  emailDomain,
  featureList,
  isConsumerDomain,
  isPersonalAccount,
  linkIncludesPush,
  refusedFeatureEmail,
  refusedFeatureMailto,
} from "./microsoft";

const URL_WITHOUT_PUSH =
  "https://login.microsoftonline.com/contoso.example/v2.0/adminconsent?client_id=abc&scope=openid+https%3A%2F%2Fgraph.microsoft.com%2FMail.ReadWrite&redirect_uri=http%3A%2F%2Flocalhost";
const URL_WITH_PUSH =
  "https://login.microsoftonline.com/contoso.example/v2.0/adminconsent?client_id=abc&scope=openid+api%3A%2F%2Fabc%2FSync.Receive&redirect_uri=https%3A%2F%2Fflatpak.cumulas.co%2Fadmin-consent%2F";
const ADMIN_PAGE = "https://flatpak.cumulas.co/admin-consent/";
const ROLES =
  "a Global Administrator, Privileged Role Administrator, Cloud Application Administrator or Application Administrator can either:";
const PORTAL =
  "- or, if Cumulas Mail is already listed in the Microsoft Entra admin center, go to Enterprise apps > Cumulas Mail > Permissions and select Grant admin consent.";

function microsoftAccount(tenantId: string): Account {
  return {
    id: "a",
    kind: "microsoft",
    displayName: "Sam",
    email: "sam@contoso.example",
    color: "#0b7a75",
    settings: { kind: "microsoft", tenantId, userId: "u" },
    createdAt: "2026-10-01T09:00:00Z",
    lastSyncedAt: null,
    syncError: null,
    needsPermission: [],
    refusedFeatures: [],
  };
}

describe("Microsoft's permissions", () => {
  it("list what mail needs first, then one optional permission per feature", () => {
    expect(MICROSOFT_PERMISSIONS.map((p) => [p.scopes[0], p.feature])).toEqual([
      ["Mail.ReadWrite", undefined],
      ["Mail.Send", undefined],
      ["User.Read", undefined],
      ["offline_access", undefined],
      ["MailboxSettings.Read", "mailboxSettings"],
      ["Calendars.ReadWrite", "calendar"],
      ["Contacts.ReadWrite", "contacts"],
      ["Tasks.ReadWrite", "tasks"],
    ]);
  });
});

describe("consumer domains and personal accounts", () => {
  it.each([
    "outlook.com",
    "Hotmail.com",
    "hotmail.co.uk",
    "live.com.au",
    "outlook.de",
    "live.nl",
    "msn.com",
    "passport.com",
    "windowslive.com",
    " OUTLOOK.COM ",
  ])("%s is a consumer domain", (domain) => {
    expect(isConsumerDomain(domain)).toBe(true);
  });

  it.each([
    "contoso.com",
    "live.unc.edu",
    "outlook.contoso.com",
    "hotmail.example",
    "mylive.com",
    "organizations",
    "",
  ])("%j isn't", (domain) => {
    expect(isConsumerDomain(domain)).toBe(false);
  });

  it("finds an address's domain", () => {
    expect(emailDomain("sam@contoso.example")).toBe("contoso.example");
    expect(emailDomain("odd@name@outlook.com ")).toBe("outlook.com");
    expect(emailDomain("no-domain")).toBeUndefined();
    expect(emailDomain("sam@")).toBeUndefined();
    expect(emailDomain(undefined)).toBeUndefined();
  });

  it("knows personal accounts by their tenant", () => {
    expect(isPersonalAccount(microsoftAccount("9188040d-6c67-4c5b-b112-36a304b66dad"))).toBe(true);
    expect(isPersonalAccount(microsoftAccount("9188040D-6C67-4C5B-B112-36A304B66DAD"))).toBe(true);
    expect(isPersonalAccount(microsoftAccount("consumers"))).toBe(true);
    expect(isPersonalAccount(microsoftAccount("Consumers"))).toBe(true);
    expect(isPersonalAccount(microsoftAccount("72f988bf-86f1-41af-91ab-2d7cd011db47"))).toBe(false);
    expect(isPersonalAccount(microsoftAccount("organizations"))).toBe(false);
  });
});

describe("classifyAadsts", () => {
  it.each([
    [50105, { kind: "blocked", reason: "notAssigned" }],
    [7000112, { kind: "blocked", reason: "appDisabled" }],
    [65005, { kind: "blocked", reason: "appNotSetUp" }],
    [650052, { kind: "blocked", reason: "serviceNotEnabled" }],
    [53004, { kind: "blocked", reason: "securityInfo" }],
    [53010, { kind: "blocked", reason: "securityInfo" }],
    [53000, { kind: "blocked", reason: "conditionalAccess" }],
    [53001, { kind: "blocked", reason: "conditionalAccess" }],
    [53002, { kind: "blocked", reason: "conditionalAccess" }],
    [53003, { kind: "blocked", reason: "conditionalAccess" }],
    [53011, { kind: "blocked", reason: "conditionalAccess" }],
    [530000, { kind: "blocked", reason: "conditionalAccess" }],
    [530032, { kind: "blocked", reason: "conditionalAccess" }],
    [530999, { kind: "blocked", reason: "conditionalAccess" }],
    [65001, { kind: "adminConsent", canRequestApproval: false }],
    [90093, { kind: "adminConsent", canRequestApproval: false }],
    [90094, { kind: "adminConsent", canRequestApproval: false }],
    [90095, { kind: "adminConsent", canRequestApproval: true }],
    [900941, { kind: "adminConsent", canRequestApproval: false }],
    [900981, { kind: "adminConsent", canRequestApproval: false }],
    [65004, { kind: "declined" }],
    [12345, null],
    [531000, null],
    [53005, null],
  ])("AADSTS%i", (code, expected) => {
    expect(classifyAadsts(code)).toEqual(expected);
  });
});

describe("the approval link", () => {
  it("covers push when its scope names Sync.Receive", () => {
    expect(linkIncludesPush(URL_WITH_PUSH)).toBe(true);
    expect(linkIncludesPush(URL_WITHOUT_PUSH)).toBe(false);
    expect(linkIncludesPush("not a link")).toBe(false);
  });

  it("lists features as a sentence would", () => {
    expect(featureList([])).toBe("");
    expect(featureList(["calendar"])).toBe("calendar");
    expect(featureList(["calendar", "tasks"])).toBe("calendar and tasks");
    expect(featureList(["calendar", "contacts", "tasks", "mailboxSettings"])).toBe(
      "calendar, contacts, tasks and mailbox time zone",
    );
  });
});

describe("adminConsentEmail", () => {
  it("asks to approve signing in, with the roles, the link and the admin center", () => {
    const { subject, body } = adminConsentEmail({
      adminConsentUrl: URL_WITHOUT_PUSH,
      email: "sam@contoso.example",
      tenant: "contoso.example",
    });
    expect(subject).toBe("Please approve Cumulas Mail for Microsoft 365");
    const lines = body.split("\r\n");
    expect(lines[0]).toBe("Hello,");
    expect(lines[2]).toBe(
      "I'd like to use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account (sam@contoso.example). When I sign in, Microsoft says an administrator needs to approve the app first.",
    );
    expect(body).toContain(`To approve it for contoso.example, ${ROLES}`);
    expect(body).toContain(`- open this link and sign in: ${URL_WITHOUT_PUSH}`);
    expect(body).toContain(PORTAL);
    expect(body).toContain("- Read and write my mail (Mail.ReadWrite)");
    expect(body).toContain("- Read and write my tasks (Tasks.ReadWrite)");
    expect(body).not.toContain("Sync.Receive");
    expect(lines.at(-1)).toBe("Thank you!");
  });

  it("names the publisher and the page for administrators only with that page", () => {
    const published = adminConsentEmail({
      adminConsentUrl: URL_WITH_PUSH,
      adminPageUrl: ADMIN_PAGE,
    }).body;
    expect(published).toContain(
      "Cumulas Mail is published by Cumulas Inc, a Microsoft-verified publisher. It only asks for delegated permissions",
    );
    expect(published).toContain(
      `More for administrators, including how to let people approve Cumulas Mail for themselves: ${ADMIN_PAGE}\r\n\r\nThank you!`,
    );

    const own = adminConsentEmail({ adminConsentUrl: URL_WITHOUT_PUSH, adminPageUrl: null }).body;
    expect(own).not.toContain("Microsoft-verified publisher");
    expect(own).not.toContain("More for administrators");
    expect(own).toContain(
      "\r\nIt only asks for delegated permissions: it acts as the signed-in person and can only reach their own mail, calendar, contacts and tasks, never anyone else's.\r\n",
    );
    expect(own).toContain(`To approve it, ${ROLES}`);
    expect(own).toContain(PORTAL);
  });

  it("lists the push permission when the link covers it", () => {
    const push =
      "- Receive notices that my mail or calendar changed, through the Cumulas push service (Sync.Receive, on Cumulas Mail's own API)";
    expect(adminConsentEmail({ adminConsentUrl: URL_WITH_PUSH }).body).toContain(push);
    expect(adminConsentEmail({ adminConsentUrl: URL_WITHOUT_PUSH }).body).not.toContain(push);
    expect(
      adminConsentEmail({ adminConsentUrl: URL_WITHOUT_PUSH, purpose: "push" }).body,
    ).toContain(push);
  });

  it("asks for features when mail works already", () => {
    const one = adminConsentEmail({
      adminConsentUrl: URL_WITHOUT_PUSH,
      email: "sam@contoso.example",
      purpose: "features",
      features: ["calendar"],
    });
    expect(one.subject).toBe("Please approve Cumulas Mail for Microsoft 365");
    expect(one.body).toContain(
      "I use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account (sam@contoso.example). My mail works, but Cumulas Mail needs an administrator's approval to use my calendar.",
    );
    const several = adminConsentEmail({
      adminConsentUrl: URL_WITHOUT_PUSH,
      purpose: "features",
      features: ["calendar", "contacts", "tasks"],
    });
    expect(several.body).toContain(
      "with my Microsoft 365 account. My mail works, but Cumulas Mail needs an administrator's approval to use my calendar, contacts and tasks.",
    );
  });
});

describe("blockedSignInEmail", () => {
  const request = { aadsts: 50105, email: "sam@contoso.example", adminPageUrl: ADMIN_PAGE };

  it.each([
    [
      "notAssigned",
      "Microsoft says I haven't been assigned to the app (AADSTS50105). Could you assign me? In the Microsoft Entra admin center: Enterprise apps > Cumulas Mail > Users and groups > Add user/group.",
    ],
    [
      "appDisabled",
      "Microsoft says the app is disabled in our organization (AADSTS50105). If we may use it, it can be turned back on in the Microsoft Entra admin center: Enterprise apps > Cumulas Mail > Properties > Enabled for users to sign in.",
    ],
    [
      "appNotSetUp",
      "Microsoft says the app isn't set up in our organization (AADSTS50105). Approving it may fix this.",
    ],
    [
      "serviceNotEnabled",
      "Microsoft says our organization hasn't subscribed to or enabled a service the app needs (AADSTS50105), usually Exchange Online. Could you check our subscription?",
    ],
    [
      "securityInfo",
      "Microsoft says I need to finish setting up my sign-in security info, such as multifactor authentication (AADSTS50105), and I couldn't do that during sign-in. Could you help me set it up?",
    ],
    [
      "conditionalAccess",
      "Our sign-in rules blocked the sign-in (AADSTS50105). Could you tell me what's required, or whether Cumulas Mail can be allowed? The Entra sign-in logs show which policy applied.",
    ],
  ] as const)("explains %s", (reason, sentence) => {
    const { subject, body } = blockedSignInEmail({ ...request, reason });
    expect(subject).toBe("Cumulas Mail sign-in is blocked (AADSTS50105)");
    expect(body.split("\r\n")).toEqual([
      "Hello,",
      "",
      `I'd like to use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account (sam@contoso.example). ${sentence}`,
      "",
      `More for administrators: ${ADMIN_PAGE}`,
      "",
      "Thank you!",
    ]);
  });

  it("gives the approval link to an organization that isn't set up", () => {
    const { body } = blockedSignInEmail({
      reason: "appNotSetUp",
      aadsts: 65005,
      adminConsentUrl: URL_WITHOUT_PUSH,
      adminPageUrl: null,
    });
    expect(body).toContain(
      `Microsoft says the app isn't set up in our organization (AADSTS65005). Approving it may fix this; an administrator can open this link and sign in: ${URL_WITHOUT_PUSH}`,
    );
    expect(body).not.toContain("More for administrators");
  });
});

describe("refusedFeatureEmail", () => {
  it("asks IT to check a mailbox Microsoft 365 refuses", () => {
    const { subject, body } = refusedFeatureEmail({
      email: "sam@contoso.example",
      features: ["calendar", "tasks"],
    });
    expect(subject).toBe("Cumulas Mail can't open my calendar and tasks");
    expect(body.split("\r\n")).toEqual([
      "Hello,",
      "",
      "I use Cumulas Mail, an email and calendar app for Linux, with my Microsoft 365 account (sam@contoso.example). It has permission to use my calendar and tasks, but Microsoft 365 refuses access (HTTP 403). This often means my mailbox doesn't include that feature, for example because of my license, or that a policy blocks it.",
      "",
      "Could you check? Thank you!",
    ]);
    expect(refusedFeatureMailto({ features: ["tasks"] })).toMatch(
      /^mailto:\?subject=Cumulas%20Mail%20can't%20open%20my%20tasks&body=/,
    );
  });
});
