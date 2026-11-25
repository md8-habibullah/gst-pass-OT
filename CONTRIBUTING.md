# Contributing to Hexis

Thanks for helping. Hexis holds people's secrets, so the bar for changes is "correct and verified" before
"fast". This page tells you how to set up, what every change must do, and what a good pull request looks like.

By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). **Security problems are not issues:** follow
[SECURITY.md](SECURITY.md).

## Ways to help

* **Report a bug** with the [bug form](../../issues/new/choose). Use a throw-away vault and made-up data. Never paste a real
  password, vault file, one-time-code seed or export.
* **Test an import** with a real export from another app and tell us what was wrong. Only two vendor exports have
  been checked against real files so far; the rest are read from public format descriptions (see
  [`CHANGELOG_AND_STATE.md`](ai-docs/CHANGELOG_AND_STATE.md)). Describe the *shape* of what went wrong, not your data.
* **Run the Android build on a real device** and report what you see. Android is the least-tested part.
* **Review the cryptography and the storage code.** No third party has. Findings go through [SECURITY.md](SECURITY.md).
* **Improve the docs.** `ai-docs/` is the source of truth for the design; fixes there are as valuable as code.

## Set up

You need Node.js 24 or newer, Rust 1.90 or newer (`rustup default stable`), Python 3, and on Linux the system
libraries from [`ai-docs/BUILD_TARGETS.md`](ai-docs/BUILD_TARGETS.md) §2 (for Debian and Ubuntu:
`libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`).

```
git clone <your fork>
cd hexis
npm install
npm run tauri dev        # the real app
npm run dev              # UI only in a browser, with an in-memory preview backend (not secure)
```

The crypto module compiles without the GUI libraries: `python3 scripts/crypto-harness.py` runs its tests in a bare
crate, which is handy on a small machine. Rust builds are heavy; on 8 GB of RAM use `CARGO_BUILD_JOBS=2` and run one
build at a time.

## Before you open a pull request

Run the same command CI runs:

```
scripts/verify_all.sh          # types, unit tests, production build, rustfmt, clippy -D warnings, cargo test, script tests
scripts/verify_all.sh --e2e    # also the real-core end-to-end suites (needs Chromium; see e2e/README.md)
```

It must end with `ALL CHECKS PASSED`. If you touch the installer (`scripts/install.sh`) or the repository builder
(`scripts/make-repos.sh`), `shellcheck` runs on them there, and `scripts/tests/test_install_sh.py` tests the installer
with stub tools and a local server. To try it for real in containers (needs docker; downloads images):
`scripts/test_installer_containers.sh <folder with the built bundles> ubuntu:22.04 fedora:latest archlinux:latest`.
The installer must stay plain POSIX `sh` (it runs under dash and busybox). CI repeats it on Linux for every push and pull request, and also builds the
Linux bundles and the Android APK, so a change that only breaks packaging is caught too.

### Before you push: what CI checks that your machine may not

A green `scripts/verify_all.sh` on a developer machine has three times not been a green CI run (2026-11-25, three red pushes). The causes were
all avoidable, so each is now a rule:

1. **Use CI's tool versions, not the newest local ones.** CI runs on `ubuntu-22.04`. Its `shellcheck` is 0.8.0 and flags `A && B || C` (SC2015);
   a local 0.11 does not. `scripts/verify_all.sh` now runs `shellcheck` a second time inside an `ubuntu:22.04` container (needs Docker). In shell
   code never write `A && B || C`; write `if A; then B; else C; fi`, or `A || C` when B is not needed.
2. **A shell test must be run under the shells it will meet.** Debian and Ubuntu run `sh` as **dash**; Fedora, Arch and openSUSE as bash. Dash
   remembers where `command -v prog` found a program and keeps answering after the program was removed; call `hash -r` first, or look inside a
   subshell. When you change `scripts/install.sh` or `scripts/tests/container_case.sh`, run
   `scripts/check_installer_in_containers.sh` (Debian 12, Ubuntu 22.04 and Fedora against the packages of the newest green CI run) **before you
   push**. If Docker is up and the run is a few minutes, "not run" is not an acceptable answer.
