// DEV-ONLY preview of the demo vault ("Hexis123"), mirroring src-tauri/src/storage/demo.rs closely enough that the
// first-run screens can be reviewed in a normal browser. One made-up person, Hexis Alden; every value is invented.
import type { ItemDraft } from "./types";

export const DEMO_NAME = "Hexis123";
export const DEMO_PASSWORD = "Hexis123";

const DAY_MIN = 24 * 60;
const SYMBOLS = "!@#$%^&*-_=+?";
const ALNUM = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

function pick(set: string, n: number): string {
  const bytes = crypto.getRandomValues(new Uint8Array(n));
  return Array.from(bytes, (b) => set[b % set.length]).join("");
}
const password = (n = 18) => pick(ALNUM + SYMBOLS, n);
const secret = () => pick(B32, 32);

export interface DemoSeed { minsAgo: number; editedMinsAgo?: number; draft: ItemDraft }

function login(title: string, username: string, url: string, folder: string, tags: string[], totpIssuer: string | null, ageDays: number, editedDays: number, pw = password()): DemoSeed {
  const fields: Record<string, string> = { username, password: pw, url };
  if (totpIssuer) fields.totp = `otpauth://totp/${totpIssuer}:${username}?secret=${secret()}&issuer=${totpIssuer}&period=30&digits=6`;
  return { minsAgo: ageDays * DAY_MIN, editedMinsAgo: editedDays * DAY_MIN, draft: { type: "login", title, favorite: false, reprompt: false, tags, folder, fields, customFields: [] } };
}

