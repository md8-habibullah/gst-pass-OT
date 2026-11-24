# CHANGELOG_AND_STATE — where the project is right now

Update this file after every major milestone. Newest entries first.

## Update (2026-11-24, after the 0.2.0 push): the first CI run of the multi-S3 e2e, and a faster pipeline

* **What failed:** the commit `3487630` went green everywhere except the end-to-end job, where only `e2e/real-multi-s3.mjs` failed ("did not become ready", every other e2e script passed, `real-sync` 26/26). The cause was the test, not the app: it signed its S3 requests with `curl --aws-sigv4`, and curl 7.81 (what Ubuntu 22.04 CI runners ship) leaves `x-amz-content-sha256` out of the signed headers, so the S3 server answered 403 to every probe; curl 8.x signs it, which is why it passed on the dev machine. Reproduced in an `ubuntu:22.04` container against the same server image.
* **Fix:** `sigv4Headers()` in that script now computes the Signature V4 headers in Node (empty body) and hands curl finished headers. Checked with curl 7.81 (PUT, list with a query string, GET, and a forged signature that must give 403) and with the whole script against the real core: 25/25. The "not ready" error now also prints the last HTTP answer and the container's stdout and stderr.
* **Pipeline:** `e2e`, `linux` and `android` no longer wait for `checks` (they never used its output), so the four heavy jobs run side by side. `release` still needs every job, so nothing is published unless all of them pass. Not changed on purpose: tag builds start without a cache (clean release builds), and only GitHub's own actions are used.
* **Known gap:** `nightly` does not wait for `e2e`, so the rolling nightly was replaced by a build whose e2e job was red. Left as it was; add `e2e` to its `needs` if the nightly should be gated too.
* **The tag:** `v0.2.0` was pushed at `3487630` while the e2e job was red (the tag command list was not chained with `&&`). No release was opened, because the release job needs `e2e`. It has to be re-created at the commit that contains the fix.

## Update (2026-11-24, Q5): trusted time, time zones, a demo vault, shortcuts, layered settings, a calmer look