3. **Never assert that a short string is absent from generated text.** A test that does `!text.contains("737")` on a result that holds a random
   id, a UUID, a millisecond timestamp, a hash or base64 fails by chance, about once in fifty runs (`clone_is_password_gated_and_never_returns_secrets`
   did, on CI). Assert the structure instead (exact keys, exact values), and use `contains` only for strings long enough that a chance match is
   impossible (a full card number, a marker such as `SECRET-BANK-REF`). The same goes for searching sealed files (random bytes) for a secret:
   `nothing_that_could_unlock_a_vault_is_ever_written_to_disk` searched about a megabyte of them for the PIN `2468` and failed once in a local
   `verify_all` run. Search for a long, distinctive value instead (`FINDABLE_PIN`, thirteen digits); the `contains` helper in
   `src-tauri/src/gate/app_tests.rs` now refuses a needle under 8 bytes, so a short one fails at once and on every run, not now and then.
4. **A test fixture is a copy of what the real tool writes.** The Android label test used `<string name="app_name">Hexis</string>`, but Tauri
   writes `"Hexis"` (with quoting marks that aapt removes), so the build copied the marks as text and the launcher showed `"Hexis"` with quotes
   until a phone showed it. Take fixtures from a real generated project (`src-tauri/gen/android/...`), not from what you expect it to look like.
5. **A test never reads the real clock when it asserts an exact time.** `mockGate.test.ts` compared `retryAfterMs` with `90_000` while the mock
   gate used `Date.now()`; on a slower CI runner a millisecond ticked over between two calls and it got `89999` (2026-11-25, the push of 0.4.0 was
   red because of it, though 203 tests had passed locally). Pin the clock (`g.now = () => t`, `vi.useFakeTimers()`, a `FixedClock` in Rust), and
   a test that cannot pin it asserts a range, not a number. Run a time-based test in a loop (`for i in 1 2 3 4 5 6 7 8; do ...; done`) before you trust it.
6. **Say exactly what was run.** "Verified" means it ran with CI's versions and shells. A tool that was newer than CI's, or a container that was not
   started, is reported as "not run", and a cheap check is run instead of being listed as a gap. After a push, read the CI result (`gh run view`)
   and fix a red job before moving on.

### Rules every change follows

1. **Tests with the change.** A bug fix gets a test that fails without it; a feature gets tests for its failure
   paths as well as the happy path. Tests use **made-up data only** (`example.com` addresses, invented names and
   keys). Never commit a real export, vault, key or token.
2. **Secrets stay in Rust.** No command returns a key, a seed, a password or a hash of one. One-time codes reach the
   UI as a code and the seconds left, never the secret. Commands that reveal or change secrets take the master
   password again (the re-prompt guard) and the core re-verifies it; nothing is cached. See
   [`IPC_CONTRACT.md`](ai-docs/IPC_CONTRACT.md) and [`ARCHITECTURE.md`](ai-docs/ARCHITECTURE.md) §4.
3. **Errors are fixed sentences.** An error shown to the user, logged, or sent over IPC never contains secret
   material, file contents or an internal path. Errors are `{code, message}` with a stable `code`.
4. **The content-security policy is closed.** No inline scripts, no inline `style` attributes, no remote fonts,
   scripts, images or analytics. All network I/O happens in Rust and only to a target the user configured.
5. **Crypto changes are format changes.** Touch `src-tauri/src/crypto/` only together with
   [`CRYPTO_SPEC.md`](ai-docs/CRYPTO_SPEC.md) and the stored fixtures; the known-answer tests must keep passing,
   and old vaults must still open. `kat_vectors.rs` is generated by `scripts/gen_crypto_kat.py`: never edit it by hand.
6. **New dependencies are checked first.** Look up the latest *stable* version on the registry, read the real API if
   it is unfamiliar, pin the exact version, and record it in `ARCHITECTURE.md` §2. Prefer no new dependency. Many
   RustCrypto crates changed major versions in 2026: do not code them from memory.
7. **No background activity.** No timers, polling, tray icon, notification or foreground service. Sync runs on
   four triggers only (unlock, a saved change, leaving the app, the device coming back online). `scripts/check_no_notifications.py` enforces part of this.
