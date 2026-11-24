# Changelog

All notable changes are listed here, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [Semantic Versioning](https://semver.org/)
once 1.0 is reached (before that, a minor bump may change a stored format; the release notes say so).

Where the project stands, what is verified and what is not: [`ai-docs/CHANGELOG_AND_STATE.md`](ai-docs/CHANGELOG_AND_STATE.md).

## [Unreleased]

## [0.3.0] - 2026-11-25

Light and dark themes, the next one-time code, a clean uninstall that leaves nothing behind, and a password generator that remembers its settings.

**Upgrading from 0.2.0:** `device.json` (next to the vaults) gains `theme` and `clipboardClear`, which older builds ignore. How long a copied secret stays on the clipboard starts at 30 seconds once, because it used to live in the window's own storage; set it again in Settings, Security. The cache files an older version let the window write into the data folder are removed when this version first starts. Vault files and S3 backups are unchanged. On Android, automatic backup is switched off for Hexis.

### Added

* **The next one-time code.** Under the live code of a TOTP field, and on each tile of the Codes page, Hexis now shows the code that comes next (with a copy button), and on the item itself the one that just ended. Handy when a code is about to run out.
* **Light and dark themes.** Settings > Theme: *Device setting* (the default: light by day and dark by night, or whatever the device uses), *Light* or *Dark*. Both are complete palettes with every text colour checked for contrast, including the lock screens, the sheets and the logo. The choice applies at once, is kept on the device (not in the window), and is there before the first screen on the next start. On a computer the title bar follows; on Android the status and navigation bar icons follow.
* **Remove Hexis from this device.** Settings > Security. Three confirmations and a sum, then it destroys every vault and the app lock (files are overwritten with zeros before they are deleted), deletes the settings and the window's leftovers, makes nothing new and closes the app. Then you uninstall the program. The last screen says what is gone and what is not (backups in S3 buckets are never touched).
* **The password generator remembers your last settings.** Choose 30 characters (or switch symbols off) once; the Generator page and the item editor both open with that next time. The settings are kept inside your encrypted vault, copied to every backup target and picked up by your other devices on their next sync. The newest change wins. The generated passwords are still never stored.

### Changed

* **A clean uninstall.** The installer's `--uninstall` now removes Hexis *and* everything it keeps for your account, after printing every folder and asking you to type `REMOVE` (`--yes` confirms in advance; `--keep-data` removes only the program; it refuses while Hexis is running). Removing the program with a package manager alone still leaves your data, as every distribution requires; the README and the Download page say so.
* **The window keeps no cache, cookies or stored page data.** It runs without a persistent profile, and the two settings it used to keep (how long a copied secret stays on the clipboard, and the theme) are now in the app's own `device.json`. Files an older version let the window write into the data folder are removed when this version starts and when you erase. **Upgrading:** the clipboard time starts at 30 seconds once, because the old value lived in the window's storage.
* **Android does not back up Hexis** (`allowBackup` off, no cloud backup, no device transfer), so a vault cannot be restored behind your back after a reinstall.
* Erasing everything also removes the window's leftovers, and the erase and the removal share the same wipe.

### Fixed

* **The Android launcher showed the app name inside quotation marks** (`"Hexis"`). Tauri writes its app name with quoting marks that Android removes when it reads the file; the build that makes the icon-and-name choices copied them as text. The name is now copied the way Android reads it. Also fixed in the shell tests: an uninstall check that failed on Debian and Ubuntu, and a test that could fail by chance.

## [0.2.0] - 2026-11-24

Trusted time, a time zone chosen on the first page, a built-in demo vault, keyboard shortcuts, settings in pages and a calmer look, on top of the phone-QA fixes below.

**Upgrading from 0.1.0:** two stored files change shape. `gate.json` gains the boot-clock fields of a wait (an older build reads that record as damaged), and an item with "ask for the master password again" set reads as damaged in an older build. Vault files and S3 backups are otherwise unchanged. Hexis also writes a small `device.json` (your time zone, two switches) next to the vaults.

### Added

* **A time zone is the first page.** Choose it once (the device's zone is suggested); change it later in Settings, Date and time. Everything Hexis stores is UTC, and every time on screen is written in your zone.
* **Waits that a changed device date cannot shorten.** The waits after wrong passcodes are measured on the device's boot clock and, when the online check is on, on UTC read from `google.com`, `cloudflare.com` and `aws.amazon.com` (a `HEAD` request and the `Date` header, nothing of yours). While a wait cannot be judged (after a restart with no network) the lock refuses every attempt and says so. The check can be switched off in Settings; then no request is made and a restart is judged by the device clock.
* **A demo vault on first start.** With no vault on the device, Hexis opens "Hexis123" (password "Hexis123"), full of believable items for one invented person. An erase and a forgotten-password reset land in a fresh one, so nothing on screen looks unusual. Your own vault is one tap away ("Create my own vault"), and the item editor says so when you save into the demo, because its password is public.
* **Keyboard shortcuts** (about 30, `?` lists them) and a Help and shortcuts page.
* **Settings in pages:** Security, App lock, Accounts, Date and time, App icon and name, Backup and files, Help and shortcuts, About. A list on phones, two columns on tablets and desktops.
* **"Ask for the master password again"** on any item (a switch in the editor, a key mark in lists and in the item). An open vault now shows, copies, edits and clones ordinary items directly, as Bitwarden does; a flagged item asks every time, and its live one-time code waits behind a "Show code" button. Importers and exporters carry the flag.
* **Waits after wrong passcodes** on the app lock: 3 wrong attempts lock the entry for 1.5 minutes, 6 for 5 minutes, 9 for 45 minutes, 12 for 8 hours and 15 for 3 days, with a countdown on screen. The wait survives closing the app. Nothing typed is checked while it runs.
* **A last-attempt warning.** After the 15th wrong attempt a red panel says that a wrong passcode erases everything on the device, and sending it asks for a confirmation that shows how many digits were typed.
* **Settings, App lock:** "Lock after you leave the app" on phones (right away, 15 or 30 seconds, 1, 5, 15 or 30 minutes, 1 or 4 hours; default 5 minutes), and 1 and 4 hours in the inactivity choices.
* A long sum before "Erase everything" in the manual reset. Only the right answer erases; a wrong one erases nothing and brings a new sum.
* `e2e/real-multi-s3.mjs`: one primary and two secondary buckets on two real S3-compatible servers in Docker, including a restore from the primary alone.

### Changed

* A calmer look: no glass blur, lattice, glow or pulse; plain labels; every transition 80 to 180 ms. The number grid on the app lock no longer fades or dims while a code is checked.
* The first-run choices (create, open a file, restore) moved from the lock screen into a one-time sheet over the demo vault.
* Erasing everything keeps your time zone and the welcome flag, so the app can open into a fresh demo without asking again.
* The app now makes three small requests to public websites for the time (switchable). See PRIVACY.md.
* The attempt limit before the app lock wipes the device is now 16 (it was 15).
* Leaving the app on a phone no longer locks it at once; it locks after the time chosen in Settings (default 5 minutes). The timer runs in the core, so it holds even if the system freezes the page.
* Permanent delete, changing the master password, exports, saving or deleting drive files, backup secrets and changing the app lock still always ask for the master password.
* Android 13 and later: the app's picture in the recent-apps list is blank, so secrets on screen are not kept there.
* A build older than this one reads an item that has the new flag set as damaged. Items without the flag are unaffected.
* The repository moved to `post-quantum-passman-gst`. The website, the one-line installer and the package repositories now live at <https://md8-habibullah.github.io/post-quantum-passman-gst/>; GitHub does not redirect the old Pages address (`.../gst-pass-OT/`), so a machine that registered the package repository from there must run the installer once more. Links to `github.com/md8-habibullah/gst-pass-OT` still redirect. The product name is unchanged: Hexis.
* The README, website, package descriptions, release notes and social preview now lead with multi-S3 sync (one bucket or several at once) instead of describing the app only as local-first.
* `scripts/github-setup.sh android-key` now does the whole job: it creates the Android release key outside the repository, stores it in GitHub as encrypted secrets and publishes the certificate fingerprint as a variable. CI signs release APKs with it (no `-testkey` in the name), never gives it to pull-request builds, and a version tag build fails instead of shipping a throw-away-key APK. The fingerprint is printed in release notes and on the Download page.
* `scripts/github-setup.sh apply` replaces the repository topics, and also enables secret scanning, push protection and Dependabot security updates; new `release-notes vX.Y.Z` rewrites a release description from this file.

### Fixed

* Android: the camera scan closed the app. R8 had removed the constructors of ML Kit's component registrars; keep rules now protect them (`scripts/android/hexis.pro`). What is checked, and what still needs a phone, is listed in `ai-docs/CHANGELOG_AND_STATE.md`.
* Android: the Back button now sends the app to the background instead of ending the process while the runtime shuts down.
* Android: the keyboard covered the "Confirm it is you" and settings sheets, and the status bar overlapped sheet text. The window now follows the keyboard and the system bars.

## [0.1.0] - 2026-11-24

First public build, under its final name: **Hexis**.

### Added

* Multi-S3 sync and backup: any number of S3-compatible targets at once, one master that other devices sync from plus mirrors that receive every change, a health indicator per target, promotion of a mirror to master, and restore into a new vault. Event-driven (unlock, saved change, leaving the app), no timers.
* Post-quantum vault: hybrid ML-KEM-1024 + X25519 key wrap, Argon2id password stretching, XChaCha20-Poly1305 items with HKDF sub-keys.
* Items: logins, cards, identities, SSH keys, secure notes, media; RFC 6238 one-time codes with QR setup on phones; immutable version history, trash with two-step permanent delete, archive, clone.
* Several accounts per device, each with its own master password.
* Optional Stage 1 gate (PIN, password or number grid) behind a decoy maths screen, panic credential, attempt limit with wipe, idle lock in the core.
* App disguise: 11 name-and-icon presets plus a custom title, with launcher aliases on Android.
* Encrypted 50 MB file drive (chunked, authenticated) with attachments.
* Importer for about 25 formats recognised by content, with preview, duplicate modes and one all-or-nothing transaction; readable Bitwarden-compatible export behind a master-password re-prompt.
* Linux packages (deb, rpm, AppImage, tar.gz, Arch PKGBUILD) and an Android arm64 APK.
* One-line Linux installer, `curl -fsS <site>/install.sh | sh`: detects the distribution, package manager, CPU, libc and root helper; registers a signed apt, rpm or pacman repository (Debian family, Fedora and RHEL family, openSUSE, Arch family) or downloads and verifies the tarball or AppImage (everything else); `--nightly`, `--version`, `--user`, `--apk`, `--dry-run`, `--detect`.
* `--check` says whether a newer version exists; `--update` brings an install up to date the way it was made (package manager for repository installs, the same folder for tarball and AppImage installs, stale files removed); `--uninstall` removes it and keeps your vaults. Installs without a package manager get a `hexis-update` command next to the program. The signed repositories are built by CI from the newest stable release and deployed with the site.
* The Hexis logo and icon set: a squircle app icon, a transparent mark for the app, and every platform size (Linux, Windows, macOS, iOS, Android adaptive and themed icons) from one vector source (`src/assets/`, `scripts/gen_icons.py`). On unlock the mark closes its two layers; with a disguise active the disguise's name and icon replace the mark everywhere, and turning it off brings Hexis back.
* Release descriptions written from this changelog, with install commands, a downloads table and checksums (`scripts/release_notes.py`).
* GitHub Actions: checks and end-to-end suites on every push and pull request, bundles built and published to a rolling `nightly` pre-release, draft releases from `vX.Y.Z` tags, and a documentation website on GitHub Pages.

### Changed

* Renamed from the working title to **Hexis** everywhere: program, packages (`hexis`), bundle identifier `dev.habibullah.hexis`, installer, repositories, site, documentation and export file names. Builds made before the rename keep working: the old desktop data folder (`~/.local/share/app.passman.vault`) is adopted on first start, the open dialog still lists `.passman` files, and old Bitwarden-style exports still bring their tags back. The stored format is unchanged: its internal identifiers (the `passman/v1/...` key-derivation labels and the `PMV1` file id) keep their historical spelling on purpose, because changing them would make existing vaults unreadable (`ai-docs/CRYPTO_SPEC.md`).
* The core's messages no longer name the product ("this app" instead), so a disguised app never announces itself.

### Known limits

* Not independently audited. The Stage 1 gate is a privacy screen, not encryption.
* The signed package repositories (apt, rpm, pacman) are built from the newest stable release; before one exists the installer downloads release files. There is no Linux build for aarch64.
* Sync was tested against Hexis's own local S3-compatible reference server, not yet against real Amazon S3, Cloudflare R2, Backblaze B2, Wasabi or MinIO accounts. There is no manual merge screen for two devices that edited the same item.
* Android: the file pickers return `content://` addresses, so import, drive add and save-a-copy do not work there yet.
* Only Bitwarden web-vault JSON and Ente Auth plain text were checked against real exports.