**Versioned 0.2.0** (`package.json`, `package-lock.json`, `src-tauri/Cargo.toml` and `Cargo.lock`, `tauri.conf.json`, `packaging/arch/PKGBUILD`; the changelog's accumulated Unreleased section is now `[0.2.0] - 2026-11-24`, with an upgrade note because `gate.json` and the item re-prompt flag change shape). Nothing is tagged: the owner tags `v0.2.0`, which opens the draft release. The website's home page lists the new features and the time check, and its stylesheet lost the lattice background and the gradient button to match the app; the site builds with all 40 links resolving. The social preview image was not regenerated (it needs `rsvg-convert`).

**Built (uncommitted; the owner commits):** the six parts of the request are in `FEATURE_QUEUE.md` Q5; the decisions are D27 to D30; the contract is `IPC_CONTRACT.md` §11; the files are `STORAGE_AND_SYNC.md` §13; the look is `UI_DESIGN_SYSTEM.md` §9. New Rust: `clock/` (boot clock, HTTPS time fetcher, `TrustedClock`), `storage/device.rs`, `storage/demo.rs`, `storage/app_demo.rs`, `commands/device.rs`; the gate's waits now carry a boot-clock deadline and a checked-UTC deadline (`gate.json` counter, MAC `v3`). New front end: the zone page, the first-run sheet, the shortcut table and overlay, eight settings pages, `lib/zones.ts`, `lib/motion.ts`, a rebuilt lock screen, a flat surface system.

**Verified (this machine, the real Rust core, headless Chromium):**

* Rust: `cargo test --lib` 841 passed, 0 failed, 2 ignored (the live network tests) on the final code. The first full run had 3 failures (two gate IPC tests that listed the old allow-list and status shape, and a test case of mine); they were fixed. New tests cover the clock, the waits with the check on and off, the demo vault, the device settings and the IPC shapes. `cargo clippy --all-targets -D warnings` and `cargo fmt --check` are clean; `tsc`, `vitest` (101) and `npm run build` pass, and the production bundle holds none of the preview backend.
* Security review of the working tree (a read-only agent over the clock, the gate waits, the allow-list, the demo vault, the shortcuts, the dev shim and the Android template): no finding met the bar. Checked by hand afterwards: the time fetcher is HTTPS-only with no redirects and certificate checks on (rustls, bundled roots).
* End to end: the main run 88/88 (first start, the zone page, the demo vault's 15 to 25 items, create / rename / delete accounts through the new Settings pages, the Date and time page, the shortcuts overlay, chords, `n`, `/`, Ctrl+K, Ctrl+,), and `real-gate` 75, `real-disguise` 16, `real-import` 13, `real-drive` 18, `real-backup` 13, `real-sync` 26, `real-multi-s3` 25 (two real S3-compatible servers in Docker), plus the preview-backend scripts `gate-ui`, `mobile-ui`, `sync-ui`, `files-ui`, `import-ui`. The scripts now run the shim with `--no-time-check` so they never touch the network.
* Screens reviewed as screenshots at phone and desktop width: the zone page, the first-run sheet, Date and time, the shortcut overlay, the phone settings list, the gate.

**Found and fixed on the way:** an unlayered `button { color: inherit }` beat every Tailwind v4 utility (all buttons had pale text) and now sits in `@layer base`; with the online check off, a restart would have locked the person out for good (`deadline_utc_ms` now falls back to the device clock only when the check is switched off); the first-run sheet disappeared because `status()` dropped the demo mark; the dev preview read the device before seeding it.

**Not verified:** the time servers from inside the app (checked with `curl` only; the live test in `clock/net.rs` is `#[ignore]`); that Android lets the app read `/proc/uptime` and the kernel boot id (if it does not, the boot clock falls back to a per-run id and a restart needs one online check); Windows and macOS (the boot id is per run there); a real phone; a real desktop window; that the Date header of the three sites is within 15 s of each other on every network (a captive portal or a proxy can break the quorum, in which case the wait stays "checking" until it works).

**Honest limits:** the demo vault's password is public, so anyone who opens the app sees the demo (that is the point: it holds nothing real). The check being on does not stop someone who can edit `gate.json` (D15). With the check off, a changed date can shorten a wait after a restart.

## Update (2026-11-24, night): v0.1.0 published, repository public and renamed, multi-S3 sync is the headline

**State on GitHub (checked from outside, without signing in):** the repository is public; `v0.1.0` is a published release marked Latest; all five files download anonymously and match `SHA256SUMS`; GitHub Pages is built with HTTPS enforced; private vulnerability reporting is on. The signed package repositories exist: the key on the site has fingerprint `A867 F074 B960 75BE 1E28 ED1B 9388 04F3 3177 B6FF`, the apt `InRelease` and `Release.gpg`, the rpm `repomd.xml.asc` and the pacman `hexis.db.sig` all verify with it, and the `.deb` hash in the apt index equals the release file's. The APK (`-testkey`, signed with a throw-away CI key) is arm64-v8a only, minSdk 28 (Android 9), targetSdk 37, v3 signature, permissions limited to internet, camera, biometrics and network state, with the default launcher entry enabled and the 11 disguise aliases off.

**Rename:** the repository is now `md8-habibullah/post-quantum-passman-gst` (was `gst-pass-OT`). The product is still Hexis; the slug is only a URL (ARCHITECTURE.md D9) and `scripts/tests/test_no_old_name.py` allows exactly that slug and fails on any link to the dead pre-rename address. GitHub redirects git and web URLs but **not Pages**: `https://md8-habibullah.github.io/gst-pass-OT/` answers 404. Everything that names the site is derived at build time (`@REPO@` and `@SITE@` in the installer, `GITHUB_REPOSITORY` in CI, `configure-pages` in `pages.yml`), so only the README had the old address written out. The site that was live right after the rename still carried the old address inside `install.sh` and `hexis.repo` because it had been built a minute before; any Pages run (a push touching the paths in `pages.yml`, or `workflow_dispatch`) rebuilds it with the new one. The v0.1.0 release description was written with the old address; `scripts/github-setup.sh release-notes v0.1.0` rewrites it. Nothing inside the release files names the address, so the release itself does not need to be cut again.

**Positioning:** README, site, package descriptions, release notes, the social preview and the repository description now lead with multi-S3 sync (one bucket or several: a master plus mirrors) instead of "local-first" alone.

**Open:** secret scanning, push protection and Dependabot security updates were found disabled on the public repository; `scripts/github-setup.sh apply` now turns them on and also replaces the topics (20 at most).

**Android signing:** the v0.1.0 APK is `-testkey` (a throw-away key; no `ANDROID_KEYSTORE_*` secrets existed when it was built). `scripts/github-setup.sh android-key` now creates the release key itself (PKCS12, RSA 4096, outside the repository, mode 600), stores `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` as secrets over stdin and the public fingerprint as the variable `ANDROID_CERT_SHA256`. Tested with a stand-in `gh`: refusals (mismatch, short, decline, existing file), file mode 600, and a real signing of the released APK with the stored values using CI's exact `apksigner` command, whose certificate digest equals the published fingerprint. CI now keeps the key away from pull-request runs and fails a version-tag build that has no key (override: variable `ALLOW_TESTKEY_RELEASE=true`). The key is NOT created yet; it needs the owner's password and must be run by the owner. After it exists, the next release (v0.1.1) carries a normally named, normally signed APK; the `-testkey` file in v0.1.0 should then be deleted from that release.

**Not verified:** sync against real S3 providers (AWS, R2, B2, Wasabi, MinIO, Storj) and real TLS or clock skew, only `scripts/mock_s3.py`; the one-line installer pasted from the live site on fresh distributions (CI runs the same installer against local copies of the repositories on eight images, all passed; a run against the live site was started and stopped because of the rename); the apps on a real desktop window and a real phone.

## Update (2026-11-24, evening): first CI run, the one-line installer, signed package repositories

**First run on GitHub (the owner's push, commit `maga-v2`):** all jobs passed (`checks`, `e2e`, `site`, `linux`, `android`, `nightly`); the `nightly` pre-release holds the deb, rpm, AppImage, tar.gz, the
Android APK (`-testkey`, verified with `apksigner`: v3 scheme, `dev.habibullah.hexis`, minSdk 28, targetSdk 37) and `SHA256SUMS`. The Pages site deployed (`status: built`, HTTPS enforced) while the repository is
still private. A later run on the Dependabot pull request failed once in `e2e`: the first headless Chrome on the runner did not answer within 10 s; `e2e/cdp.mjs` now waits 20 s and retries (this fix is in the
tree, not yet pushed). The CI-built binary needs glibc 2.34 (the local CachyOS build needs 2.39, which is why the local files must not be used for compatibility tests); the AppImage's libraries need 2.35.

**One-line installer (`scripts/install.sh`)**, built after studying `dl.brave.com/install.sh` (a thin script that registers the vendor's signed repository and lets apt/dnf install and update). Hexis's version
detects the system and picks a signed apt or rpm repository, or a checksum-verified tarball/AppImage/APK; details, flow, trust model and the verified matrix are in `BUILD_TARGETS.md` section 11. Also new:
`scripts/make-repos.sh`, `scripts/make-arch-repo.sh`, the `pages.yml` repository step, `scripts/github-setup.sh repo-key`, the `installer` CI job (`scripts/test_installer_containers.sh`, `scripts/tests/container_case.sh`),
33 hermetic tests (`scripts/tests/test_install_sh.py`), the Download page and `/download/install-script/` on the site, and the README, SECURITY, PRIVACY, CONTRIBUTING and CHANGELOG sections.

**Verified for real (containers, CI-built files):** Ubuntu 22.04, Debian 12, Fedora, openSUSE Tumbleweed, Arch (installs through the signed repository of each family, libraries resolve, runs under Xvfb, uninstall), refusals on Alpine, Ubuntu 20.04, aarch64;
tampered checksum refused; APK download. **Not verified:** the Pages repository build on a real runner (needs `REPO_SIGNING_KEY` and a published stable release), the `installer` CI job itself (written after the run),
`pkexec`/`run0`/`doas`, rpm-ostree and SteamOS hosts, `make-arch-repo.sh` on a GitHub runner, aarch64 Linux. **Needs the owner:** make anonymous downloads possible (see below), create the signing key
(`scripts/github-setup.sh repo-key`), tag `v0.1.0`, review and publish the draft release.

**Private-repository caveat:** while the repository is private, release assets answer 404 to anonymous downloads, so the one-liner cannot download release files for anyone who is not signed in with `gh` (the installer
tries `gh release download` as a fallback). The package repositories are an exception: they live on Pages, which is public, and are built from the stable release with the workflow's own token.

## Update (2026-11-24, later): Android builds, the repository got CI, releases and a website

**What changed**

* **Android was built for the first time** (it had never been built before). With the SDK installed (see `BUILD_TARGETS.md` 4.1), `tauri android init`, the manifest patch script, the secure-screen plugin and a
  debug build for aarch64 all worked: a 252 MB debug APK. Findings from the real build, all fixed or recorded: the disguise patch script ran against a real generated project for the first time (125 changes,
  idempotent); `check_no_notifications.py` takes no path argument (my earlier invocation was wrong); the merged manifest carried `VIBRATE` (now removed by the patch script) and a *required* camera feature
  (now optional); the template compiles against API 37, not 36. `apkanalyzer` shows `DisguisePlugin` and `SecureScreenPlugin` in the dex and no notification or foreground-service permission.
* **On-device checks did not run: the phone refused the install** (`INSTALL_FAILED_USER_RESTRICTED`; it was asleep and locked, and Xiaomi's "Install via USB" needs the owner). Steps in `BUILD_TARGETS.md` 4.4.
  FLAG_SECURE, lock on background, the 12 launcher aliases, biometrics, camera scan and `adb reverse` sync therefore remain **unverified**.
* **New privacy finding (open):** the QR scanner plugin uses Google ML Kit via Play services and the APK contains Google's data-transport runtime. ML Kit may send technical usage metrics to Google. Hexis's own
  code sends none, but "no telemetry" can no longer be stated without this Android exception (`PRIVACY.md` and the README now say so). Not measured. Fix options: an on-device decoder (`jsqr` is already bundled) or
  removing the transport services and re-testing the scanner.
* **`scripts/verify_all.sh` had a bug:** the three Rust steps ran in a subshell, so a failing `cargo test` or `clippy` set `fail` only inside it and the script could still print `ALL CHECKS PASSED`. Fixed (the steps now run
  in the main shell). Earlier "passed" results were read from the test logs directly, but any run before this fix relied on the script's last line alone for the Rust part.
* **Repository set up for release:** `LICENSE` (MIT; the copyright holder line is the owner's to confirm), `CONTRIBUTING.md`, `SECURITY.md`, `PRIVACY.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`, issue forms and a pull-request template,
  Dependabot, a full `README.md`, `packaging/arch/PKGBUILD`, `.github/workflows/ci.yml` and `pages.yml`, the website builder, and `scripts/github-setup.sh`. Details and limits: `BUILD_TARGETS.md` sections 9 and 10.
* The `README.md` was never longer than a short stub (597, 1,117, then 2,634 bytes over three commits); nothing had been cut. The architecture lives in `ai-docs/` (eight files, about 285 KB, unchanged).

**Local dry run of the CI build jobs (2026-11-24, real tree, same commands; the packaging and signing steps were extracted from `ci.yml` and executed as written)**

| Step | Result |
|---|---|
| `npm run tauri build -- --bundles deb,rpm,appimage` (release, LTO) | exit 0; Rust release compile 7 min 28 s; `Hexis_0.1.0_amd64.deb` 8.1 MiB (Depends `libwebkit2gtk-4.1-0, libgtk-3-0`), `Hexis-0.1.0-1.x86_64.rpm` 8.1 MiB (requires `libwebkit2gtk-4.1.so.0`), `Hexis_0.1.0_amd64.AppImage` 112.7 MiB |
| CI step "Collect, name and archive" | exit 0; `hexis-0.1.0-linux-x86_64.tar.gz` holds the binary, `hexis.desktop`, `hexis.png`, `LICENSE` |
| `npm run tauri android build -- --apk --target aarch64` (release) | exit 0; 18.2 MB APK |
| CI step "Align and sign" (no release key, so a throw-away key) | exit 0; `hexis-0.1.0-android-arm64-testkey.apk`; `apksigner verify` passes for minSdk 28; the run exposed a stray `.idsig` file, now disabled with `--v4-signing-enabled false` |
| R8 on the release APK | `DisguisePlugin` and `SecureScreenPlugin` survive; permissions are `INTERNET`, `ACCESS_NETWORK_STATE`, `CAMERA`, `USE_BIOMETRIC`, `USE_FINGERPRINT` (no `VIBRATE`, no notification, no foreground service) |
| `scripts/verify_all.sh` (the `checks` job) | `ALL CHECKS PASSED`, exit 0: 764 Rust tests, 66 frontend tests, clippy and fmt clean |
| `scripts/ci_e2e.sh` (the `e2e` job) | all 7 suites OK, 179 checks, 0 failures (`run` 60, `real-gate` 35, `real-drive` 18, `real-import` 13, `real-disguise` 16, `real-backup` 12, `real-sync` 25). The first run found a bug in the script itself (it probed Vite on `127.0.0.1` while Vite listens on `localhost`); fixed |
| Site build with its link and anchor check | 18 pages; a deliberate broken link and a broken anchor each make it exit 1; injected raw HTML is shown as text |
| Workflow files | parse as YAML; every `needs` target exists; every action is a first-party `actions/*@vN` |

**Not verified (new)**

1. **The GitHub workflows have never run.** Nothing was pushed. The commands inside them ran locally, but not on a GitHub runner: the `ubuntu-22.04` apt packages, the runner's own Android SDK and `sdkmanager` (the workflow installs `platforms;android-37.0`, `build-tools;36.0.0`, `ndk;28.2.13676358` and accepts the licence), artifact upload and download, `gh release create` and the draft-release path, and signing *with* the secrets (only the throw-away-key branch ran). Read the first run before relying on it.
2. **GitHub Pages:** the repository is private. Pages from a private repository needs a paid plan and the site is public regardless; the first deployment has not happened.
3. A full `makepkg` of the PKGBUILD; Windows and macOS builds (not targets); build provenance attestations; an `aarch64` Linux build.

## Current state (2026-11-24): the five queued slices (Q0 to Q4) are built, merged into one tree and verified

Phase 2 (storage, history, trash, UI), Phase 4a (multi-S3 backup, master backup, restore) and the five slices of `FEATURE_QUEUE.md` are in one tree: **Q0** event-driven sync (three triggers, no timers, pull and reconcile), **Q1** Stage 1 gate with decoy maths, panic and wipe and the idle lock in Rust, **Q2** the 11-name disguise, **Q3** the encrypted 50 MB drive (schema v4), **Q4** the Rust importer and plaintext exporter (its attachments go into the drive). Each section below this one has its own log entry, contract and honest limits.

### Verified (fresh run 2026-11-24, `scripts/verify_all.sh --e2e`, ALL CHECKS PASSED)

| Check | Result |
|---|---|
| `tsc --noEmit`, `vitest run`, `vite build` | clean; 66 / 66 tests; no dev-only strings in `dist/` |
| `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` | clean |
| `cargo test --locked` | **764 passed, 0 failed, 1 ignored** (the fixture rewriter) |
| `scripts/check_no_notifications.py`, Android patch script tests | pass |
| Real Rust core behind the dev shim, driven from headless Chromium (`e2e/`) | `run` 60/60, `real-gate` 35/35, `real-drive` 18/18, `real-import` 13/13 (25/25 when pointed at real exports), `real-disguise` 16/16, `real-backup` 12/12, `real-sync` 25/25 |
| Two real vendor exports (Bitwarden web-vault JSON, Ente Auth plain text) | import correctly end to end; they found and fixed three defects (Ente `codeDisplay`, a wrong one-time-code count in the preview, a test that looked in the wrong place for attachments). See the Q4 section |

### NOT verified (the honest list for the merged tree)

1. **Android has not been built or run.** The toolchain is only partly installed (cmdline-tools, JDK 21 and the rust target are in; the SDK licences, platform-tools, platform 36, build-tools and the NDK are not, because the licences need the owner to accept them: `scripts/setup_android_toolchain.sh`). Everything Android-specific is untested: the activity aliases of the disguise, `FLAG_SECURE`, the biometric prompt, the lifecycle hooks, the camera QR scanner, and on Android the dialogs return `content://` addresses, so importing, adding drive files and saving copies do not work there yet.
2. **No real desktop window run of the merged app** (rule: no synthetic input into the desktop). The native dialogs and the launcher refresh of the disguise are covered by the shim, the test picker and a recording launcher, not by a real window.
3. **Real S3 providers** (AWS, R2, B2, MinIO, Wasabi, Storj), real TLS and clock skew: only `scripts/mock_s3.py` was used.
4. **Vendor formats other than the two real files above** were read from public descriptions and synthetic fixtures. KeePass `.kdbx` files are real KDBX but were written by the crate that reads them.
5. **The Stage 1 gate is a privacy gate, not encryption** (nothing wraps the vault key; a copied data folder resets the attempt counter; the overwrite on wipe cannot be proven on SSD or copy-on-write file systems). The vault itself needs the master password every time.
6. No third-party review of the crypto, storage, gate or importer code; no external fuzzer; power-loss durability is not measured.
7. ~~Placeholder bundle identifier~~ Decided: `dev.habibullah.hexis` is final (decision D9, 2026-11-24).

The sections from "Earlier state" down are the 2026-11-23 record; where they say "not built", the slices above supersede them.

## Earlier state (2026-11-23): **Phase 2 — local storage, history, trash and the full UI are built; Phase 3 (lock states, quick unlock, Android shell) is next**

After the user ran the system-package command and said "You can continue", Phase 2 started. Mid-turn the user issued a **UI course correction** (bespoke dark GUI, 1Password / Proton Pass / Apple Keychain feel, platform-adaptive): the working Rust crypto was left untouched and the frontend was rebuilt to that brief (`UI_DESIGN_SYSTEM.md`).

| Item | Status |
|------|--------|
| `ai-docs/` (7 files) | done; `IPC_CONTRACT.md` and `UI_DESIGN_SYSTEM.md` are new |
| `src-tauri/src/crypto/` | done (Phase 1), untouched since |
| `src-tauri/src/storage/` (SQLite via rusqlite 0.43, items, immutable versions, trash, purge, TOTP, re-prompt session) | done, tested through the Rust API |
| `src-tauri/src/commands/` (20 IPC commands, camelCase wire) | done, tested over Tauri's real command dispatch (mock runtime) |
| Frontend "Strongroom" UI (`src/`): lock/setup, split-pane shell, bottom-nav compact layout, list, detail, editor, version history, trash and two-step purge, codes, settings, re-prompt guard | done, verified in headless Chromium at desktop and Android widths; first-run screen verified in the real WebKitGTK window |
| Lock states, PIN / biometric, auto-lock timers, clipboard clear timer on the Rust side, FLAG_SECURE | **not built** (Phase 3). The PIN and biometric UI exists only against the preview backend |
| Attachments and encrypted media blobs | placeholder UI only (Phase 5) |
| Multi-S3 sync | **not built** (Phase 4); the UI says "Local only" |

### Verified (fresh runs 2026-11-23, after the Phase 2 work)

| Check | Result |
|-------|--------|
| `cargo test --locked` (full crate) | **152 passed, 0 failed, 1 ignored**: 84 crypto (unchanged), 66 storage, 2 end-to-end IPC tests through the Tauri mock runtime (camelCase argument names, snake_case rejected, errors cross as `{code,message}`). The ignored test regenerates crypto fixtures on purpose |
| `cargo clippy --locked --all-targets -- -D warnings`, `cargo fmt --check` | clean |
| Storage behaviour covered by tests | create / unlock / wrong password; item create, update with a single-use edit token and `expectedVersion` conflict detection; immutable history and restore-as-new-version; trash lists only id, title and deletion time; purge needs password plus a matching confirmation; tampered ciphertext is reported and skipped (`damagedItems`) instead of locking the vault; lock wipes the edit tokens; change-password re-wraps only the key bundle |
| `npx tsc --noEmit` (TypeScript 7.0.2, strict) | clean |
| `npm test` (vitest 5.1.1) | **22 passed** (password generator, rejection sampling, strength meter, formatting) |
| `npm run build` | 464 kB JS (146 kB gzip), 44.5 kB CSS; no preview-backend strings in `dist/` (grep for the demo password, PIN, seed data and banner text finds nothing) |
| Real window (`npm run tauri dev`, WebKitGTK on Hyprland) | the app opened and showed the first-run "Create your vault" screen, answered by the real Rust `vault_status`: bundled fonts, lattice texture, glass blur and the keyhole ring render under the strict CSP (`style-src 'self'`). React and Motion apply styles through the CSSOM, which that CSP allows |
| Headless Chromium against the Vite dev server (preview backend) | screenshots reviewed at 1440x900 and 412x915 for: setup, locked, list + detail, version history, trash, re-prompt dialog, edit sheet, new-item sheet, settings, codes |

### NOT verified yet (be honest about these)

1. **No end-to-end run of the new UI against the real Rust backend in a real window.** The IPC contract is verified from the Rust side and the UI is verified against an in-memory implementation of the same TypeScript `Backend` interface, but nobody has yet created a vault, added an item, revealed a password, restored a version or purged an item in the actual app window. An attempt to drive the window with synthetic keystrokes was abandoned after it misfired (see the log); the manual smoke test in "Next steps" covers this.
2. Motion transitions (sheet slides, layout animations, unlock moment) and `backdrop-filter` under load are unchecked in WebKitGTK beyond the first paint. Android WebView is unchecked entirely.
3. **Android** (`BUILD_TARGETS.md` §4): no SDK/NDK on this machine; `npm run tauri android init` has not been run. The 412x915 screenshots are a Chromium proxy for the layout only.
4. **Bundles** (deb/rpm/AppImage/PKGBUILD) have not been built.
5. ~~**Placeholder bundle identifier**~~ Decided 2026-11-24 (D9): `dev.habibullah.hexis` is final; it cannot change later without a new app identity.
6. The crypto and storage code have had no third-party review. Power-loss durability, byte-level secure deletion after purge, and unlock speed with production Argon2 parameters (64 MiB) are not measured.
7. In development the app keeps its data in `~/.local/share/dev.habibullah.hexis/` (the folder exists, mode 0700, created by the app at launch). `vault.db` has **not** been created yet: the assistant only looked at the first-run screen and never created a vault in the real app.

<details><summary>Phase 1 verification record (unchanged)</summary>

### Verified (fresh run 2026-11-23)

| Check | Result |
|-------|--------|
| `python3 scripts/crypto-harness.py test` | **84 passed, 0 failed, 1 ignored** (the ignored one rewrites fixtures on purpose) |
| `python3 scripts/crypto-harness.py clippy` (`-D warnings`, all targets) | clean |
| rustfmt on the crate (`lib.rs` → `mod crypto`) | clean (`kat_vectors.rs` is generated and marked `#[rustfmt::skip]`) |
| Known-answer vectors from independent implementations | Argon2id, HKDF-SHA512 sub-keys, NFKC, hybrid combiner, XChaCha20-Poly1305 (incl. IETF draft vector), envelope bytes; `gen_crypto_kat.py --check` confirms the committed vectors are current |
| TOTP | RFC 6238 Appendix B vectors (SHA-1/256/512), cross-checked against a stdlib HOTP |
| Interop | `scripts/verify_bundle_fixture.py`: pure Python (`cryptography` 50.0.2) unlocks the **Rust-written** `fixtures/bundle_v1.json` end to end (Argon2id → keyring → ML-KEM-1024 + X25519 decapsulation → combiner → VaultRootKey → commitment → item key → stored item) |
| Format regression | `kat::bundle_v1_fixture_still_unlocks` opens the stored v1 bundle and item with today's code |
| Crate versions tested == versions shipped | harness lock vs `src-tauri/Cargo.lock`: every crate the crypto code uses has the identical version. Extra older duplicates in the real lock (sha2 0.10, digest 0.10, crypto-common 0.1, getrandom 0.3, thiserror 1, base64 0.21) come only from wry, tauri, the gtk/glib bindings, jni and swift-rs; no crypto crate depends on them |
| Frontend: `npm install`, `npm run typecheck`, `npm run build` | all exit 0 with TypeScript 7.0.2, Vite 8.3.8, plugin-react 6.3.0, Tailwind 4.3.5 (51 packages; bundle 222 kB JS / 8.5 kB CSS) |
| `dist/index.html` vs the CSP | one external module script, no inline script |
| Full `hexis` crate (after the user installed webkit2gtk-4.1 on 2026-11-23) | `cargo check --locked --all-targets`: 0 warnings; `cargo clippy --all-targets -- -D warnings`: clean; `cargo fmt --check`: clean; `cargo test`: **84 passed, 0 failed, 1 ignored** (same as the harness); `tauri-build` accepted `tauri.conf.json` and `capabilities/default.json` |
| App launch (`npm run tauri dev`, Wayland/Hyprland) | window `Hexis` opens within ~4 s and stays open (sampled for 90 s); a screenshot shows the status page filled from the Rust command `crypto_suite_info` (suite v1, ML-KEM-1024+X25519, Argon2id 64 MiB/4/4, XChaCha20-Poly1305); the strict CSP blocked nothing visible |
| `tauri.conf.json` | validated against the downloaded Tauri JSON schema with a stdlib checker (unknown keys, required keys, types, enums); the checker was itself tested by injecting two faults, both caught |

Test split by module: aead 3, envelope 12, error 1, ids 2, kat 8 (+1 ignored), kdf 6, kem 13, rng 1, secret 7, subkey 2, totp 14, vault_keys 14, mod 2.

</details>

### Rules the UI follows because of the CSP (`tauri.conf.json`)

`style-src 'self'` and `script-src 'self'`: no inline `<script>`, no HTML `style="…"` attributes, no `<style>` blocks, no `eval`. Styles come from Tailwind classes and the bundled stylesheet; React `style` props and Motion work because they write through the CSSOM (`element.style.x = …`), which this CSP does not block (confirmed by the first-run screen in the real window). `connect-src` is limited to Tauri IPC; S3 traffic in Phase 4 goes through Rust, not the webview.

## Environment notes (dev machine)

* Linux CachyOS, ~7.4 GB RAM (often 1.5–3.5 GB free): run heavy jobs one at a time (`CARGO_BUILD_JOBS=2`).
* Rust 1.100.0 installed via rustup on 2026-11-23 (minimal profile + clippy + rustfmt). Node 26.13.1, npm 12.2.3.
* `webkit2gtk-4.1` 2.54.3 and the other Tauri system packages are installed (the user ran the pacman command from `BUILD_TARGETS.md` §2 on 2026-11-23). `bc` is not installed.
* No Android SDK installed.
* Git: this directory is not a git repository; the user handles all commits. Nothing in this project has ever been committed or pushed by the assistant.

## Log

### 2026-11-24 — Q0 (slice R3a): sync without timers, exit flush, pull and reconcile
* **Event-driven sync (D18).** The replicator thread sleeps on a condition variable and runs only at unlock, after a committed local change, and at the exit or background flush (plus "Back up now"). A failed job stays queued with its error and is tried again at the next trigger: no timer, polling, backoff or retry schedule remains (`Timing` is now `lagging_after`, `failing_after`, `background`). A settled worker makes no store calls (tested). One trigger counts as one failed attempt. `backup_run` = reconcile, then drain.
* **Exit and background flush.** `AppCore::flush_for_exit(8 s)`; Linux `CloseRequested` hook in `lib.rs` (the close is held back, the flush runs on its own thread, then the window is destroyed; a second close request closes at once; an empty queue closes at once); `#[cfg(mobile)] WindowEvent::Suspended` (Android `onPause`) flush; IPC `sync_flush -> { complete }` for the page's `visibilitychange`. `scripts/check_no_notifications.py` guards the "no notification, no tray" rule. Details and limits: `BUILD_TARGETS.md` §8.
* **Pull and reconcile** (`storage/backup/pull.rs`, `pull_apply.rs`), on the worker at unlock (never blocking `vault_unlock`) and at "Back up now": source = master, else the first enabled target; list items and tombstones, compare ETags, GET only what changed, authenticate every object, tombstones win, rollback guard, per-item lineage through sealed `parent` pointers, fast-forward, local-ahead, and diverged states kept as "from another device" snapshots (nothing overwritten, nothing merged silently). The apply is one transaction under the session lock that re-checks the head, so a local edit during a pull wins. Algorithm: `STORAGE_AND_SYNC.md` §6 and §10.
* **Schema v3** (`versions.origin`, `remote_version`, `reviewed`; v1 and v2 files migrate in one transaction). **Contract (additive):** `VaultStatus.syncRevision`, `reconciling`, `remoteDamaged`; `ItemView.changedElsewhere`; `VersionEntry.otherDevice`; commands `item_review_other_device`, `sync_flush` (`IPC_CONTRACT.md` §2, §3, §6.2). Item version numbers can now have gaps.
* **Verified (2026-11-24):** `cargo fmt --check` clean; `cargo clippy --locked --all-targets -- -D warnings` clean; `cargo test --locked` **329 passed, 0 failed, 1 ignored** (baseline 302; three backoff tests removed, about 30 added: no-timer tests, two simulated devices on `MemStore`, a pull through the real `S3Store` against `scripts/mock_s3.py`, IPC tests for `sync_flush` and the other-device flow); `python3 scripts/check_no_notifications.py` exits 0. The dev shim (`examples/e2e_server.rs`, flags unchanged) was rebuilt and answers `sync_flush` and `vault_status` with the new fields.
* **NOT verified:** the Android lifecycle code (`cfg(mobile)`, no SDK on this machine; `onStop` is not forwarded by tauri-runtime-wry 2.12.5); the real window's `CloseRequested` path (only the Tauri-free gate logic is tested); any real S3 provider. Pre-R3a item payloads have no `parent`, so a state written by an older build counts as "diverged" instead of fast-forwarding (conservative, pre-release only).

### 2026-11-23 — Phase 2 and the UI rebuild
* **Audit, then course correction.** After the user's "STOP and audit" message the repo and `ai-docs/` were checked: Rust crypto done and tested, frontend still a placeholder status page, no storage layer. The crypto was kept as is; the frontend was designed from scratch to the new brief.
* **Design system "Strongroom"** (`UI_DESIGN_SYSTEM.md`): deep-marine ground with a faint triangular lattice, frosted panes, brass accent for the single primary action, verdigris for "secure / fresh code", ember for danger. Fraunces for titles and empty states, Instrument Sans for the interface, IBM Plex Mono only for secrets and codes, all bundled locally. One orchestrated moment: the unlock. Deliberately avoided: aurora gradients, uppercase tracked labels, identical cards for every field, mono for decoration.
* **Adaptive shell** (`useLayout`): wide (>=1100 px) = sidebar / list / detail; medium (720-1099) = icon rail / list / detail; compact (<720) = single column, bottom navigation, FAB, detail as a full page with working Back, bottom sheets for add / edit / re-prompt, safe-area insets, 44-48 px touch targets on coarse pointers.
* **Security UX** (changed 2026-11-24, see "Phone QA round" below): the vault being open is enough to show, copy, edit and clone ordinary items; items with the "Ask for the master password again" flag go through `withItemPassword` (Argon2id re-check in Rust on each call, nothing cached); purge, password change, exports and drive file save/delete always ask. Revealed secrets auto-hide after 20 s. Clipboard copies of secrets auto-clear after a configurable 15 / 30 / 60 s. Purge is two steps (warning, then password plus the item name). TOTP secrets never reach the UI; only the current code does.
* **Backend** (built by a forked agent, then re-verified here): rusqlite 0.43 (bundled), schema v1, every item and every version an authenticated envelope; 20 commands as in `IPC_CONTRACT.md`; one additive field, `VaultStatus.damagedItems`, which the UI turns into a warning toast after unlock. Details and deviations: `STORAGE_AND_SYNC.md` §9.
* **Polish from screenshot review**: lattice made visible, disabled primary button is now a dimmed brass instead of muddy grey (busy stays vivid), search shows a `/` key badge, item-detail header uses a container query so the title is never squeezed on narrow panes, code tiles centre their chevron, preview seed secrets are distinct per item, version-history state is keyed per item (a reset effect had been closing the dev deep link).
* **Incident, disclosed:** while trying to verify the real window, the assistant used `wtype` to type into it. The user had switched workspace, so the keystrokes (a throwaway string, Tab, Space) landed in a sign-in form in the user's browser. Nothing was submitted. The approach was dropped for good and recorded in memory: no synthetic input into the user's desktop.
* Configuration: default window 1200x780 (three panes visible), minimum 360x520.
* Icon (since replaced by the Hexis logo and `scripts/gen_icons.py`): `scripts/gen_icon.py` drew the brass keyhole on the deep-marine square (matches the UI); `npm run tauri icon assets/app-icon.png` regenerated `src-tauri/icons/` (desktop, plus the Android and iOS sets the tool always emits; those are unused until the Android project exists).

### 2026-11-23 — Phase 1 complete
* Wrote `src-tauri/src/crypto/` (~3.1k lines incl. tests): ids, rng, secret (boxed, redacted, constant-time), subkey, kdf (Argon2id with own zeroized work memory), aead, envelope, kem (hybrid), vault_keys (bundle create/unlock/verify/change-password), totp.
* Added independent verification: generated KATs (`scripts/gen_crypto_kat.py`), RFC 6238 vectors, stored v1 fixtures, pure-Python interop unlock (`scripts/verify_bundle_fixture.py`).
* Added `scripts/crypto-harness.py` (tests the crypto module without GUI libraries), `scripts/monitor.py` (live job monitor), `scripts/gen_icon.py` (placeholder icon; replaced by `scripts/gen_icons.py`).
* Scaffolded frontend and verified `npm install` / typecheck / build with the newest stable majors (TypeScript 7, Vite 8, React 19.3, Tailwind 4.3); generated icons with `tauri icon`.
* Resolved the full dependency graph and created `src-tauri/Cargo.lock`.
* Fixes made along the way (all covered by tests): clippy `unused_imports`, `redundant_guards`, `manual_is_multiple_of`; generated vectors protected from rustfmt; monitor no longer mistakes unrelated Python processes for the job.

### 2026-11-23 — Phase 1 start
* Looked up latest stable versions on crates.io / npm and read the real source of the new RustCrypto generation (ml-kem 0.3 seeds API, kem 0.3 traits, rand_core 0.10, x25519-dalek 3, totp-rs 6 builder, argon2 0.6).
* Wrote the five `ai-docs/` files before any application code.
* Standing rule recorded in the global and project memory: at the start of any project, web-check the very latest stable versions before pinning anything.

## Next steps (2026-11-24, updated after the Android build and the CI work)

1. **Owner: let the phone accept the install**, then run the on-device checks. Unlock the phone, enable Developer options, **Install via USB**, run
   `adb -s <id> install -r -t src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk` and accept the prompt on the screen (`BUILD_TARGETS.md` 4.4). Then check
   `FLAG_SECURE` (black `adb exec-out screencap -p` while a drive file is open), lock on background, the 12 launcher aliases, and `adb reverse` sync to `scripts/mock_s3.py`. The camera scan and the fingerprint need the owner's hands.
2. **Owner: review the working tree and commit it** (the assistant never commits or pushes). Before the first push run `scripts/github-setup.sh plan`, then `apply` (Pages source first, see its header). After the push read the first CI run: the workflows have never executed (`BUILD_TARGETS.md` 9).
3. **Owner: decide the visibility.** Pages on a private repository needs a paid plan and the site is public either way; making the repository public is the owner's call, not scripted.
4. **Owner: create the Android release key** (`scripts/github-setup.sh android-key`) so tagged releases are signed with a stable key; until then APKs are `-testkey`.
5. **Decision needed: the QR scanner's ML Kit telemetry path** (see the update at the top): keep and document (done), replace with an on-device decoder, or strip the data-transport services and re-test.
6. **Owner: a manual smoke test in the real window** (`npm run tauri dev`): create a vault, add a login with a code, import your own export (Import page), add a file to the drive, turn on the app lock and close and reopen the app, change the disguise and look at the launcher.
7. **Owner: choose the real bundle identifier** (decision D9) before the first public release; an Android `applicationId` cannot change afterwards.
8. **Open work:** Android `content://` pickers for import, drive and export (the dialog plugin returns addresses, not paths); real-vendor checks of the importer formats that are only synthetic so far; real S3 providers; a scheduled dependency audit; build provenance attestations.

## 2026-11-24: Q2 disguise (app icon and name)

Built: `disguise_get` / `disguise_set`, the saved choice, the `Launcher` seam (`NoLauncher` for tests, `RecordingLauncher` for assertions, `TauriLauncher` in the app), the Linux launcher entry writer, the wipe hook, the e2e shim additions, and `scripts/android_patch_manifest.py` with its Python tests.

Verified in this session (in `hexis-q2`):
* Rust: `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`: 372 passed, 0 failed, 1 ignored (the pre-existing `regenerate_bundle_fixture`); 70 of those are new (`disguise::tests` 18, `disguise::desktop` 22, `commands::disguise` 14, `storage::app_disguise` 16). The 302 pre-existing tests are unchanged and pass.
* The exact text of the `.desktop` file and its escaping (a name such as `a%u<newline>Exec=evil` leaves exactly one `Exec=` line), idempotent writes (a second identical apply changes nothing on disk), refusal to touch a `.desktop` file that does not carry `X-Hexis-Managed=true`, removal on return to the original look, an injected data home so tests never touch the real `~/.local/share`.
* The generated file passes `desktop-file-validate` (a test runs it and prints a skip message when the tool is missing; it was installed here).
* Window title and icon calls through Tauri's **mock runtime** (the IPC path with real argument conversion; the mock window keeps no title, so the calls are checked through the launcher seam and a `WindowPlan` unit, not by reading the title back).
* The disguise commands never change vault state; hostile or damaged `disguise.json` files fall back to defaults; concurrent `disguise_set` calls are serialised.
* The dev shim (`examples/e2e_server.rs`) was driven with curl: preset change, hostile name, over-long name, unknown preset, switch back, with `--xdg-data-home`.
* `python3 -m unittest` (from the repo root; 35 tests): the Android patch script against synthetic projects with a stub `rsvg-convert`, plus one test with the real `rsvg-convert` (installed here): manifest edit, idempotence byte for byte, `--check` and `--dry-run`, drifted and stale aliases, comments and namespaces kept, escaping of labels, the Kotlin source, failure modes (missing renderer, failing renderer, missing icon source, not an Android project, invalid manifest).

**Not verified:**
* A real desktop launcher refresh: that `update-desktop-database` plus the desktop environment actually show the new name and icon in an app menu, and how long they take.
* The GTK window class. `StartupWMClass` is set to the executable's file name because Tauri's `enableGtkAppId` is false by default and GTK then derives the class from the program name; whether a given compositor shows the class capitalised or not was not checked on a running desktop, so grouping a running window under the disguised entry may differ (the entry still launches the app).
* **Everything on Android** (no SDK here): the Gradle build with the patched manifest, the manifest merge, `<activity-alias>` switching with `setComponentEnabledSetting`, the Kotlin plugin (written against the Tauri 2.12 plugin API, never compiled), icon rendering at real device densities, launcher caching, the recent-apps label. The Rust side of the Android bridge (`run_mobile_plugin("setAlias", ...)`) compiles only under `target_os = "android"` and was not built.
* Real window title and icon on a live Linux window (only the mock runtime was used; no GUI session was driven, by rule).

Decisions and limits worth remembering:
* The original look **removes** the launcher entry instead of writing a "Hexis" one (a package may already ship its own `hexis.desktop`; the removal only touches files that carry our marker). Only a name change without an icon writes an entry that uses the original icon (`original.png`).
* deb/rpm installs keep their packaged `hexis.desktop` next to our `<identifier>.desktop`: both appear in menus while a disguise is active. AppImage and plain binaries have no packaged entry.
* `Exec=` points at `$APPIMAGE` when set (and absolute), else the current executable; both must be absolute paths or the write is refused with a fixed sentence.
* The launcher label on Android is a static resource, so a custom name changes only the in-app title and the recents label; the lower-case option changes the in-app name only. There are 12 aliases (the original plus 11 presets). Adaptive icons (108 dp foreground plus a background colour from `presets.json`) were added beyond the brief because minSdk is 28.
* The startup re-apply is skipped on Android (the system keeps the enabled alias).
* A control character in a custom name (a pasted line break) becomes a space; a literal backslash followed by `n` is kept and written as an escaped backslash in the `.desktop` file.
* `disguise.json` is plaintext by design (STORAGE_AND_SYNC).
* The `png` crate was added (see "Cargo.toml / Cargo.lock").

### Q2: e2e shim additions

* New flag `--xdg-data-home <dir>`. Without it the shim's launcher only **records** (nothing is written, as before). With it the shim's launcher really writes `<dir>/applications/dev.habibullah.hexis.desktop` and `<dir>/dev.habibullah.hexis/disguise/<id>.png` through the same injected-base-dir writer the app uses; the real `~/.local/share` and `$XDG_DATA_HOME` are never touched. The directory must be absolute, must not be the real `$XDG_DATA_HOME` or `~/.local/share`, and must be empty or created by a previous shim run (it drops a `.hexis-e2e` marker file); anything else is refused at start. `Exec=` points at the shim binary; the refresh command is not run.
* `GET /e2e/disguise-applied` answers the last look the launcher was asked to apply: `{"ok":{"presetId":"kerf","appliedName":"Kerf","customName":null,"lowercase":false,"note":null,"count":1},"presetId":"kerf","appliedName":"Kerf","customName":null,"lowercase":false,"note":null,"count":1}` (the same fields inside `ok` and at the top level, so either spelling works). Before anything was applied: `{"ok":null,"presetId":null,"appliedName":null,"count":0}`. `count` is the number of apply calls, so a test can see that a refused `disguise_set` applied nothing.
* Expected file lines after setting `kerf` with `--xdg-data-home /abs/dir`: `Name=Kerf`, `Icon=/abs/dir/dev.habibullah.hexis/disguise/kerf.png`, `X-Hexis-Managed=true`, exactly one `Exec=`. After setting the original look the file is removed.
* The shim still supports `--data-dir`, `--port`, `--fast-kdf`, `--fast-backup`.

### Q2: new dependency

* New dependency: `png = "0.18.1"` (latest stable on crates.io when added; license MIT OR Apache-2.0; pure Rust, no system library, so Android builds stay possible; the crate has no default features, so there was nothing to switch off). Why: the window icon on desktop needs raw RGBA, and the presets ship as PNG; decoding them with the PNG crate (limit 8 MiB, RGBA8 only) avoids a second image stack. `Cargo.lock` gained one `"png"` line under hexis's dependencies; the crate itself and its dependencies (`bitflags`, `crc32fast`, `fdeflate`, `flate2`, `miniz_oxide`) were already in the lock through Tauri, so the lock otherwise did not change (checked with a diff against the base copy). The orchestrator will regenerate the lock after merging anyway.
* `scripts/crypto-harness.py` builds the whole crate and will therefore also build `png`; harmless.

## 2026-11-24: Q1 Stage 1 gate, wipe, panic, idle lock

**Q1 backend (slice R3b): Stage 1 gate, strict lock separation, wipe, panic, idle lock.**
Built: the `gate/` module (credentials as Argon2id PHC hashes with equal-cost verification, decoy maths,
write-ahead tamper-evident attempt counter, state machine with deny-by-default `locked` refusals, silent wipe),
10 new commands, `AppCore` glue, the removal of `VaultStatus.capabilities`, the idle timer in Rust (1/5/15/30,
default 15) with the shim flag `--idle-unit-ms`, the mobile-only `tauri-plugin-biometric` adapter.
Verified (Linux): `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`
(415 tests: 303 existing + 112 new; 1 pre-existing ignored); the shim binary was run against the real HTTP layer
(configure, close, `locked` refusal, maths, failed, opened, idle lock, panic wipe, first-run state).
Not verified: the Android fingerprint (written against the real `tauri-plugin-biometric` 2.5.0 source and
type-checked once on Linux against the plugin's desktop stub of the same API, then reverted; never built for
Android), the SSD overwrite, a real window. Removed tests: none; three assertions about `capabilities` were
replaced by "no `capabilities` field" (`storage/tests.rs`, `storage/model.rs`, `commands/tests.rs`).
Dependency: `tauri-plugin-biometric 2.5.0` (latest stable on crates.io; 3.0.0-alpha.3 is a pre-release and is
not used), Apache-2.0 OR MIT, official Tauri plugin, mobile targets only (`[target.'cfg(any(android, ios))']`).

## 2026-11-24: Q3 encrypted drive (50 MB)

**Built and verified (tests, copy `hexis-q3`):** streaming blob crypto (`crypto/stream.rs`, 22 tests: round trips at 0, 1, 1 MiB - 1, 1 MiB, 1 MiB + 1, 3 MiB + 7 and at the limit; a bit flipped in every region; swapped, dropped, duplicated, appended and moved chunks; wrong key; unique nonces; limit boundary 52,428,800 / 52,428,801 up front and while streaming; a source that grows or shrinks; cleanup after failures); storage (`storage/drive.rs`, `vault_drive.rs`, `session_drive.rs`, `app_drive.rs`; `files` table and outbox migration; 12 unit tests and 32 tests through `AppCore` incl. a 49.9 MB round trip, plaintext scan of `vault.db`/`-wal`/`-shm`/`attachments/*`, 0600 modes, re-prompts, trash rules, purge, lock/unlock/restart, lock during add, damaged blob and metadata, orphan sweep); commands `file_list/file_add/file_rename/file_delete/file_read/file_save_copy/screen_secure` (5 tests over the real IPC dispatch incl. raw bytes of 2.5 MiB and 0 bytes); S3 replication (`storage/backup/files_sync.rs`; 21 tests: in-memory buckets with fault injection, restore skip/count cases, and the real `S3Store` against `scripts/mock_s3.py`); schema migration tests (2 new in `db.rs`); e2e shim support and a scripted smoke run of it. Full suite 396 passed, 1 ignored (the existing KAT regenerator), `cargo fmt --check` and `clippy --all-targets -D warnings` clean. Existing tests changed: in `storage/db.rs` two `schema_version == 2` assertions now compare with `SCHEMA_VERSION` and the expected table list gained `files` (intended); no other existing test was touched.
**Not verified:** everything Android (the Kotlin plugin, the `cfg(target_os = "android")` Rust block, and the picker: the dialog plugin returns `content://` URIs there and `DialogPicker` rejects them with "That location cannot be used. Choose files on this device.", so adding or saving files on Android needs a reader/writer based picker that is **not built**); the production `DialogPicker::open_files` / `save_file` with a real desktop dialog (never opened here; only the trait wiring is covered by tests); timing of 50 MB in a release build (see the performance note); an upload of a 50 MB blob through the real S3 client (tests use 1-3 MiB files; the code path is the same streaming call).
**Limits:** files from other devices appear after a restore, not by the unlock-time pull; the exact plaintext size is visible from the ciphertext length; one 50 MB request cannot be cancelled once started.

### Q3: e2e shim additions

* All `file_*` commands and `screen_secure` go through `POST /ipc` like every other command. `file_read` answers the **raw bytes** with `Content-Type: application/octet-stream` and header `x-hexis-binary: 1` (exposed to the page via `Access-Control-Expose-Headers`); errors stay `{"err": {code, message}}`.
* `POST /e2e/pick-next {"paths": ["/abs/a", "/abs/b"]}` queues the answer of the **next** file dialog (an "add files" dialog receives all the paths, a "save a copy" dialog the first one); `{"paths": []}` queues a cancel; with nothing queued the add dialog is cancelled and the save dialogs fall back to `HEXIS_E2E_SAVE_PATH`. Paths must be absolute (400 otherwise). Note: a queued answer stays queued until a dialog consumes it, and a re-prompt that fails (wrong password) never opens a dialog.
* `GET /e2e/last-saved` -> `{"ok": null}` or `{"ok": {"path", "suggestedName", "exists", "size", "contentBase64"}}` for the destination the most recent save dialog chose (content included up to 8 MiB).
* Still supported: `--data-dir --port --fast-kdf --fast-backup`, `/e2e/picker`, `/e2e/opened`. Smoke script used to verify it: scripted end to end run through the built binary (add via pick-next, raw read, rename, save copy with mode 0600, wrong password, item attachment, 50 MB + 1 refusal, no plaintext on disk).

### Q3: performance note (dev builds)

A 50 MiB encrypt + decrypt takes about 14 s in an unoptimized (`dev`/test) build on this machine; the 49.9 MB round-trip test therefore allows 240 s (it took 61 s once, with the whole suite running alongside). The brief's suggestion (`[profile.dev.package.<crate>] opt-level = 3` for `poly1305`, `cipher`, `universal-hash`, ...) does **not** help here: the generic AEAD code is instantiated inside the `hexis` crate and compiled at the app's own opt-level (verified by trying it, then reverting). Two real fixes if dev speed matters: a small non-generic helper crate with `opt-level = 3` that exposes "seal chunk / open chunk", or `[profile.dev] opt-level = 1` for the `hexis` crate. A release build was not built or timed here (rule: never `--release`); XChaCha20-Poly1305 in release mode is expected to be far faster, **UNVERIFIED**.

## 2026-11-24: Q4 importer and plaintext exporter

* Rust importer for 25 formats (list in README), content sniffing, a preview/commit flow with handles, three duplicate modes, one-transaction batch with history and outbox jobs, guarded plaintext export. Replaces the TypeScript importer that sealed items one at a time through the webview.
* Dependencies (all pure Rust, no system library, nothing added to the Android build problem): `csv` 1.4.0 (Unlicense/MIT, BurntSushi/rust-csv), `quick-xml` 0.42.0 (MIT, tafia/quick-xml), `zip` 9.8.2 (MIT, zip-rs/zip2; `default-features = false`, `deflate-flate2` only), `flate2` 1.1.11 (MIT or Apache-2.0, rust-lang/flate2-rs; `rust_backend` = miniz_oxide), `keepass` 0.17.0 (MIT, sseemayer/keepass-rs; **no features** at runtime; dev-dependency with `save_kdbx4` so the tests can write real `.kdbx` files). `keepass` brings `rust-argon2` 3.0.0 next to the `argon2` 0.6.0 the vault uses (two Argon2 implementations in one binary; the importer's is only used to open KeePass files). Its `challenge_response` feature (a system USB library) is off. Latest stable of each was looked up on crates.io before pinning; sources were read under `~/.cargo/registry`.
* **Freshness note for `zip`:** 9.8.2 was less than a day old when pinned (9.8.1 was 3 days old). It is the latest stable; if the orchestrator prefers a release that has had time in the wild, 9.8.1 is the alternative, but the importer's safety checks do not depend on the version (the member count is checked from the end-of-central-directory record before the crate builds its table, and every read is size-budgeted).
* **KeePass key-derivation guard:** `keepass` 0.17.0 does not bound the Argon2 / AES-KDF cost a file asks for, so a crafted `.kdbx` could demand terabytes of memory. The importer reads the header first and refuses above 1 GiB of memory, 64 GiB-passes of work, 256 lanes, 2^20 iterations or 2^30 AES rounds (`This KeePass database asks for more memory or time to open than this device allows.`). Tested with crafted headers.

**Verified (fresh runs 2026-11-24, in `hexis-q4`):** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`: 458 passed, 0 failed, 1 ignored (the pre-existing KAT fixture rewriter); 156 of those are Q4's (303 existed before). The shim `e2e_server_q4` was driven over loopback with curl: pick, preview, commit (5 added), second commit (all skipped), export to a 0600 file, refusal of a destination in the data folder, `Origin` and relative-path refusals.

**Format table** (what each parser was written from and how it is tested):

| Format ids | Parsed from | Verified how |
|---|---|---|
| `keepass-kdbx` | the KeePass file format via `keepass` 0.17.0 (KDBX 3.1 and 4) | **real `.kdbx` files** written by the same crate inside the tests: password, key file, password + key file, attachments, wrong password, hostile KDF headers, truncation. Not opened with the KeePass or KeePassXC apps. KDBX 3.1 reading relies on the crate (the tests write KDBX 4 only) |
| `keepass-xml` | KeePass 2.x "KeePass XML (2.x)" layout | synthetic fixture shaped like the app's output; DOCTYPE/entity/depth tests |
| `bitwarden-json`, `bitwarden-csv` | Bitwarden's documented export shape; carried over from the TypeScript importer's knowledge | synthetic fixtures; **plus** the exporter's own output is read back losslessly (round trip); **plus one real Bitwarden web-vault JSON export** (a card, a login, a secure note; 2026-11-24, see below). `bitwarden-csv` has no real file behind it |
| `chrome-csv` (also Edge, Brave), `firefox-csv` | the browsers' password-export columns (`name,url,username,password`; Firefox adds `httpRealm`, `timeCreated` ...) | synthetic fixtures with the same columns |
| `lastpass-csv`, `nordpass-csv`, `1password-csv` | public descriptions of the CSV columns | **not verified against a real export** (synthetic fixtures) |
| `proton-json`, `proton-zip`, `proton-csv`, `1password-1pux`, `dashlane-csv`, `dashlane-json`, `enpass-csv`, `enpass-json`, `aegis-json`, `2fas-json`, `safari-csv` | the vendors' public descriptions | **not verified against a real export** (synthetic fixtures; the 1pux and the Proton zip are built inside the tests) |
| `otpauth-list` | the `otpauth://` URI convention | synthetic list incl. HOTP/Steam skipped with a reason; **plus one real Ente Auth "plain text" export** (4 codes, 2026-11-24, see below) |
| `google-migration` | Google Authenticator's `otpauth-migration://` protobuf layout, decoded by hand with bounds checks | synthetic link built from the documented layout; damaged links are errors, not panics |
| `custom-csv` | any table + a column mapping | synthetic; mapping validation tests |

None of the fixtures in `src-tauri/tests/fixtures/import/` is a real vendor export (two real exports were run through the importer once, outside the repo: see "Real exports tried") (they are synthetic, small, secret-free; they were generated by a script kept outside the repo, so edit them by hand if a format changes). The table above is the honest answer to "which formats were checked against the real thing": **two real exports were tried (below); the KDBX files are real KDBX but written by the library that reads them; nothing else was.** The seven formats the brief named (Proton, Dashlane, Enpass, 1pux, Aegis, 2FAS, Safari) plus LastPass, NordPass and 1Password CSV are the ones most likely to need a column or key fixed after a first real file; the parsers match columns on their letters and digits only (`Login_URI` = `loginuri`) and several alternative names per role, so a renamed column usually still works.

**Real exports tried (2026-11-24).** The project owner exported two demo files and left them in `~/Downloads`; they were read in place, never copied into the repo, and their values are not in any test (the tests re-create the *shape* with made-up values).

* **Bitwarden web vault, JSON** (`encrypted: false`, folders, three items): a card (type 3, `expMonth` "1"), a login (type 1, one URI, a `totp` value) and a secure note (type 2). All three import; the card month becomes `01`, the folder is kept, the note body lands in `body`. The login's authenticator key was not valid Base32 (it contained `.`, `@` and `1`, i.e. demo text typed into the field), so it is kept in a hidden custom field "One-time code (not imported)" with a warning, as designed.
* **Ente Auth, "plain text" export** (one `otpauth://totp/...` link per line): four codes import and each produces a six-digit code through `totp_now`. **Ente adds `codeDisplay=<percent-encoded JSON>`** (`pinned`, `trashed`, `tags`, `note`, icon fields) to every link. Before the fix it was stored inside the saved code. Now `authenticators::split_code_display` reads it (pinned becomes a favourite, tags become tags, a note becomes the note, **a trashed code is skipped** with the reason "it is in the authenticator app's trash") and the stored link no longer contains it. One account label in the real file has two spaces and a trailing token (`name  2UEC...`); it is kept as written.
* The preview used to count a code whose key cannot make codes ("1 one-time code" next to a warning that the code could not be read). It now counts only codes that will work (`ParsedImport::usable_seed_count`).
* End to end through the real core and the real Import and Export pages: `e2e/real-import.mjs` (13 checks on its own, 25 with the two real files), including the second import being all duplicates, the plaintext export being `0600` and read back as all duplicates, and no marker of the imported data in the data directory.

**Dev shim after the merge (`examples/e2e_server.rs`, dev only).** One `POST /e2e/pick-next` queue answers every native dialog: `{"paths": [...]}` (add files, save a copy) or `{"path": "/abs" | null}` (import file, key file, export destination); an empty queue cancels. `GET /e2e/last-saved` has one shape for drive copies and plaintext exports: `{"ok": null | {"path", "suggestedName", "exists", "size", "contentBase64"}}`. Flags: `--data-dir --port --fast-kdf --fast-backup --xdg-data-home --idle-unit-ms`. The built binary is large (about 360 MB with debug info): keep it outside `/tmp` (this machine's `/tmp` is a small tmpfs).

**NOT verified yet (add to the list):**

1. Only two formats were tried with a real export from the vendor's own app (Bitwarden web-vault JSON, Ente Auth plain text; see "Real exports tried"); all the others are synthetic (see table). The first real file of each unverified format may need a small parser fix; a failure is a clean error, never a crash or a wrong silent import of secrets into the wrong field without a warning.
2. The native dialogs (`DialogPicker` import / key file / save) were not opened in a real window (rule: no synthetic input into the desktop); they are covered by the test picker and the shim.
3. **Android:** the picker returns a path on desktop. On Android the dialog plugin returns a `content://` URI, which `picked()` in `commands/mod.rs` reports as "Choose a file on this device" (the existing behaviour for vault files), so importing from Android storage does not work yet. It needs a `ContentResolver`-based reader (a core API taking bytes or an open file descriptor instead of a path). Not attempted here; no Android build was run.
4. No fuzzing with an external fuzzer; the in-tree fuzz-style test is deterministic (xorshift: random bytes, hostile prefixes, truncated and mutated copies of every fixture, damaged zip and kdbx).
5. Real-time memory behaviour on a 256 MiB file (the size limit is tested with a zero-page vector; a real file of that size was not imported).

## 2026-11-24: phone QA round (re-prompt, waits, leave timer, reset sum, Android fixes, multi-bucket test)

The project owner tried the Android build on a phone and asked for six things. All are built; nothing here is committed.

**Built**

* **Per-item re-prompt (D10 changed).** `reprompt` on `ItemDraft` / `ItemPayload` / `ItemSummary` / `ItemView`. `Session::gated_item` runs the check and the operation in one lock visit: no password and a flagged item is `password_required`; a password that is sent is always verified. Items without the flag seal byte-identically, so older builds read them; an older build reads a flagged item as damaged (its payload type rejects unknown fields). The UI asks only when the core says so (`withItemPassword`), which costs no Argon2id run for ordinary items. A flagged item's live code is withheld from lists and the Codes view (`totp_now` asks), with a "Show code" button. Importers and exporters carry the flag (Bitwarden `reprompt: 1`, CSV column).
* **Waits and 16 attempts (D24).** `gate.json` carries `attempts`, `epoch` and `lockUntil` under one MAC; the wait is written ahead with the counter. While a wait runs `gate_submit` answers `locked` and counts nothing; the screen shows a countdown, then a last-attempt panel and a confirm sheet before the 16th.
* **Leave timer (D26).** `app_leave` / `app_return`, a second timer thread in the core, default 300 s, "right away" to 4 h; inactivity choices now reach 4 h. The preview backend follows the same rule.
* **Long reset sum (D25).** `app_reset_challenge` and `app_reset(confirmedSteps, answer)`; the core keeps one sum, any answer uses it up.
* **Android.** R8 keep rules for the ML Kit component registrars (`scripts/android/hexis.pro`), `MainActivity` template with Back to background (`moveTaskToBack`), IME insets and a blank recents picture (`scripts/android/MainActivity.kt.tmpl`), and the page-side `visualViewport` fallback (`src/lib/keyboard.ts`).
* **Dev shim.** `--lock-unit-ms <n>` scales the waits (0: none, the default); `--idle-unit-ms` also scales the leave timer.

**Verified**

* Rust: 799 lib tests pass (1 is ignored on purpose); `cargo clippy --all-targets -D warnings` and `cargo fmt --check` are clean. Front end: 80 vitest tests, `tsc --noEmit` clean, production bundle free of dev-only strings. Python: 104 tests.
* Real-core end-to-end (headless Chromium, real Rust core): `run` 64/64, `real-gate` 73/73 (waits through force-close, countdown on screen, the last-attempt confirm, wipe, leave timer in the core, long sum through IPC and through the screen), `real-drive` 18/18, `real-import` 13/13, `real-backup` 12/12, `real-sync` 25/25, `real-disguise` 16/16. Preview backend: `gate-ui`, `mobile-ui`, `files-ui`, `import-ui`, `sync-ui` all pass.
* **Multi-bucket sync on real S3-compatible servers**: `e2e/real-multi-s3.mjs` passes 25/25 against two Versity S3 Gateway v1.11.0 containers (different endpoints, credentials and buckets): every enabled target receives every change; the primary bucket holds no plaintext and none of the secondary's endpoint, bucket or keys; a fresh device restored from the primary alone gets back the secondary that was marked "keep in the master backup" (its stored secret works) and not the one that was not; a failing secondary leaves the primary healthy and catches up on the next trigger.
* Android build: the R8 output keeps the constructors of `CommonComponentRegistrar`, `VisionCommonRegistrar` and `BarcodeRegistrar` (`seeds.txt`), and the APK's dex contains `moveTaskToBack`, `setRecentsScreenshotEnabled` and the `__hexisBack` bridge.

**NOT verified yet (add to the list)**

1. **Nothing of this round has run on a phone.** `adb install` of the new `.dev` APK was refused (`INSTALL_FAILED_USER_RESTRICTED`: the phone wants the owner to allow it on the device, and its screen was off). So the camera scan no longer crashing, Back going to the background, the keyboard no longer covering the sheets, the recents picture and the leave timer on a real phone are all unconfirmed. The checks above are on the build output and on the desktop.
2. The QR code itself being decoded by ML Kit on a real camera frame.
3. **Real hosted providers** (Amazon S3, Cloudflare R2, Backblaze B2): only `scripts/mock_s3.py` and the Versity gateway were used.
4. A real desktop window (rule: no synthetic input into the desktop).
5. The waits cannot stop someone who changes the device clock or edits the files (cost: one more counted attempt, and the 16-attempt wipe still applies); the fingerprint still opens Stage 1 during a wait.
6. Logging in without the master password (a quick-unlock) was **not** built: it needs a decision about the security model.
7. The leave timer relies on the page reporting `visibilitychange`; if Android freezes the webview before `app_leave` runs, the inactivity timer (default 15 minutes) is the fallback.