8. **Docs move with the code.** If behaviour, a command, an error code or a limit changes, update the matching file
   in `ai-docs/` in the same pull request, and say plainly in `CHANGELOG_AND_STATE.md` what you did **not** verify.
9. **Say what you could not test.** "Not run on a device" is a fine thing to write; an unverified claim is not.

### Code style

* Rust: `cargo fmt` and `cargo clippy --locked --all-targets -- -D warnings` must be clean. No `unwrap()` on input
  that comes from a file, the network or the UI. Zeroize secret buffers; avoid `Debug`, `Clone` and `Serialize`
  on secret types.
* TypeScript: strict mode, no `any` for IPC data, UI strings in plain language (what the user did and what to do
  next, not how the system works).
* Commit messages: a short imperative summary line (`Fix duplicate detection for Ente Auth links`), then why.

## Pull requests

1. Fork, branch from `main`, keep the change focused (one concern per pull request).
2. Fill in the pull-request template: what changed, how you verified it, what you did not verify.
3. Keep CI green. A red job is a finding, not noise; if you believe a failure is flaky, say which job and link the run.
4. A maintainer reviews for correctness first (secret handling, failure paths, format compatibility), then clarity.
   Expect questions; they are not criticism.

## Releases (maintainers)

A release is made by raising the version on `main`. Nobody tags or publishes by hand.

1. **One command sets the version everywhere.** The file `VERSION` is the single source of truth; `scripts/version.py` copies it into
   `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` (the `hexis` entry
   only) and `packaging/arch/PKGBUILD`, and turns `## [Unreleased]` in `CHANGELOG.md` into `## [X.Y.Z] - date` with a new empty one above it.
   ```
   python3 scripts/version.py suggest      # what the CHANGELOG's [Unreleased] section calls for, and why
   python3 scripts/version.py bump auto    # do it (or: bump major | minor | patch, or: set 1.0.0)
   python3 scripts/version.py check        # every file agrees (scripts/verify_all.sh runs it; CI runs it before it publishes)
   ```
   **Nobody, and no AI agent, edits a version number by hand.** `auto` reads the CHANGELOG: a `BREAKING` mark or a `### Removed` section is
   a major bump (a minor one before 1.0.0, when a minor bump may change a stored format), a `### Added`, `### Changed` or `### Deprecated`
   section is minor, only `### Fixed` or `### Security` is patch, and an empty `[Unreleased]` is refused. So the CHANGELOG entry is written
   first, and the bump follows from it. Android's version is made by Tauri from `tauri.conf.json` at build time.
2. Merge or push that to `main`. The `ci.yml` workflow sees that `vX.Y.Z` has no tag yet, builds every variant from a clean
   cache, runs every test and, **only if every job passes**, tags that commit, **publishes** the release (notes, downloads
   table with checksums, the changelog section) and asks the site to rebuild its signed apt and rpm repositories. A version
   with a suffix such as `0.3.0-rc1` is published as a pre-release.
3. CI stops early, before building anything, if any file disagrees with `VERSION` (`python3 scripts/version.py check --release`) or `CHANGELOG.md` has no section for
   the version. A push that does not raise the version is an ordinary build (it only refreshes the rolling `nightly`).
4. **Where the newest build is.** Every push to `main` builds every variant; when all jobs pass, the rolling **`Nightly (main, <sha>)`**
   pre-release on the Releases page is replaced with that build (Linux packages and the Android APK). The release marked **Latest** is a
   different thing: it moves only when the version is raised. If you pushed and *Latest* did not change, the version was not raised;
   the new build is the nightly, and its commit is in its title.
5. To hold a release back, do not raise the version on `main`. To redo a release that was published by mistake and that nobody
   has installed yet: `gh release delete vX.Y.Z --cleanup-tag --yes`, then `gh workflow run CI` (it publishes again from `main`).
   This rewrites a public release, so keep it for mistakes.
6. The bundle identifier `dev.habibullah.hexis` is final (decision D9 in `ARCHITECTURE.md`): an Android `applicationId`
   cannot change once the app has been distributed, and the desktop data folder is named after it.
