import { CreditCard, Fingerprint, Image, KeyRound, StickyNote, Terminal, Timer, type LucideIcon } from "lucide-react";
import type { ItemType } from "../../backend";

export interface TypeMeta {
  icon: LucideIcon;
  hint: string;
}

/** Icon and one-line description per item type (type picker, list badges). */
export const TYPE_META: Record<ItemType, TypeMeta> = {
  login: { icon: KeyRound, hint: "A username and password, with an optional one-time code." },
  card: { icon: CreditCard, hint: "A credit or debit card: number, expiry, security code." },
  identity: { icon: Fingerprint, hint: "Your name, address and ID numbers, ready to copy." },
  sshKey: { icon: Terminal, hint: "An SSH key pair with its passphrase and fingerprint." },
  secureNote: { icon: StickyNote, hint: "Private text: recovery codes, Wi-Fi details, anything you would not email." },
  totp: { icon: Timer, hint: "Just a one-time code, for services where you keep the password elsewhere." },
  media: { icon: Image, hint: "A description for a document or image. File attachments arrive with media support." },
};

/** The order types appear in the picker. */
export const TYPE_ORDER: ItemType[] = ["login", "card", "identity", "sshKey", "secureNote", "totp", "media"];
