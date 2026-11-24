# Privacy

Short version: **Passman's own code collects nothing.** There is no account, no server of ours, no analytics, no
telemetry, no crash reporting and no update check. What you store stays on your device, encrypted, unless you choose
to back it up to storage you control. **One exception on Android:** the optional QR scanner uses Google's ML Kit
library, which Google documents as able to send technical usage metrics (see "Android permissions" below).

## The app

* **Your vault** is a file on your device. Every item is encrypted on the device before it is written; the master
  password and keys exist only in memory while the vault is unlocked.
* **Network.** The app contacts the internet only when you add an S3-compatible backup target, and then only that
  endpoint, over HTTPS (plain HTTP is accepted for the loopback address only, for testing). What leaves the device is
  ciphertext plus the object names, sizes and timestamps the storage provider sees by nature. Sync runs on three
  triggers only: unlock, a saved change, and leaving or backgrounding the app. There are no timers and no background services.
* **Links** in your items open in your default browser only when you click them, and only `http` and `https` links.
* **Clipboard.** Copied passwords and codes are cleared after 15, 30 or 60 seconds (your choice). The timer runs inside
  the app, so it only works while the app is open; if the app is closed first, the copied value stays on the clipboard
  until something else replaces it. A system-level clipboard manager can keep its own history that Passman cannot clear.
* **Files you import or export** are read or written only where you point the app. A readable (unencrypted) export is
  written with owner-only permissions after you re-enter the master password; delete it when you are done.
* **No tracking identifiers.** The app generates none and sends none.

### Android permissions and the QR scanner

The Android app asks for only what its features need. It declares no notification permission, no foreground
service and no vibration permission (a build check and the manifest patch enforce this).

* `INTERNET` and `ACCESS_NETWORK_STATE`: to reach the backup endpoint you configure.
* `CAMERA` (asked at the moment you scan): to read an `otpauth://` QR code. Scanning is optional; you can type the setup key. The camera is not required to install the app.
* `USE_BIOMETRIC` and `USE_FINGERPRINT`: the optional fingerprint for the Stage 1 gate. It never opens a vault; the master password always does.
* The screen is protected from screenshots and the recent-apps preview while a decrypted file is open.

**Google ML Kit.** The QR scanner is the Tauri barcode-scanner plugin, which relies on Google's ML Kit barcode library
through Google Play services. Google documents that ML Kit may send technical usage and performance metrics to Google,
and the APK contains Google's data-transport components for that. Passman's own code never receives or sends that
traffic, and it has **not been measured** what the library sends or when. If that matters to you, do not use the
scanner (type the setup key instead), or use the Linux builds, which do not include it. Replacing the scanner with
one that never leaves the device is an open task (see `ai-docs/CHANGELOG_AND_STATE.md`).

The exact list in a given build is the merged manifest of that build; the release notes of each version state it.

## The project website and repository

The project site is static pages hosted by GitHub Pages. It sets no cookies, loads no third-party scripts, fonts or
images, and has no analytics. GitHub, as the host, keeps its own server logs under
[GitHub's privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).

## If you report a bug

Issues are public. Never paste real passwords, vault files, one-time-code seeds or exports into an issue; use a
throw-away vault with made-up data.

## Changes

If this ever changes (for example an opt-in feature that contacts a server), this file changes first and the release
notes say so.