7. The release description (install commands, downloads table with checksums, the changelog section) is written by
   `scripts/release_notes.py` from the `## [X.Y.Z] - date` section of `CHANGELOG.md`.
8. One-time repository setup and the signing secrets: [`scripts/github-setup.sh`](scripts/github-setup.sh).

### What a release contains, and what it does not

CI builds and attaches the Debian package, the RPM, the AppImage and the tarball (64-bit x86), the Android arm64 APK signed with the
repository's release key, and `SHA256SUMS`; then it starts the Pages workflow, which rebuilds the signed apt, rpm and pacman repositories
and the installer from the newest stable release. **Not built by this pipeline, so not released:** Linux on aarch64, Windows, macOS, iOS.
The browser extension is not a release file: the app carries it and writes it to the data folder (see below for the stores).
**The order for a release:** merge the feature pull requests; write the `[Unreleased]` section of `CHANGELOG.md` (it becomes the release
notes, so it needs a summary line and, when something needs care while updating, an "Upgrading from X" paragraph); on a branch of its own
run `python3 scripts/version.py bump auto`; open the pull request; merge it with a merge commit once its checks pass; watch the CI run on
`main` (the `release` job is the last one); then open the release page and check the files, the checksums and the notes, and that the
Pages run that follows has finished. A release of a version with a suffix such as `0.6.0-rc1` is a pre-release.

### Publishing the browser extension (owner steps, not automated)

A release of Hexis never needs a store: the app carries the extension and writes it to `<data folder>/extension/` when Settings >
Browsers is switched on. A store only makes installing easier, and for Firefox it is the only way to a permanent install, because a
release Firefox runs an add-on only when Mozilla has signed it. Nothing below runs in CI (it needs your accounts and keys) and **none of it
has been tried**: it was written on 2026-11-25 from Mozilla's and Google's own pages, so expect the dashboards to differ in small ways.

Before either store, and again for every new submission:

1. `node --test extension/tests/*.test.mjs` and `python3 scripts/build_extension.py`. The second writes `extension/dist/hexis-passkeys-chromium.zip`
   and `extension/dist/hexis-passkeys-firefox.zip` (ignored by git). Nothing is minified or compiled, so the zip is also the source a
   reviewer may ask for.
2. Raise `"version"` in **both** `extension/manifest.chromium.json` and `extension/manifest.firefox.json` (the extension's version is its
   own, 0.1.0 now, not Hexis's), because a store refuses a version it has seen. Run the script again (it also checks that the two
   manifests and the file list agree). The app embeds these exact files, so a changed manifest is a changed app: it goes out in a Hexis
   release too.
3. Have a privacy policy to point at (`PRIVACY.md`, published on the site) and the plain facts for the forms: the extension sends a
   website's passkey request and the page's address to the Hexis program on the same computer through the browser's native messaging, never
   to a server; it keeps nothing. Its permission is `nativeMessaging`, and its scripts run on every `https` page and on `localhost`,
   because any website may ask for a passkey.

**Firefox (addons.mozilla.org).** The add-on id is already in the manifest (`hexis-passkeys@habibullah.dev`), which a Manifest V3 add-on
needs before it can be signed.

1. Create an account on addons.mozilla.org and make an API key pair on its credentials page (`/developers/addon/api/key/`): a JWT issuer
   (`user:12345:67`) and a secret. Keep both out of the repository.
2. Unpack the Firefox zip and sign it. Without a public page (self-distribution), then with one:
   ```
   unzip -q extension/dist/hexis-passkeys-firefox.zip -d /tmp/hexis-firefox-ext
   npx web-ext sign --source-dir /tmp/hexis-firefox-ext --artifacts-dir extension/dist --channel=unlisted \
       --api-key "$AMO_JWT_ISSUER" --api-secret "$AMO_JWT_SECRET"
   # a public listing instead: --channel=listed --amo-metadata=<your metadata JSON file> (web-ext 8 or newer)
   ```
   Mozilla's automatic checks can take up to a day, longer when a person reviews it. The result is a signed `.xpi`.
