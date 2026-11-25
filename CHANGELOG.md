# Changelog

All notable changes are listed here, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [Semantic Versioning](https://semver.org/)
once 1.0 is reached (before that, a minor bump may change a stored format; the release notes say so).

Where the project stands, what is verified and what is not: [`ai-docs/CHANGELOG_AND_STATE.md`](ai-docs/CHANGELOG_AND_STATE.md).

## [Unreleased]

## [0.5.0] - 2026-11-25

Passkeys on Linux and on Android, and one scanner for every kind of code.

**Upgrading from 0.4.0:** vault files and S3 backups keep their format and nothing is migrated, but **update every device before you add a passkey**: Hexis 0.4.0 and older cannot save an edit to a login that holds one, and skip it when they merge a pull (read from the 0.4.0 code, not tried with a built 0.4.0). `device.json` gains `browserBridge` and `passkeyAppCheck` (older builds ignore them). Both passkey features are off until you turn them on, so nothing changes if you do not: Settings > Browsers on a Linux computer, Settings > Passkeys on Android 14 or later. Turning Browsers on writes an `extension` folder and a small launcher into the data folder and, for each browser you connect, one native messaging manifest in that browser's own folder; Disconnect, and "Remove Hexis from this device", take them away again. The Firefox extension is not signed by Mozilla yet, so a release Firefox can use it only as a temporary add-on (Chromium, Chrome, Brave, Edge and Vivaldi load it unpacked).

### Added

* **Passkeys in Firefox and Chromium-family browsers on Linux** (Settings > Browsers, off until you turn it on). A browser extension hands a website's passkey request to the running Hexis, which opens it in its own window: you see the website, choose the passkey (or the login to save a new one on) and click. A website that requires verification gets your master password, typed in the Hexis window. Nothing is signed or saved until you choose; a name that is not the page's own is refused; "Use another way" and "Cancel" are different answers; with the switch off nothing listens and the browser uses its own passkey screen. The extension is carried inside Hexis (Settings names its folders): Chromium-family browsers load it unpacked; a release Firefox needs it signed by Mozilla, which has not been done, so until then it can be loaded as a temporary add-on. ES256 only; the top frame of a page only; Snap and Flatpak browsers cannot use it. A read-only security review of the bridge was done and everything it found was fixed with tests (the inactivity lock still runs while Browsers is on, a typed master password is checked before a save counts as verified, the host re-checks its socket, a request id is used once, a hidden tab cannot raise the window). Tried end to end with a real Chromium and a real Firefox on this computer; **not tried through Mozilla's signing, a store, or a Wayland compositor's window focus**.
* **Passkeys.** A login can hold passkeys (`passkey`, `passkey#2` …) next to its sign-ins and one-time codes. The private key stays inside the core: the screen shows the website and the user name, never the key. Nothing changes in the vault file format. Update every device before adding passkeys: Hexis 0.4.0 and older cannot save an edit to a login that holds one and skip it when they merge a pull.
* **Passkey provider for Android 14 and later** (Settings > Passkeys, off until you turn it on): browsers and apps can sign in with, and save passkeys to, your vault through Android's own passkey manager. Each use asks for the fingerprint or screen lock; a locked vault opens with the master password only; nothing works while the app lock is closed. ES256 only. An app is accepted only when its website lists it (one request to that website, switchable). **Not yet tried on a real phone.**
* **A camera button and one scanner for everything.** It tells a one-time code, a browser's passkey code, an authenticator-app export and a link apart, suggests the logins with the same account, website or name, always lists the others and always offers "Create a new login".
* **Bitwarden passkeys.** Import reads `fido2Credentials` (ECDSA P-256; other kinds are skipped with a warning) and the plain-text Bitwarden JSON export writes them, with a warning that the file holds private keys. CSV leaves them out.

## [0.4.0] - 2026-11-25

Folders, a search that looks inside everything, order and filters, several websites per login, automatic merging between devices, and a safer way to delete for good.

**Upgrading from 0.3.0:** vault files and S3 backups keep their format, and a 0.3.0 build can still open everything this version writes. `device.json` gains `openInSearch` and `listSort` (older builds ignore them), the synced settings object gains `folders` (older builds keep what they do not know), and a login with several websites stores them one per line in its existing website field (an older build shows them as one long line). A change made to the same item on two devices is now merged for you; before, the other device's copy waited in the history. Permanently deleting no longer asks you to type the item's name: see Changed.

### Added

* **Folders.** Make folders (and folders inside folders, up to six levels), rename and remove them, and put an item in one with *Move to folder* in its menu or the Folder box of the editor. The list shows the folders first, with how many items each holds, then the items that sit at that level; a path above the list takes you back up. Removing a folder only removes the folder: what was inside moves up one level and nothing is deleted. Imported folders (Bitwarden writes `Work/Clients`) keep their structure.
* **A search that looks inside everything.** Words are matched one by one, in any order and in any field: titles, websites, usernames, notes, the names and descriptions of your own fields, tags and folders. A result says where it found the words ("Found in Notes"). Tick *Include earlier versions* to look in what an item used to say as well. Passwords, card numbers, security codes, one-time-code secrets and private keys are never searched, and the notes of an item that asks for the master password stay sealed.
* **Order and filters.** One panel under the search box. Nine orders: recently changed, longest ago, newest, oldest, title A to Z and Z to A, by type, most versions and fewest versions. Filters: type, favorites, has a one-time code, asks for the master password, a tag, at least and at most a number of versions, and changed within a day, week, month or year. A filter or a search looks through every folder.
* **Open in search.** Settings > Vault list: after the vault opens, the cursor is already in the search box (on a phone the keyboard comes up with it). The default order of the list is there too.
* **Several websites per login.** A box for each address and a button for another; the item shows every address with its own open and copy buttons, and the item menu opens any of them. Bitwarden import and export keep every address.
* **Works offline, merges when you are back.** Nothing needs a connection: items are made, edited, searched and deleted locally and the changes wait in the queue. When the device comes back online it fetches what your other devices changed and sends what waited. If you changed the same item on two devices, the two are merged field by field, like a notes app does: a field only one side changed takes that side; tags, websites and custom fields are united; notes are merged line by line, and when both sides changed the very same lines both versions are written one after the other, marked, so nothing is dropped. When both sides changed the same single field, the newer change wins and the other value stays in the history. Both earlier states are kept in the history, which labels the merge.
* **Import: merge matching accounts.** The default way to import. The same account arriving from two apps (a Bitwarden login and its Ente Auth code, say) becomes one item: it matches by one-time-code secret or by the same account and an alike name, and **every note is kept** (Ente's notes included), along with every website, tag and custom field. Nothing is overwritten, and importing the same file twice changes nothing the second time.

### Changed

* **The copy notice is a small notice.** It no longer counts down on top of your buttons: it shows briefly and goes after 5 seconds, or when you swipe it left or right. The clipboard still clears after the time you chose.
* **The New item menu** lists Login, Code only, Secure note, Identity, Card and SSH key first.
* **Deleting for good.** An item that has been in the trash for less than six months needs your master password **and** the answer to a long sum; an older one needs only the master password. Hexis checks the age on its own checked clock, and a wrong answer uses the sum up and shows a new one. Typing the item's name is gone.
* **The password generator under the password box** can be hidden and shown again with a button.
* **An item changed on two devices** is merged automatically instead of waiting in the history for you to restore it by hand (see Added). A merge is only refused, and the old way kept, for items of different types.

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
