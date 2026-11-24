<p align="center"><img src="src/assets/logo.svg" width="128" height="128" alt="Hexis logo: two pillars joined by a crossbar of two layers"></p>

# Hexis

**A zero-knowledge, post-quantum password manager that syncs to your own S3 storage: one bucket, or several at once.**
Passwords, one-time codes, secure notes and an encrypted file drive live in a vault on your device. Add any
S3-compatible storage (more than one if you like) and Hexis keeps sealed copies on all of them, so a lost phone or a
lost provider loses nothing. No account, no server of ours, no telemetry of ours: every byte of your data that leaves
your device is ciphertext. (Two things to know: the optional Android QR scanner uses Google's ML Kit, and the app can
ask three large websites for the real time so a changed device clock cannot shorten a lock wait; you can switch that
off. Both are in [PRIVACY.md](PRIVACY.md).)

[Website](https://md8-habibullah.github.io/post-quantum-passman-gst/) · [CI runs](../../actions/workflows/ci.yml) · [MIT licensed](LICENSE) · [Privacy](PRIVACY.md) · [Security policy](SECURITY.md)

> **Status: pre-1.0 and not independently audited.** The design, the tests and the honest list of what is *not*
> verified are in [`ai-docs/CHANGELOG_AND_STATE.md`](ai-docs/CHANGELOG_AND_STATE.md). Keep a separate backup of anything you cannot lose.

Built with Tauri v2: a Rust core (all cryptography, storage and sync) behind a React 19 + TypeScript + Tailwind UI.
The webview never sees a key, a seed or a master-password hash.

## Why Hexis

* **Your own storage, as many as you want.** Sync goes straight from your device to S3-compatible buckets you own
  (Amazon S3, Cloudflare R2, Backblaze B2, Wasabi, a MinIO on your own machine, and others that speak the S3 protocol).
  Add several: one **master** that your other devices sync from, and any number of **mirrors** that receive every
  change. No middleman server exists. Details in [Sync and backup](#sync-and-backup-to-your-own-s3-storage).
* **Zero knowledge.** The master password and every key exist only in Rust memory while the vault is unlocked.
  Storage providers see ciphertext, object names, sizes and timestamps. Nothing else.
* **Post-quantum ready.** The vault key is wrapped with a hybrid **ML-KEM-1024 + X25519** exchange; the password goes
  through **Argon2id** (64 MiB x 4 passes by default, configurable); items use **XChaCha20-Poly1305** with per-object HKDF sub-keys.
  Details and wire formats: [`CRYPTO_SPEC.md`](ai-docs/CRYPTO_SPEC.md).
* **Offline first.** Everything works without a network and without any bucket; sync is something you add, not
  something you need. A first start opens a ready-made demo vault (its password is on the screen), so there is nothing
  to set up before you look around.
* **Quiet by design.** Sync runs on three triggers only: unlock, a saved change, and leaving the app. No timers, no
  background services, no notifications, no telemetry in Hexis's own code.
* **Honest about limits.** The threat model says what is and is not protected. The optional Stage 1 gate is a privacy
  screen, never a shortcut to the keys: the master password is always required to open a vault.

## What it does

| Area | Details |
|---|---|
| Items | Logins, cards, identities, SSH keys, secure notes, media; one-time codes (RFC 6238) with QR setup on phones; folders, tags, favourites |
| History | Immutable version history per item, trash with a two-step permanent delete, archive, clone |
| Accounts | Several independent vaults per device, each with its own master password and its own backup targets |
| Encrypted drive | Files up to 50 MB, chunked and authenticated, attachable to items; screen capture blocked while one is open on Android |
| Sync and backup | Several S3-compatible targets at once (a master plus mirrors) over HTTPS, single-writer replication, pull-and-reconcile at unlock, per-target health, restore into a new vault |
| Time | Every time is stored in UTC and shown in the time zone you pick on the first page (changeable in Settings). Lock waits are measured on a clock the device date cannot move, and, when the online check is on, on UTC read from public HTTPS servers |
| Keyboard | About 30 shortcuts (`?` lists them): search, new item, go-to chords such as `g` then `c`, copy, edit, archive, trash. Settings is a list of pages on phones and a two-column layout on tablets and desktops |
| Gate and panic | Optional Stage 1 gate (PIN, password or number grid) behind a decoy maths screen, a panic credential, 16 attempts with growing waits (1.5 minutes up to 3 days) and then a wipe, idle lock and lock after leaving the app |
| Disguise | Rename and re-icon the app (11 presets plus your own title), including the launcher icon on Android |
| Import | About 25 formats recognised by content, with a preview, duplicate handling and one all-or-nothing transaction (see below) |
| Export | Encrypted archive, or a readable Bitwarden-compatible JSON/CSV copy after a master-password re-prompt |

## Sync and backup to your own S3 storage

Hexis has no sync server. It writes sealed objects to buckets that belong to you, and it can write to several at once:

* **Bring any S3-compatible storage.** Add each target on the **Backup** page with its endpoint, bucket and access
  keys. The keys are stored sealed inside your vault, so you are never asked for them twice.
* **A master and any number of mirrors.** The master is the copy your other devices sync from and the one that settles
  disagreements. Mirrors receive every change from your device (same objects, same bytes), so losing one provider loses
  nothing. Each target shows its health (healthy, lagging or failing), its queue and its last success; **Make master**
  promotes a mirror after re-checking every object.
* **Sealed before upload.** Items are encrypted on your device with keys that never leave it. A provider sees object
  names, sizes and times, never content and never your master password.
* **Several devices, no silent overwrites.** At unlock a device pulls from the master and reconciles by version, and a
  push never overwrites remote data that device has not seen (the target shows `failing` with the reason instead).
* **Restore anywhere.** Point a fresh install at a bucket, enter the master password, and the vault is rebuilt.
* **Event-driven.** Sync runs when you unlock, save a change or leave the app: no timers, no background service, no
  notifications.

What is *not* verified yet: sync has been tested against Hexis's own local S3-compatible reference server
(`scripts/mock_s3.py`, which checks AWS Signature V4 independently of the client), not against real AWS, Cloudflare,
Backblaze, Wasabi or MinIO accounts. There is also no manual merge screen for two devices that edited the same item;
the rules and what is built are in [`STORAGE_AND_SYNC.md`](ai-docs/STORAGE_AND_SYNC.md).

## Download

**Linux, one command.** It works on any 64-bit Linux with glibc, finds out which system it is on, and installs the right
thing (Debian, Ubuntu and relatives through a signed apt repository; Fedora, RHEL and openSUSE through a signed rpm
repository; Arch, CachyOS and Manjaro through a signed pacman repository; everything else through a checksum-verified
tarball or AppImage under your home folder):

```
curl -fsS https://md8-habibullah.github.io/post-quantum-passman-gst/install.sh | sh
```

No `curl`? `wget -qO- https://md8-habibullah.github.io/post-quantum-passman-gst/install.sh | sh`. Options go after `sh -s --`
(`--nightly`, `--version X.Y.Z`, `--user`, `--apk`, `--check`, `--update`, `--uninstall`, `--dry-run`, `--detect`, `--help`). Read the script
first: it is [`scripts/install.sh`](scripts/install.sh), also shown at `/download/install-script/` on the site, and
`--dry-run` prints every command without running one. What it does on each kind of system, and how to add the
repositories by hand, is on the [Download page](https://md8-habibullah.github.io/post-quantum-passman-gst/download/).

**Update and remove.** The same script looks after an install later; your vaults are never touched:

```
curl -fsS https://md8-habibullah.github.io/post-quantum-passman-gst/install.sh | sh -s -- --check       # is there a newer version?
curl -fsS https://md8-habibullah.github.io/post-quantum-passman-gst/install.sh | sh -s -- --update      # update, the way it was installed
curl -fsS https://md8-habibullah.github.io/post-quantum-passman-gst/install.sh | sh -s -- --uninstall   # remove it
```

An install from the package repository also updates with `apt upgrade`, `dnf upgrade`, `zypper update` or `pacman -Syu`;
a tarball or AppImage install has a short `hexis-update` command next to the program.

Every push to `main` builds all variants. A push that raises the version number is also published as a release, by CI, once
every test has passed. Find the files on the [**Releases page**](../../releases) (the rolling `nightly` pre-release is the
newest `main`).

| Variant | File | Notes |
|---|---|---|
| Linux, Debian / Ubuntu | `Hexis_<version>_amd64.deb` | needs `libwebkit2gtk-4.1` (installed automatically by `apt`) |
| Linux, Fedora / openSUSE | `Hexis-<version>-1.x86_64.rpm` | needs `webkit2gtk4.1` |
| Linux, any distribution | `Hexis_<version>_amd64.AppImage` | `chmod +x` and run |
| Linux, plain archive | `hexis-<version>-linux-x86_64.tar.gz` | the binary, desktop entry and icons |
| Linux, Arch | [`packaging/arch/PKGBUILD`](packaging/arch/PKGBUILD) | builds from the release source; the installer uses the signed pacman repository instead |
| Android 9+ (arm64) | `hexis-<version>-android-arm64.apk` | sideload, or `... | sh -s -- --apk`. Official files are signed with the project's release key (fingerprint in the release notes). A `-testkey` suffix means a build made without that key, for example in a fork; it cannot update an app installed from another build |

Windows, macOS and iOS are not targets: the code has never been built or tested there.

Check a download before you run it:

```
sha256sum -c SHA256SUMS --ignore-missing
```

## Run it from source

```
npm install
npm run tauri dev          # the real app; needs the system libraries listed in ai-docs/BUILD_TARGETS.md
npm run dev                # UI only, in a browser at http://localhost:1420 with an in-memory PREVIEW backend (not secure)
npm test                   # frontend unit tests (vitest)
(cd src-tauri && cargo test)        # crypto + storage + IPC tests
python3 scripts/crypto-harness.py   # test the crypto core without any GUI libraries
scripts/verify_all.sh               # every check CI runs, in one command
scripts/verify_all.sh --e2e         # plus the end-to-end suites against the real Rust core
```

The browser preview has dev deep links for design review, for example
`http://localhost:1420/?vault=unlocked&open=1&history` (see `ai-docs/UI_DESIGN_SYSTEM.md`).
Its demo password is shown on its lock screen; the preview is excluded from production builds.

Build the bundles yourself:

```
npm run tauri build -- --bundles deb,rpm,appimage      # Linux, into src-tauri/target/release/bundle/
npm run tauri android build -- --apk --target aarch64   # Android; see ai-docs/BUILD_TARGETS.md §4 for the one-time setup
```

## Security in one page

**Protects against:** theft of a locked device, a copied data folder, a copied or hostile S3 bucket, a "harvest now,
decrypt later" quantum adversary, shoulder-surfing (secrets stay hidden until you reveal them and hide again after 20 seconds; items you flag ask for the master
password every time), and a forgotten unlocked app (idle lock, and on phones a lock after you leave the app).

**Does not protect against:** malware or a debugger on an *unlocked* device, a weak master password, a provider that
rolls objects back to older valid versions (partly mitigated), and traffic-pattern analysis.

The gate is a privacy screen: nothing wraps or caches the vault key, a copied data folder resets its attempt counter,
and an overwrite on wipe cannot be proven on SSDs. Full text: [`ARCHITECTURE.md`](ai-docs/ARCHITECTURE.md) §3 and §5.
To report a vulnerability, read [SECURITY.md](SECURITY.md). What the app does and does not collect: [PRIVACY.md](PRIVACY.md).

## Import and export

**Import** from KeePass (`.kdbx`, with password and/or key file, and KeePass XML), Bitwarden (JSON, CSV), Proton Pass (JSON, ZIP, CSV), 1Password (`.1pux`, CSV), LastPass, Dashlane (CSV, JSON), NordPass, Enpass (CSV, JSON), Chrome / Edge / Brave / Firefox / Safari password CSVs, Aegis and 2FAS (unencrypted exports), lists of `otpauth://` links, Google Authenticator export links, and any other CSV through a column mapper. The format is recognised from the file's content, not its name. Files are read in Rust, in memory, with hard size and nesting limits, and a preview shows what would happen before anything is saved; three modes handle duplicates (skip, add as a new version, import everything); the whole import is one all-or-nothing transaction and is queued for backup like any other change. Encrypted exports of other apps are refused with instructions (export unencrypted, import, delete the file).

**Export** a readable (Bitwarden-compatible JSON or CSV) copy of your vault from Tools. That file is **not encrypted**: it needs your master password again, is written with owner-only permissions, and you should delete it as soon as it has done its job. The encrypted archive export is unchanged.

Authenticator exports are read with their own settings: an Ente Auth plain-text export keeps pinned codes as favourites, its tags and notes, and skips codes that were in its trash. A one-time code key that cannot make codes is never dropped: it stays in a hidden field with a warning.

## Documentation

The whole design is written down in [`ai-docs/`](ai-docs/). Read them in this order:

| Document | What it answers |
|---|---|
| [`ARCHITECTURE.md`](ai-docs/ARCHITECTURE.md) | What it is, the threat model, trust boundaries, the lock state machine, the module map, and the decision log (D1 to D23) |
| [`CRYPTO_SPEC.md`](ai-docs/CRYPTO_SPEC.md) | Primitives, key hierarchy, envelope and keyring wire formats, test vectors |
| [`STORAGE_AND_SYNC.md`](ai-docs/STORAGE_AND_SYNC.md) | SQLite schema, version history, trash, the backup protocol, sync triggers, the drive, the importer |
| [`IPC_CONTRACT.md`](ai-docs/IPC_CONTRACT.md) | Every command the UI can call, its inputs, outputs and error codes |
| [`UI_DESIGN_SYSTEM.md`](ai-docs/UI_DESIGN_SYSTEM.md) | The "Strongroom" design language: tokens, layouts, motion, accessibility |
| [`BUILD_TARGETS.md`](ai-docs/BUILD_TARGETS.md) | System packages, Linux bundles, the Android toolchain, mock S3, what leaves the app and when |
| [`FEATURE_QUEUE.md`](ai-docs/FEATURE_QUEUE.md) | The queued feature specs (sync, gate, disguise, drive, importer) and how each was built |
| [`CHANGELOG_AND_STATE.md`](ai-docs/CHANGELOG_AND_STATE.md) | Where the project is right now, what is verified, and the honest list of what is not |

The same pages are published as a website by the [`pages` workflow](.github/workflows/pages.yml).

## Repository layout

```
src/                  React + TypeScript UI (Strongroom); talks to the core only through a typed Backend interface
src-tauri/src/
  crypto/             primitives, envelope, hybrid KEM, key bundle, TOTP; tested without GUI libraries
  storage/            SQLite vault, versions, trash, accounts, drive, importer glue, S3 backup and sync
  importer/           ~25 format readers, preview and commit
  commands/           the thin IPC layer, the Stage 1 gate wrapper, disguise, drive, dialogs
src-tauri/examples/   dev-only HTTP shim (e2e_server) used by the end-to-end tests; never part of a release
e2e/                  real-core end-to-end scripts driven from headless Chromium (see e2e/README.md)
scripts/              verify_all.sh, ci_e2e.sh, install.sh (the one-line installer), make-repos.sh and make-arch-repo.sh (apt, rpm
                      and pacman repositories),
                      Android patch and setup scripts, mock S3, site builder
assets/disguise/      the disguise presets and icons (single source of truth)
src/assets/           the Hexis logo: logo.svg (app icon), logo-mark.svg (the H), logo-mark-mono.svg; `python3 scripts/gen_icons.py` renders every platform icon from them
packaging/arch/       PKGBUILD for Arch Linux
ai-docs/              the design documents above
.github/              CI, release and Pages workflows, issue and pull-request templates
```

## Continuous integration and releases

| Workflow | Runs on | What it does |
|---|---|---|
| [`ci.yml`](.github/workflows/ci.yml) | every push, every pull request | type-check, unit tests, `rustfmt`, `clippy -D warnings`, `cargo test`, the Android patch tests, the site build with a link check, then the real-core end-to-end suites; builds the Linux bundles and the Android APK and keeps them as artifacts |
| [`ci.yml`](.github/workflows/ci.yml) (`installer` job) | every push, every pull request | runs the one-line installer in Ubuntu, Debian, Fedora, openSUSE and Arch containers against that build's packages, and checks it refuses Alpine and a too-old glibc |
| [`ci.yml`](.github/workflows/ci.yml) (publish jobs) | push to `main` | replaces the rolling `nightly` pre-release with that build, plus `SHA256SUMS` |
| [`ci.yml`](.github/workflows/ci.yml) (publish jobs) | tag `vX.Y.Z` | checks the tag matches the version in `package.json`, `tauri.conf.json` and `Cargo.toml`, then opens a **draft** release with every variant for you to review and publish |
| [`pages.yml`](.github/workflows/pages.yml) | push to `main`, a published stable release | builds the website from this README and `ai-docs/`, adds the signed apt, rpm and pacman repositories of the newest stable release, and deploys it to GitHub Pages |

Maintainers: [`scripts/github-setup.sh`](scripts/github-setup.sh) lists the one-time repository settings (Pages source,
private vulnerability reporting, topics) and the signing secrets: the Android key and the key that signs the package repositories.

## Contributing

Bug reports, fixes and docs are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first, and follow the
[Code of Conduct](CODE_OF_CONDUCT.md). Security problems go through [SECURITY.md](SECURITY.md), never a public issue.

## License

[MIT](LICENSE). The third-party crates and packages Hexis builds on keep their own licences.