/** The persona's vault: about two dozen items, back-dated so the list reads like a vault that has been used. */
export function demoSeeds(): DemoSeed[] {
  const gmail = "hexis@gmail.com";
  const outlook = "hexis.alden@outlook.com";
  const out: DemoSeed[] = [
    login("Google", gmail, "https://accounts.google.com", "Personal", ["email"], "Google", 1103, 38),
    login("Microsoft account", outlook, "https://login.live.com", "Personal", ["email"], "Microsoft", 1021, 96),
    login("GitHub", "hexis-dev", "https://github.com/login", "Dev", ["code"], "GitHub", 978, 12),
    login("Apple ID", "hexis@icloud.com", "https://appleid.apple.com", "Personal", ["phone"], null, 940, 203),
    login("Amazon", gmail, "https://www.amazon.com/ap/signin", "Shopping", ["shopping"], null, 902, 57),
    login("Netflix", gmail, "https://www.netflix.com/login", "Entertainment", ["streaming"], null, 861, 311),
    login("Spotify", gmail, "https://accounts.spotify.com/login", "Entertainment", ["streaming"], null, 840, 150),
    login("Dropbox", gmail, "https://www.dropbox.com/login", "Personal", ["files"], "Dropbox", 770, 64),
    login("LinkedIn", outlook, "https://www.linkedin.com/login", "Work", ["career"], null, 702, 28),
    login("AWS Console", "hexis-admin", "https://signin.aws.amazon.com/console", "Work", ["cloud"], "Amazon Web Services", 655, 9),
    login("Chase Bank", "hexis.alden", "https://secure.chase.com", "Finance", ["bank"], null, 610, 21),
    login("PayPal", gmail, "https://www.paypal.com/signin", "Finance", ["payments"], "PayPal", 588, 73),
    login("Coinbase", gmail, "https://www.coinbase.com/signin", "Finance", ["crypto"], "Coinbase", 421, 5),
    login("Cloudflare", gmail, "https://dash.cloudflare.com/login", "Dev", ["code"], "Cloudflare", 390, 33),
  ];
  // the AWS root user is a second sign-in with its own password and its own one-time code
  const aws = out.find((s) => s.draft.title === "AWS Console")!;
  aws.draft.fields["username#2"] = "hexis-root";
  aws.draft.fields["password#2"] = password(20);
  aws.draft.fields["totp#2"] = `otpauth://totp/Amazon%20Web%20Services:root?secret=${pick(B32, 16)}&issuer=Amazon%20Web%20Services&period=30&digits=6`;
  out[2]!.draft.favorite = true;
  out[0]!.draft.favorite = true;
  out[10]!.draft.reprompt = true;
  const card = (title: string, brand: string, number: string, age: number, year: number, code: string): DemoSeed => ({
    minsAgo: age * DAY_MIN, draft: { type: "card", title, favorite: false, reprompt: true, tags: ["card"], folder: "Finance", fields: { cardholder: "Hexis Alden", brand, number, expMonth: "09", expYear: String(year), code, pin: pick("0123456789", 4) }, customFields: [] },
  });
  const nextYear = new Date().getFullYear();
  out.push(
    card("Chase Sapphire", "Visa", "4242 4242 4242 4242", 560, nextYear + 3, pick("0123456789", 3)),
    card("Amex Gold", "American Express", "3782 822463 10005", 330, nextYear + 2, pick("0123456789", 4)),
    { minsAgo: 800 * DAY_MIN, draft: { type: "identity", title: "Hexis Alden", favorite: false, reprompt: false, tags: ["home"], folder: "Personal", fields: { salutation: "Mx", firstName: "Hexis", lastName: "Alden", email: gmail, phone: "+1 415 555 0134", address1: "2210 Harbor View Drive", city: "San Francisco", postalCode: "94109", country: "United States" }, customFields: [] } },
    { minsAgo: 640 * DAY_MIN, draft: { type: "secureNote", title: "Home Wi-Fi", favorite: false, reprompt: false, tags: ["home"], folder: "Personal", fields: { body: `Network: Alden-5G\nPassword: ${pick("abcdefghjkmnpqrstuvwxyz", 6)}-${pick("abcdefghjkmnpqrstuvwxyz", 6)}-${pick("23456789", 4)}\nRouter admin: 192.168.1.1` }, customFields: [] } },
    { minsAgo: 300 * DAY_MIN, draft: { type: "secureNote", title: "Recovery codes (GitHub)", favorite: false, reprompt: true, tags: ["code"], folder: "Dev", fields: { body: Array.from({ length: 6 }, () => `${pick("0123456789abcdef", 4)}-${pick("0123456789abcdef", 4)}-${pick("0123456789abcdef", 4)}`).join("\n") }, customFields: [] } },
    { minsAgo: 220 * DAY_MIN, draft: { type: "sshKey", title: "Deploy key (side project)", favorite: false, reprompt: false, tags: ["code"], folder: "Dev", fields: { publicKey: `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI${pick(ALNUM, 38)} hexis@laptop`, privateKey: `-----BEGIN OPENSSH PRIVATE KEY-----\n${pick(ALNUM, 70)}\n${pick(ALNUM, 70)}\n-----END OPENSSH PRIVATE KEY-----`, fingerprint: `SHA256:${pick(ALNUM, 43)}` }, customFields: [] } },
    { minsAgo: 150 * DAY_MIN, draft: { type: "totp", title: "Twitch", favorite: false, reprompt: false, tags: ["streaming"], folder: "Entertainment", fields: { issuer: "Twitch", account: gmail, totp: `otpauth://totp/Twitch:${gmail}?secret=${secret()}&issuer=Twitch` }, customFields: [] } },
    { minsAgo: 90 * DAY_MIN, draft: { type: "secureNote", title: "Passport and visa notes", favorite: false, reprompt: true, tags: ["documents"], folder: "Personal", fields: { body: "Passport renewal due next spring. Photos taken at the pharmacy on Polk Street." }, customFields: [] } },
    login("Notion", outlook, "https://www.notion.so/login", "Work", ["notes"], null, 260, 40),
    login("Zoom", outlook, "https://zoom.us/signin", "Work", ["meetings"], null, 500, 120),
  );
  return out;
}
