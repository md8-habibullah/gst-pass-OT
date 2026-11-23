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
