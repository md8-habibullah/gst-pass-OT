# Passman

Local-first, zero-knowledge, post-quantum password / TOTP / secure-note / encrypted-media manager.
Tauri v2 (Rust core, React + TypeScript + Tailwind UI). Targets Linux (deb, rpm, AppImage, Arch) and Android 9+.

**Start with [`ai-docs/`](ai-docs/):** `ARCHITECTURE.md` → `CRYPTO_SPEC.md` → `STORAGE_AND_SYNC.md` → `BUILD_TARGETS.md` → `CHANGELOG_AND_STATE.md` (current progress).

```
npm install
npm run tauri dev          # needs the system libraries listed in ai-docs/BUILD_TARGETS.md
python3 scripts/crypto-harness.py   # test the crypto core without any GUI libraries
```
