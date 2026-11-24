# Privacy

Short version: **Hexis's own code collects nothing.** There is no account, no server of ours, no analytics, no
telemetry, no crash reporting and no update check. What you store stays on your device, encrypted, unless you choose
to back it up to storage you control. **One exception on Android:** the optional QR scanner uses Google's ML Kit
library, which Google documents as able to send technical usage metrics (see "Android permissions" below).

## The app

* **Your vault** is a file on your device. Every item is encrypted on the device before it is written; the master
  password and keys exist only in memory while the vault is unlocked.
* **Your last password generator settings** (the length and which character types) are kept inside the encrypted vault, so they
  are in your backups and follow the vault to your other devices. The generated passwords themselves are never stored.
* **Not encrypted, next to the vaults:** the list of account names, the app lock's settings and hashes, the icon and name you chose
  for the disguise, and `device.json` (your time zone, whether the first-run welcome was seen, the time-check switch and the id of the
  demo vault). None of them holds an item, a key or a password. Erasing everything removes all of them except `device.json`'s zone and
  switches, so that an erased app can open without asking again.
* **Network.** Two things reach the internet. (1) **Your S3-compatible backup targets**, only when you add one, and then only
  that endpoint, over HTTPS (plain HTTP is accepted for the loopback address only, for testing). What leaves the device is
  ciphertext plus the object names, sizes and timestamps the storage provider sees by nature. Sync runs on three
  triggers only: unlock, a saved change, and leaving or backgrounding the app. There are no timers and no background services.
  (2) **The time check**, on by default and switchable in Settings, Date and time. To keep the waits after wrong passcodes
  honest when someone changes the device date, the app sends a `HEAD` request to `google.com`, `cloudflare.com` and
  `aws.amazon.com` and reads the `Date` header of the reply: at most once every six hours after a good check, and again
  after the device restarts. Nothing of yours is sent; those sites see your IP address and the request, as with any website.
  Switched off, no such request is made, and after a restart a wait is judged by the device clock.
* **Links** in your items open in your default browser only when you click them, and only `http` and `https` links.
* **Clipboard.** Copied passwords and codes are cleared after 15, 30 or 60 seconds (your choice). The timer runs inside
  the app, so it only works while the app is open; if the app is closed first, the copied value stays on the clipboard
  until something else replaces it. A system-level clipboard manager can keep its own history that Hexis cannot clear.
* **Files you import or export** are read or written only where you point the app. A readable (unencrypted) export is
  written with owner-only permissions after you re-enter the master password; delete it when you are done.
* **No tracking identifiers.** The app generates none and sends none.

### Android permissions and the QR scanner

The Android app asks for only what its features need. It declares no notification permission, no foreground
service and no vibration permission (a build check and the manifest patch enforce this).

* `INTERNET` and `ACCESS_NETWORK_STATE`: to reach the backup endpoint you configure and, unless you switch it off, the time check.
* `CAMERA` (asked at the moment you scan): to read an `otpauth://` QR code. Scanning is optional; you can type the setup key. The camera is not required to install the app.
* `USE_BIOMETRIC` and `USE_FINGERPRINT`: the optional fingerprint for the Stage 1 gate. It never opens a vault; the master password always does.
* The screen is protected from screenshots and the recent-apps preview while a decrypted file is open.

**Google ML Kit.** The QR scanner is the Tauri barcode-scanner plugin, which relies on Google's ML Kit barcode library
through Google Play services. Google documents that ML Kit may send technical usage and performance metrics to Google,
and the APK contains Google's data-transport components for that. Hexis's own code never receives or sends that
traffic, and it has **not been measured** what the library sends or when. If that matters to you, do not use the
scanner (type the setup key instead), or use the Linux builds, which do not include it. Replacing the scanner with
one that never leaves the device is an open task (see `ai-docs/CHANGELOG_AND_STATE.md`).

The exact list in a given build is the merged manifest of that build; the release notes of each version state it.

## The one-line installer

`install.sh` (the command on the Download page) is a script you run once, not part of the app. It sends ordinary HTTPS
requests, with nothing from your machine in them beyond what any download reveals (your address, the file asked for):
to the project site for the script, the signing key and the package-repository files, and to `github.com` for release
files and their `SHA256SUMS` when it downloads directly. It reads `/etc/os-release` and checks which tools exist on your
machine to choose a package, and it does not send that anywhere. It sets nothing up that phones home. Read it before
you run it: it is one plain shell file, shown on the site at `/download/install-script/`.

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
