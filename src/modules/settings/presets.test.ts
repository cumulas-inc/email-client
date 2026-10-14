import { describe, expect, it } from "vitest";

import { adminConsentEmail, adminConsentMailto, organisationName } from "./microsoft";
import { emailDomain, findPreset, isLoopbackHost, suggestedSettings } from "./presets";
import { validateClientId, validatePort, validateTenant } from "./validation";

describe("provider presets", () => {
  it.each([
    ["sam@gmail.com", "imap.gmail.com", 993, "tls", "smtp.gmail.com", 465, "tls"],
    ["sam@googlemail.com", "imap.gmail.com", 993, "tls", "smtp.gmail.com", 465, "tls"],
    ["sam@icloud.com", "imap.mail.me.com", 993, "tls", "smtp.mail.me.com", 587, "startTls"],
    ["sam@mac.com", "imap.mail.me.com", 993, "tls", "smtp.mail.me.com", 587, "startTls"],
    ["sam@yahoo.com", "imap.mail.yahoo.com", 993, "tls", "smtp.mail.yahoo.com", 465, "tls"],
    ["sam@fastmail.com", "imap.fastmail.com", 993, "tls", "smtp.fastmail.com", 465, "tls"],
    ["sam@proton.me", "127.0.0.1", 1143, "startTls", "127.0.0.1", 1025, "startTls"],
    ["sam@protonmail.com", "127.0.0.1", 1143, "startTls", "127.0.0.1", 1025, "startTls"],
    ["sam@example.org", "imap.example.org", 993, "tls", "smtp.example.org", 587, "startTls"],
    [
      "Sam@Mail.Example.ORG ",
      "imap.mail.example.org",
      993,
      "tls",
      "smtp.mail.example.org",
      587,
      "startTls",
    ],
  ])(
    "suggests servers for %s",
    (email, imapHost, imapPort, imapTls, smtpHost, smtpPort, smtpTls) => {
      expect(suggestedSettings(email)).toEqual({
        imapHost,
        imapPort,
        imapTls,
        smtpHost,
        smtpPort,
        smtpTls,
        username: email.trim(),
      });
    },
  );

  it("recognises providers that need an app password or a bridge", () => {
    expect(findPreset("a@gmail.com")?.appPassword?.term).toBe("app password");
    expect(findPreset("a@me.com")?.appPassword?.term).toBe("app-specific password");
    expect(findPreset("a@proton.me")?.bridge).toBe(true);
    expect(findPreset("a@example.org")).toBeNull();
  });

  it.each(["outlook.com", "hotmail.com", "live.com"])("suggests Microsoft for %s", (domain) => {
    expect(findPreset(`a@${domain}`)?.preferMicrosoft).toBe(true);
  });

  it("only finds a domain in complete addresses", () => {
    expect(emailDomain("sam@")).toBeNull();
    expect(emailDomain("sam@gmail")).toBeNull();
    expect(emailDomain("@gmail.com")).toBeNull();
    expect(emailDomain("sam@GMail.com")).toBe("gmail.com");
    expect(suggestedSettings("sam").imapHost).toBe("");
  });

  it("knows which hosts are on this computer", () => {
    expect(isLoopbackHost("127.0.0.1")).toBe(true);
    expect(isLoopbackHost("localhost")).toBe(true);
    expect(isLoopbackHost("[::1]")).toBe(true);
    expect(isLoopbackHost("mail.example.org")).toBe(false);
    expect(isLoopbackHost("127.example.org")).toBe(false);
  });
});

describe("validation", () => {
  it("checks ports", () => {
    expect(validatePort("993")).toBeNull();
    expect(validatePort("0")).not.toBeNull();
    expect(validatePort("70000")).not.toBeNull();
    expect(validatePort("")).not.toBeNull();
  });

  it("checks application IDs", () => {
    expect(validateClientId("")).toBeNull();
    expect(validateClientId("1b2c3d4e-0000-4a5b-8c9d-0123456789ab")).toBeNull();
    expect(validateClientId("1b2c3d4e")).toMatch(/doesn't look like an application ID/);
  });

  it("checks tenants", () => {
    expect(validateTenant("contoso.onmicrosoft.com")).toBeNull();
    expect(validateTenant("1b2c3d4e-0000-4a5b-8c9d-0123456789ab")).toBeNull();
    expect(validateTenant("contoso")).not.toBeNull();
    expect(validateTenant("")).not.toBeNull();
  });
});

describe("admin consent request", () => {
  const url = "https://login.microsoftonline.com/contoso.example/v2.0/adminconsent?client_id=abc";

  it("names the organisation only when the tenant is a domain", () => {
    expect(organisationName("contoso.example")).toBe("contoso.example");
    expect(organisationName("organizations")).toBeNull();
    expect(organisationName(undefined)).toBeNull();
  });

  it("writes a polite request with the link and every permission", () => {
    const { subject, body } = adminConsentEmail({
      adminConsentUrl: url,
      email: "sam@contoso.example",
      tenant: "contoso.example",
    });
    expect(subject).toMatch(/approve Cumulas Mail/);
    expect(body).toContain(url);
    expect(body).toContain("sam@contoso.example");
    expect(body).toContain("for contoso.example");
    for (const scope of [
      "Mail.ReadWrite",
      "Mail.Send",
      "Calendars.ReadWrite",
      "Contacts.ReadWrite",
      "Tasks.ReadWrite",
      "User.Read",
      "offline_access",
    ]) {
      expect(body).toContain(scope);
    }
    expect(body).toMatch(/Thank you/);
  });

  it("encodes the request as a mailto: link", () => {
    const mailto = adminConsentMailto({ adminConsentUrl: url });
    expect(mailto).toMatch(/^mailto:\?subject=[^&]+&body=/);
    expect(mailto).not.toContain(" ");
    expect(decodeURIComponent(mailto)).toContain(url);
  });
});
