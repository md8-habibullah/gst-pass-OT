# Passman

Local-first, zero-knowledge, post-quantum password / TOTP / secure-note / encrypted-media manager.
Tauri v2 (Rust core, React + TypeScript + Tailwind UI). Targets Linux (deb, rpm, AppImage, Arch) and Android 9+.

**Start with [`ai-docs/`](ai-docs/):** `ARCHITECTURE.md` → `CRYPTO_SPEC.md` → `STORAGE_AND_SYNC.md` → `BUILD_TARGETS.md` → `CHANGELOG_AND_STATE.md` (current progress).

```
npm install
npm run tauri dev          # the real app; needs the system libraries listed in ai-docs/BUILD_TARGETS.md
npm run dev                # UI only, in a browser at http://localhost:1420 with an in-memory PREVIEW backend (not secure)
npm test                   # frontend unit tests (vitest)
(cd src-tauri && cargo test)        # crypto + storage + IPC tests
python3 scripts/crypto-harness.py   # test the crypto core without any GUI libraries
```

The browser preview has dev deep links for design review, for example
`http://localhost:1420/?vault=unlocked&open=1&history` (see `ai-docs/UI_DESIGN_SYSTEM.md`).
Its demo password is shown on its lock screen; the preview is excluded from production builds.

## Import and export

**Import** from KeePass (`.kdbx`, with password and/or key file, and KeePass XML), Bitwarden (JSON, CSV), Proton Pass (JSON, ZIP, CSV), 1Password (`.1pux`, CSV), LastPass, Dashlane (CSV, JSON), NordPass, Enpass (CSV, JSON), Chrome / Edge / Brave / Firefox / Safari password CSVs, Aegis and 2FAS (unencrypted exports), lists of `otpauth://` links, Google Authenticator export links, and any other CSV through a column mapper. The format is recognised from the file's content, not its name. Files are read in Rust, in memory, with hard size and nesting limits, and a preview shows what would happen before anything is saved; three modes handle duplicates (skip, add as a new version, import everything); the whole import is one all-or-nothing transaction and is queued for backup like any other change. Encrypted exports of other apps are refused with instructions (export unencrypted, import, delete the file).

**Export** a readable (Bitwarden-compatible JSON or CSV) copy of your vault from Tools. That file is **not encrypted**: it needs your master password again, is written with owner-only permissions, and you should delete it as soon as it has done its job. The encrypted archive export is unchanged.

Authenticator exports are read with their own settings: an Ente Auth plain-text export keeps pinned codes as favourites, its tags and notes, and skips codes that were in its trash. A one-time code key that cannot make codes is never dropped: it stays in a hidden field with a warning.
