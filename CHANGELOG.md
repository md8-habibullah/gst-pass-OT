# Changelog

All notable changes are listed here, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [Semantic Versioning](https://semver.org/)
once 1.0 is reached (before that, a minor bump may change a stored format; the release notes say so).

Where the project stands, what is verified and what is not: [`ai-docs/CHANGELOG_AND_STATE.md`](ai-docs/CHANGELOG_AND_STATE.md).

## [Unreleased]

## [0.1.0] - 2026-11-24

First public build, under its final name: **Hexis**.

### Added

* Post-quantum vault: hybrid ML-KEM-1024 + X25519 key wrap, Argon2id password stretching, XChaCha20-Poly1305 items with HKDF sub-keys.
* Items: logins, cards, identities, SSH keys, secure notes, media; RFC 6238 one-time codes with QR setup on phones; immutable version history, trash with two-step permanent delete, archive, clone.
* Several accounts per device, each with its own master password.
* S3-compatible backup and sync: one primary target plus mirrors, a master backup, restore into a new vault. Event-driven (unlock, saved change, leaving the app), no timers.
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
* The package repositories exist only after a stable release is published and the signing key is set up (`scripts/github-setup.sh repo-key`); until then the installer downloads release files. There is no Linux build for aarch64.
* Android: the file pickers return `content://` addresses, so import, drive add and save-a-copy do not work there yet.
* Only Bitwarden web-vault JSON and Ente Auth plain text were checked against real exports.
