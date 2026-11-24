import type { ImportFormat } from "../backend/types";

export interface FormatInfo { id: ImportFormat; label: string; group: "Password managers" | "KeePass" | "Browsers" | "Code apps" | "Other" }

export const FORMATS: readonly FormatInfo[] = [
  { id: "bitwarden-json", label: "Bitwarden (JSON)", group: "Password managers" },
  { id: "bitwarden-csv", label: "Bitwarden (CSV)", group: "Password managers" },
  { id: "proton-json", label: "Proton Pass (JSON)", group: "Password managers" },
  { id: "proton-zip", label: "Proton Pass (ZIP)", group: "Password managers" },
  { id: "proton-csv", label: "Proton Pass (CSV)", group: "Password managers" },
  { id: "1password-1pux", label: "1Password (1PUX)", group: "Password managers" },
  { id: "1password-csv", label: "1Password (CSV)", group: "Password managers" },
  { id: "lastpass-csv", label: "LastPass (CSV)", group: "Password managers" },
  { id: "dashlane-csv", label: "Dashlane (CSV)", group: "Password managers" },
  { id: "dashlane-json", label: "Dashlane (JSON)", group: "Password managers" },
  { id: "nordpass-csv", label: "NordPass (CSV)", group: "Password managers" },
  { id: "enpass-csv", label: "Enpass (CSV)", group: "Password managers" },
  { id: "enpass-json", label: "Enpass (JSON)", group: "Password managers" },
  { id: "keepass-kdbx", label: "KeePass database (KDBX)", group: "KeePass" },
  { id: "keepass-xml", label: "KeePass 2 (XML)", group: "KeePass" },
  { id: "chrome-csv", label: "Chrome (CSV)", group: "Browsers" },
  { id: "brave-csv", label: "Brave (CSV)", group: "Browsers" },
  { id: "edge-csv", label: "Edge (CSV)", group: "Browsers" },
  { id: "firefox-csv", label: "Firefox (CSV)", group: "Browsers" },
  { id: "safari-csv", label: "Safari (CSV)", group: "Browsers" },
  { id: "aegis-json", label: "Aegis (JSON)", group: "Code apps" },
  { id: "2fas-json", label: "2FAS (JSON)", group: "Code apps" },
  { id: "otpauth-list", label: "List of otpauth:// links", group: "Code apps" },
  { id: "google-migration", label: "Google Authenticator export", group: "Code apps" },
  { id: "custom-csv", label: "Another CSV (match the columns myself)", group: "Other" },
];

export const formatLabel = (id: ImportFormat | "unknown"): string => FORMATS.find((f) => f.id === id)?.label ?? "Not recognised";

const norm = (h: string) => h.toLowerCase().replace(/[^a-z0-9]/g, "");

/** Best guess of which column holds what. Earlier needles win over later ones, so a specific header beats a vague one. */
export function guessMapping(headers: string[]): Record<"title" | "username" | "password" | "url" | "totp" | "notes" | "folder", string | undefined> {
  const taken = new Set<string>();
  const find = (...needles: string[]) => {
    for (const n of needles) {
      const hit = headers.find((h) => !taken.has(h) && norm(h).includes(n));
      if (hit) { taken.add(hit); return hit; }
    }
    return undefined;
  };
  // Resolved in this order so that "Site Label" becomes the name before "site" is considered for the address.
  const title = find("title", "name", "label", "account", "site");
  const password = find("password", "pass", "secret");
  const username = find("username", "user", "login", "email");
  const url = find("url", "website", "link", "web", "site");
  return { title, username, password, url, totp: find("totp", "otp", "2fa", "authenticator"), notes: find("note", "extra", "comment"), folder: find("folder", "group", "category", "grouping") };
}
