// Small validators shared by the add-account form and the settings dialog.
// Each returns an error message for the user, or null when the value is fine.

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/;
const GUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const DOMAIN = /^(?=.{3,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/i;
const HOST = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$/i;

export function isEmail(value: string): boolean {
  return EMAIL.test(value.trim());
}

export function isGuid(value: string): boolean {
  return GUID.test(value.trim());
}

export function validateEmail(value: string): string | null {
  if (!value.trim()) return "Enter your email address.";
  if (!isEmail(value)) return "Enter an email address like name@example.com.";
  return null;
}

export function validateHost(value: string): string | null {
  const host = value.trim().replace(/^\[|\]$/g, "");
  if (!host) return "Enter a server name.";
  if (host === "::1") return null;
  if (!HOST.test(host)) return "Enter a server name like mail.example.com.";
  return null;
}

export function validatePort(value: string): string | null {
  const trimmed = value.trim();
  const port = Number(trimmed);
  if (!/^\d+$/.test(trimmed) || port < 1 || port > 65535) {
    return "Enter a port between 1 and 65535.";
  }
  return null;
}

export function validateClientId(value: string): string | null {
  if (!value.trim()) return null;
  if (!isGuid(value)) {
    return "This doesn't look like an application ID. It has 32 letters and digits in the form 00000000-0000-0000-0000-000000000000.";
  }
  return null;
}

export function validateTenant(value: string): string | null {
  const v = value.trim();
  if (!v) return "Enter your organisation's tenant ID or domain.";
  if (isGuid(v) || DOMAIN.test(v)) return null;
  return "Enter a tenant ID (like 00000000-0000-0000-0000-000000000000) or a domain (like contoso.com).";
}