3. **Unlisted:** host the `.xpi` yourself, for example as a file on the release page (`gh release upload vX.Y.Z <file>.xpi`, which is not
   in `SHA256SUMS`, so say its checksum in the notes). A server must answer with `Content-Type: application/x-xpinstall` for Firefox to
   offer the install from a click; a release asset does not, so people save it and use *Add-ons > gear > Install Add-on From File*.
   **Listed:** Firefox updates it by itself. Either way, Mozilla's policies apply to what is sent to a native application: the listing
   must say what is sent. The manifest declares `data_collection_permissions` as `none` because nothing leaves the computer; if the
   review asks for something else, change it and say so in `CHANGELOG.md`.
4. When the signed add-on exists, change what the app and the docs say about Firefox (Settings > Browsers, `README.md`,
   `PASSKEYS.md` §10, `CHANGELOG.md`: "not signed by Mozilla").

**Chrome Web Store (Brave and Vivaldi install from it too; Microsoft Edge has its own add-ons store).**

1. Register on the Chrome Web Store developer dashboard: a one-time registration fee (the dashboard shows the amount), and a developer
   email that cannot be changed later.
2. **The extension's id is decided by the store, so settle it before you publish.** Upload the Chromium zip with *Add new item* and do
   **not** publish. If the dashboard refuses the manifest because of its `key` field, upload a copy without it. On the *Package* tab,
   *View public key* shows the key of the store item. Put that one line (no line breaks) in `extension/manifest.chromium.json` as `"key"`,
   so that an unpacked copy and the store item have **one** id, then check `chrome://extensions` shows the dashboard's Item ID.
3. Replace the old id (`pejbejenceojhjhcanlfnibgdihclnhm`, made from the key in the repository today) with the new one everywhere it
   is written: `src-tauri/src/bridge/mod.rs` (the `CHROMIUM` constant, which the host manifest's `allowed_origins` is made from),
   `src/backend/mock.ts`, `e2e/real-bridge.mjs`, `ai-docs/PASSKEYS.md`, `ai-docs/ARCHITECTURE.md`, `extension/README.md`. Then
   `python3 scripts/build_extension.py`, `cargo test`, `node e2e/real-bridge.mjs`, and ship a Hexis release. **Publish the store item
   only after that release is out**: until then the store's id is not one the host allows, and the store item cannot reach Hexis. People
   who already connected a browser press *Connect again* (Settings > Browsers marks the old registration), which rewrites the host manifest.
4. Fill in the listing (description, at least one screenshot, the icon is inside the package), the *Privacy* tab (the single purpose: lets
   the Hexis app on this computer answer a website's passkey request; a reason for `nativeMessaging` and for running on every `https`
   page; the data-use answers; the privacy policy address) and the *Distribution* tab (public or unlisted). A reviewer sees nothing
   without Hexis installed and an account, so give test instructions (the install command and the steps); expect questions about the
   broad page access.
5. *Submit for Review*, wait, then publish (a staged approval reverts to a draft after 30 days).

### Who can build and release an Android app

* **Anyone** can build an APK on their own computer (`npm run tauri android build`). It is signed with a debug key, so it
  runs, but it is not an official build and cannot update one.
* **A fork** has no access to this repository's secrets. Its CI builds a `-testkey` APK signed with a throw-away key made
  by that one run. That is on purpose: a copy of the project must not be able to impersonate the official app. A fork that
  wants its own releases creates its own key (`scripts/github-setup.sh android-key` in its own repository) and should
  change the bundle identifier, because two different keys cannot share one app on a phone.
* **A collaborator with write access** never touches the key. A change that raises the version and reaches `main` makes CI
  sign with the repository's secrets and publish the release. A build that would publish fails, rather than publishing a
  `-testkey` file, when the key is missing. Give write access, and the right to merge to `main`, only to people you would
  trust with the key and with the power to publish: anyone who can change a workflow on a branch of this repository can
  make it reveal the secrets.
* **The key itself** is made once by the owner with `scripts/github-setup.sh android-key`. It is stored in GitHub as an
  encrypted secret (it can never be read back) and kept as a backup file outside the repository. If it is lost, phones
  that installed Hexis cannot be updated; they have to uninstall first, so restore from a backup afterwards.

## Licence of contributions

Contributions are licensed under the project's [MIT licence](LICENSE).
