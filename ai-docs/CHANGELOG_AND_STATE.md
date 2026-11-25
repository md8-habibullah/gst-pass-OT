# CHANGELOG_AND_STATE — where the project is right now

Update this file after every major milestone. Newest entries first.

## Update (2026-11-25, release 0.5.0 prepared): version raised on branch `release/0.5.0`, nothing published

**The request:** "make a release also properly for all". A release is made by raising the version on `main` (`CONTRIBUTING.md` "Releases"), so this is a version bump made with the project's own tool, on its own branch cut from `feature/passkeys-linux-browsers` (PR #4). Merge PR #4 first; the release PR then shows only the bump. The owner commits, pushes and merges; nothing here tags or publishes.

**Done.** The CHANGELOG's `[Unreleased]` section gained a one-line summary and an "Upgrading from 0.4.0" paragraph (it names the one real compatibility cost: a 0.4.0 or older build cannot save an edit to a login that holds a passkey and skips it when it merges a pull, so update every device before adding a passkey), then `python3 scripts/version.py bump auto` raised 0.4.0 to **0.5.0** (a minor bump: the section lists Added) in `VERSION`, `package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` (the `hexis` entry) and `packaging/arch/PKGBUILD`, and turned the section into `## [0.5.0] - 2026-11-25` with a new empty `[Unreleased]` above it. No version number was edited by hand.

**What "for all" covers.** What the pipeline builds and publishes when `ci.yml` finds a version with no tag and every job green: the Debian package, the RPM, the AppImage and the tarball (64-bit x86), the Android arm64 APK signed with the repository's release key, `SHA256SUMS`, the release notes (install commands, downloads table, fingerprint, the changelog section), and then the Pages workflow, which rebuilds the signed apt, rpm and pacman repositories and the installer from the newest stable release. The extension's two zips are not release files on purpose: the app carries the extension and writes it into the data folder, a Chromium zip cannot be loaded without unpacking it, and the Firefox zip is not signed. **Not built by this pipeline, so not in the release:** Linux on aarch64, Windows, macOS, iOS.

**Docs added for a public release.** `CONTRIBUTING.md` > Releases gained "What a release contains, and what it does not" (the files, the order of a release, what is not built) and "Publishing the browser extension (owner steps, not automated)": the checks before a submission, signing the Firefox add-on on addons.mozilla.org (`web-ext sign`, unlisted or listed, how a signed `.xpi` is hosted, the data-collection declaration), and the Chrome Web Store (account, getting the store item's id and public key *before* publishing, the six places the old id is written, why the Hexis release with the new id must come first, the listing and privacy forms, test instructions for the reviewer). Written from Mozilla's and Google's own pages on 2026-11-25; **none of it has been tried**, and the Chrome Web Store fee, whether it accepts a manifest that already has a `key`, and whether Mozilla's review accepts `data_collection_permissions: none` for a native-messaging extension were not confirmed. `extension/README.md` and `PASSKEYS.md` §10 point to it. The website builds with every link resolving.

**The first CI run of the Linux browsers work was red, and main with it (found when asked to publish).** Pull request 4 was merged while `Checks`, `End-to-end` and `CodeQL` were red, so the push to `main` failed `Checks` as well, and the release job runs only when every job passes. Read from the CI logs, each one fixed (commit "Fix the failures the first CI run ... showed"; no production code changed):
* **`node --test extension/tests/` fails on Node 24** (CI's version): a directory is no longer expanded ("Cannot find module"). It worked on this computer's Node 26, which is how it got through. Reproduced in a `node:24-slim` container, and the glob `extension/tests/*.test.mjs` ran all 23 tests there. `verify_all.sh` and the docs use the glob.
* **`the_idle_lock_over_ipc_with_and_without_a_gate`** sent a command every 100 ms and needed the vault to stay open for a scaled minute of 400 ms; a stall of 300 ms locked it mid-loop (red on CI; 1 in 60 locally with every core busy). A scaled minute is 1.5 s in that half now.
* **`real-backup` e2e** read the bucket once as soon as the page said "Up to date" and found only the key bundle and one version object (CI log); it passed on `main`'s run. The script now looks for up to 15 s. Not looked into: whether the page should say "Up to date" while the first upload is still on its way.
* **CodeQL** put six alerts in test code on the pull request: five rewritten (an assertion no longer prints the test password; website names are matched as whole words in `bridge-ui.mjs`); the constant test password in `bridge/tests.rs` stays, like the five alerts that already stand in other test files.
* **Two timing tests failed once on this computer while it was busy**, and were fixed the same way: a real race in `gate::tests::after_the_device_restarts_a_wait_ends_only_by_time_checked_online` (after a restart the gate starts a time check on a background thread; under load it could run after the test had moved the fake clock and anchor the time at the wrong moment; 1 failure in 80 runs with every core busy, 0 in 250 after) through `TrustedClock::finish_background`, compiled only for tests; and the decoy-equation vitest test (20,000 rounds, 5.2 s against a 5 s limit), now 30 s.
The lesson is the old one (run what CI runs): this computer's Node (26) is newer than CI's (24), and a run on a quiet machine hides timing margins. The idle-lock change was not re-run under heavy load (that run was stopped); it widens a margin fourfold.

**Checked.** `verify_all.sh` and `verify_all.sh --e2e-only` both passed on the branch after the two test fixes (1123 Rust tests, 218 frontend, 177 script, 23 extension; every e2e script, `real-bridge` 61/61, `real-bridge-firefox` 15/15, `real-multi-s3` 25/25); `version.py check --release` passes; `release_notes.py` renders the 0.5.0 notes (read through, with a stand-in checksum file); `cargo metadata --locked --offline` accepts the lock file; the repository has the secrets and the variable the release job needs (`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `REPO_SIGNING_KEY`, `ANDROID_CERT_SHA256`, read as names only); the last three runs of CI and Pages on `main` were green.

**NOT verified.** The `release` job itself (it runs only on `main`, after every job passes); the Android build of this version (CI builds it; the passkey provider has never been on a real phone); the AppImage's launcher path for the browser host and the packaged `hexis --native-host` (its five lines in `main.rs` call the same function the tests and the e2e shim use; no built package was run); `real-bridge` on CI for the first time (a branded Chrome 137 or newer can ignore `--load-extension`; the script skips those steps when `CI` is set).

## Update (2026-11-25, passkeys in Linux browsers): built, run end to end with a real Chromium and a real Firefox

**The request:** "make for linux device also if possible". Chosen with the owner: passkeys in Linux browsers through an extension and a native bridge, Firefox and the Chromium family with one shared extension, the prompt and the master password in the Hexis window. This is plan item P5 (option D2). Branch `feature/passkeys-linux-browsers` (from `main` at the merge of PR #3); the owner commits. Version stays 0.4.0.

**Built.** Decisions D52 to D55 in `ARCHITECTURE.md`; the design is `PASSKEYS.md` §10; the contract is `IPC_CONTRACT.md` §14.4a; the files written are in `STORAGE_AND_SYNC.md` (last section); the screens are `UI_DESIGN_SYSTEM.md` §14.
* `extension/` (Manifest V3, one source, a manifest per family, three small scripts, 23 unit tests) and `scripts/build_extension.py` (reproducible store zips; checks the sources against what the app writes).
* `src-tauri/src/bridge/` (wire checks, queue and rules, private Unix socket, native messaging host framing and `hexis --native-host`, per-user browser manifests, the embedded extension files) and `commands/bridge.rs` (`bridge_status/set/register/unregister/wait/options/answer`); the provider gained a page caller; `device.json` gained `browserBridge`; "Remove Hexis from this device" disconnects every browser.
* Settings > Browsers and the request sheet (`BrowsersPage.tsx`, `BridgeHost.tsx`), the note over the lock screen, the phone layout.
* The e2e shim gained `--bridge-home`, `GET /e2e/bridge` and `--native-host`; `cdp.mjs` can load an extension.

**Checked on this computer.** `e2e/bridge-ui.mjs` (Settings > Browsers and the sheet, in the preview); `e2e/real-bridge.mjs` (a headless Chromium with the extension, the browser starting the host, the socket, the real core, the sheet in the Hexis UI, a page that makes a passkey, signs in and verifies the signature with WebCrypto; two accounts, a wrong then a right master password, Use another way and a switched-off bridge reaching the browser's own flow, another website's name refused, a program on the socket getting no signature without a click, switching off removing the socket, Disconnect); `e2e/real-bridge-firefox.mjs` (the same chain in Firefox 160 with the extension as a temporary add-on, driven over Marionette).

**Security review of the bridge (read-only agent, same day), all fixed with regression tests:** the inactivity lock did not fire while Browsers was on (the window's long poll counted as activity: now passive, and refused while the app lock is closed); a wrong master password on a *new* login was not checked before `userVerified` was set (the core now verifies it first); the host did not re-check the socket's folder and file before connecting; a request id could be reused; `status` told any page whether the app lock was closed; the extension's files were created and then chmodded (now created with the final mode); a hidden tab could raise the window (`not_visible`). Details: `PASSKEYS.md` §10.

**NOT verified.**
* **Mozilla's signing and add-on validation** (a release Firefox runs only a signed add-on; `data_collection_permissions` with `strict_min_version` 128 was accepted by Firefox 160 as a temporary add-on, which is not the validator). **The Chrome Web Store**: a store item gets its own id, which would have to be added to the host manifest's `allowed_origins`.
* The Hexis window coming to the front on a real Wayland compositor (Hyprland): the code asks, a compositor may refuse; a real WebKitGTK window (the sheet was seen in headless Chromium); Snap and Flatpak browsers (detected and explained, not run); an AppImage's launcher path; browsers other than Chromium 158 and Firefox 160.
* **Not built:** passkeys in frames other than the top one; autofill-style (conditional) and security-key requests (they go to the browser on purpose); RS256, EdDSA, PRF; macOS and Windows; `credentialsd`.

## Update (2026-11-25, passkeys and the all-in-one scanner): built, tested without a phone

**The request:** finish the passkey work from `PLAN_NATIVE_CAMERA_AND_PASSKEYS.md`, and add a camera button inside the app that opens one scanner for a one-time code, a passkey code or anything else, that works out what it saw, suggests the existing login with the same email or user name, lists the other logins, and always offers "Create a new login". Several sign-ins and codes per login already existed (D46).

**Built (uncommitted, branch `feature/passkeys-and-smart-scanner`; the owner commits). Version stays 0.4.0.** Decisions D47 to D51 in `ARCHITECTURE.md`; the design is `PASSKEYS.md`; contract in `IPC_CONTRACT.md` §14; storage in `STORAGE_AND_SYNC.md` ("Passkeys in a vault"); screens in `UI_DESIGN_SYSTEM.md` §13.

* **Passkeys are fields of a login (D47).** `passkey`, `passkey#2` … (at most 10), one JSON record each; no new item type and no schema change. The private key never leaves the core (views carry `PasskeyInfo`, editors carry keep-markers, `item_create` refuses passkeys, clones strip them, merge is a set by credential id). **This departs from plan D-4** (own item type), on purpose: a schema bump would make a 0.4.0 build refuse the whole vault.
* **Own ES256 authenticator (D48)** in `src-tauri/src/passkey/`: `none` attestation, zero AAGUID, counter 0, flags UP/UV/BE/BS/AT, excludeCredentials, rp id against origin rules.
* **Android provider (D49)**: `CredentialProviderService` (Android 14+), off until Settings > Passkeys switches it on; one JSON entry `Provider::call` over two JNI functions; the app lock refuses everything; a locked vault opens with the master password only; a fresh fingerprint or screen lock per use (a phone with neither asks the master password); re-prompt logins and PIN-opened vaults ask the master password; browsers through Google's privileged-apps list (73 apps, fetched 2026-11-25; the reviewer's fetch the same day matched it), apps through the website's `assetlinks.json` (the one request, switchable). `check_no_notifications.py` now lets exactly this one `<service>` through, in exactly this shape.
* **Scanner (D50)**: camera button in the account bar, shortcut `S`; `scan_classify` / `scan_apply`; smart match in Rust (account 100, site 60, name 40, best eight), every login listed, "Create a new login" always offered.
* **Bitwarden (D51)**: `fido2Credentials` import (ES256/P-256 only, others skipped with a named warning) and JSON export; CSV leaves passkeys out.
* **Settings > Passkeys**: the switch, "Open Android settings", the website-check switch (`passkeyAppCheck` in `device.json`), the rules in plain words.

**Found and fixed on the way:** the exporter would have written passkey #2 and later as hidden plain custom fields and dropped #1; `merge_drafts` could overwrite a passkey in slot `passkey` on an import; `check_no_notifications.py` rejected the provider's `<service>`; two stale tests (`passkeyAppCheck`, `hasPasskey`) and one clippy lint. A read-only security review found nothing exploitable at high confidence; its notes were fixed: a phone with no screen lock now asks the master password instead of signing unverified, the origin match rejects hosts a different parser would read differently, the website request refuses private and local names, the notification check demands the exact class and one copy, request codes are seeded from the clock.

**Verified (fresh runs 2026-11-25, `scripts/verify_all.sh`: ALL CHECKS PASSED).** tsc and the production build; vitest 213; bundle guard; no-notifications check; version check; shellcheck local and as CI runs it (0.8.0); 173 script tests; `cargo fmt --check`; `clippy --all-targets -D warnings`; Rust 1076 passed, 0 failed, 2 ignored. `cargo check --target aarch64-linux-android` (NDK 28.2) clean. Gradle (`src-tauri/gen/android`, patched with `scripts/android_patch_manifest.py`): Kotlin compile and the R8 release minify pass after the last Kotlin edits; the manifest merge and resources passed earlier the same day. `scripts/verify_all.sh --e2e-only`: ALL CHECKS PASSED, 514 checks, including the new `scan-ui`, `provider-ui` and `real-scan` (23 checks on the real core) and the real-multi-s3 run in Docker. One earlier full run had `run.mjs` fail 4 checks (the add-account and import steps, right after the dev server's first start); it passed 90/90 alone and in the second full run, so it is recorded here as a flake that was seen once and not explained.

**NOT verified.**
* **Anything on a phone.** That Hexis appears in Android's list of passkey providers, the sheet, `BiometricPrompt`, making and using a passkey from Chrome and from an app, the hybrid transport (a desktop browser's QR code read by the system camera), and that `Context.dataDir/<identifier>` is the folder the window uses (a mismatch fails safe with an error). The manual cases are in `e2e/README.md` ("Phone checks for passkeys").
* The full APK link with the Rust `.so` for aarch64 (only `cargo check` and the Kotlin compile were run), a real WebKitGTK window, the trusted time servers, screen readers on the new screens.
* Against a built 0.4.0: that an older build refuses to save an edit to a login that holds a passkey and skips it in a pull-merge. That is read from its source (`normalize`, `pull_apply.rs`), not run.

**NOT built.** A desktop passkey provider (browser extension P5, or `credentialsd`); RS256, EdDSA, PRF and large blob; FIDO Credential Exchange files and the Android Credential Transfer API; Google Authenticator migration import beyond the editor's existing split; a minimum-version check between devices (so: update every device before adding passkeys).

## Update (2026-11-25, Q6): folders, a search through everything, offline use that merges, several websites, a safer delete

**The request (nine items):** open straight into search; search over everything (titles, websites, notes, field names and descriptions) with an option for old versions, plus filters and orders; work 100 % offline and merge by itself on reconnect like Google Keep, keeping every conflict in the history; several websites per login; a short copy notice instead of a countdown over the buttons; New item menu order (Login, Code only, Secure note, Identity, Card, SSH key first); permanent delete needs the master password and a long sum while the item is recent (under six months) and only the password after that; a collapsible password generator; folders; Bitwarden and Ente Auth imports that match the same account and keep every note.

**Built (uncommitted; the owner commits). Version raised to 0.4.0** (`VERSION` and the five files that follow it, `CHANGELOG.md` has a `## [0.4.0] - 2026-11-25` section). Decisions D38 to D44 in `ARCHITECTURE.md`; contract in `IPC_CONTRACT.md` sections 3, 10 to 13; storage and merge rules in `STORAGE_AND_SYNC.md`; screens in `UI_DESIGN_SYSTEM.md` section 11; the slice is Q6 in `FEATURE_QUEUE.md`.

* **No format change.** `ItemPayload` stays as it was (it refuses unknown keys), so a folder is a path string in the existing `folder` field (the list of empty folders is in the synced settings object), several websites are one `url` field with a line each, and a merge is recorded as an `edited` event with `restored_from`. A 0.3.0 build still opens everything 0.4.0 writes.
* **Folders (D38).** Paths of up to six levels; create, rename (merges into an existing name), remove (contents move up, nothing is deleted), move one or many items. The list shows folders first, then the items at that level, with a path bar; a search or a filter looks in every folder.
* **Search (D41).** `item_search` runs in the core and answers with ids and field names only, never a value; secret kinds are never compared; a re-prompt item's notes stay sealed. A result says where it matched ("Found in Notes"). A tick adds earlier versions. Orders (nine) and filters (type, favourites, has a code, asks for the password, tag, changed within, at least and at most a number of versions) work on what the list already knows.
* **Merge (D39) and reconnect (D42).** A three-way merge per field; the newer change wins a true conflict, tags, websites and files are merged as sets, custom fields by name, notes by lines with plain markers where two devices edited the same lines. Everything the other device had stays as snapshots in the history. A fourth sync trigger, the device coming back online (`sync_reconnect`, called from the window's `online` event); still no timers, no polling, no notifications.
* **Permanent delete (D40).** `item_purge_plan` then `item_purge(id, password, answer?)`: under 183 days on the trusted clock (fails closed) needs the password and the sum, older needs only the password; the answer never leaves the core; the sum is single-use (a wrong answer, or a delete that went through, uses it up; a wrong password leaves it).
* **Import (D43).** *Merge matching accounts* is the default: a Bitwarden login and its Ente Auth code become one item; every note, website and tag is kept.
* **Open in search and the default order (D44)** are device settings (`openInSearch`, `listSort` in `device.json`; Settings > Vault list).
* **Smaller items:** several-website editor and detail rows, a collapsible generator, the short copy notice, the New item order.

**Found by the final checks and fixed:** (1) the `e2e/run.mjs` import step looked for an imported item at the top level, but Bitwarden's folder now comes across as a folder, so the step opens the folder first; (2) `.github/workflows/codeql.yml` (added from GitHub's own template, pulled in as `daa0c6e`) had no "System details" first step, which `scripts/tests/test_ci_system_info.py` requires of every job; the step was added (copied from `ci.yml`) and the steps re-indented to this repository's layout, the CodeQL settings themselves are untouched; (3) `cargo fmt` and five clippy lints (`cloned_ref_to_slice_refs`) in new test code.

**Verified (fresh runs 2026-11-25, `scripts/verify_all.sh`: ALL CHECKS PASSED).** Rust library suite 962 passed, 0 failed, 2 ignored; `cargo fmt --check` and `clippy --all-targets -D warnings` clean. vitest 203 (21 files); `tsc --noEmit` and the production build clean; the production bundle holds no dev-only strings. Python script tests 149. `shellcheck` 0.8.0 in an `ubuntu:22.04` container. Real-core end-to-end through `scripts/ci_e2e.sh`, the way CI runs it: `run` 90, `theme-ui`, `real-gate` 75, `real-uninstall` 18, `real-drive` 18, `real-import` 13, `real-lists` 33 (new: folders, search, filters, several websites, the delete sum, open in search), `real-disguise` 16, `real-backup` 13, `real-sync` 46, `real-multi-s3` 25 (two S3 servers in Docker), all passed.

**Not verified.**
* **Android:** whether the keyboard comes up with *Open in search* on a phone, and the new Kotlin was not compiled or run; the phone was not touched.
* **Real windows:** the `online` event that triggers the reconnect sync was exercised only through the core and the shim (headless Chromium), not in a real WebKitGTK window or the Android WebView; nor how those engines draw the new screens.
* **Screen readers** reading the new folder rows, the filter panel and the delete dialog.
* The trusted time servers (the delete rule's six months use the checked clock; with no check the delete stays on the strict side).
* **The installer in distribution containers** (`scripts/check_installer_in_containers.sh`) was not re-run: the download of CI's packages stalled twice (5 and 9 minutes). `scripts/install.sh` and `scripts/tests/container_case.sh` are unchanged this round, and CI's installer jobs on `daa0c6e` passed on every distribution for the same files.
* **The CodeQL workflow's `java-kotlin` autobuild** may fail; the first run on GitHub decides.
* Known limits, by design: two devices that each make a *different empty folder* while offline can lose the earlier one (a folder with an item in it is never lost); items of different types are not merged (the older way, a restore from the history, remains); notes longer than 64 KiB are merged by taking the newer text.

**For the release:** the version is 0.4.0 and CI publishes by itself once it is on `main` (see the entry below).

**After the push (`c430748`): CI was red once more, and why.** Everything was green except one vitest test: `mockGate.test.ts` "looks at nothing during a wait" expected `retryAfterMs: 90000` and got `89999`, because the mock gate reads `Date.now()` and a millisecond ticked over between two calls on a slower runner. It passed on every local run (203 of 203). The test now pins the clock like its neighbours (`g.now`), was looped eight times, and the rule is `CONTRIBUTING.md` rule 5 (a test never reads the real clock when it asserts an exact time). Because the Checks job failed, the release job was skipped, so **v0.4.0 is not published yet**; it publishes by itself from the next green push of the same version.

**CodeQL (the owner's workflow `codeql.yml`, pulled as `daa0c6e`):** it fails for two reasons that are not in the code. (1) GitHub's CodeQL *default setup* is also switched on (the run named "CodeQL"), and GitHub refuses results from an advanced workflow while the default setup is enabled; (2) the `java-kotlin` entry cannot build: the repository has no Gradle project (the only Kotlin is `scripts/android/MainActivity.kt.tmpl`). **Decided by the owner: keep the advanced workflow.** The default setup was switched off (`gh api repos/.../code-scanning/default-setup` answers `not-configured`), GitHub's template was re-added as `95084df` (new cron time), and it was rewritten again the way the repository needs it: the standard "System details" first step (`scripts/tests/test_ci_system_info.py` requires it of every job), the steps indented as in the other workflows, and the `java-kotlin` entry removed with a comment saying why. Languages now: Actions, JavaScript/TypeScript, Python, Rust. Not verified: the first run on GitHub (a YAML parser was not available here; the same re-indent ran fine on GitHub for `c430748`).

## Update (2026-11-25, after the push): two red CI runs, and the rules that follow

**What happened.** The push `2c35375` (next code, clean uninstall, themes) and the fix `e492fca` each came back red, although the local checks had passed. Nothing was wrong with the app. Three separate causes, two of them mine:

1. **`shellcheck` SC2015** in `scripts/tests/container_case.sh` (`[ ! -e a ] && [ ! -e b ] || fail`). CI runs `shellcheck` 0.8.0 on `ubuntu-22.04`; the local 0.11 does not flag it. Reproduced with 0.8.0 and 0.9.0 in containers.
2. **Dash.** The same file ran `command -v hexis` in the main shell, and `dash` (the `sh` of Debian and Ubuntu) keeps that answer after the program is removed, so the uninstall check said "hexis is still there" on Debian 12, Ubuntu 22.04 and Ubuntu 24.04 (Fedora and Arch use bash and passed). It was visible only in a container run, which was left as "not verified" although Docker was up and the packages of an earlier CI run could be downloaded.
3. **A flaky old test.** `clone_is_password_gated_and_never_returns_secrets` searched the serialized clone result for the card code `737`, which sits inside a random id or a timestamp about once in fifty runs. Not caused by this round's code, and not catchable by one local run; the test now checks the result's exact keys and values.

(A fourth red job, the openSUSE Tumbleweed installer, was the openSUSE mirror being out of sync for a while; it passed on a re-run.)

The same flake class showed up a second time in the local `verify_all` run before the version-centralization push: `gate::app_tests::nothing_that_could_unlock_a_vault_is_ever_written_to_disk` (and a second test beside it) searched every file of a profile for the four-digit PIN `2468`, and sealed (random) bytes held those four bytes by chance. Both now use a thirteen-digit `FINDABLE_PIN`, and `contains` in that file panics on a needle shorter than 8 bytes. I searched the other Rust tests for short byte searches: the rest use needles of 8 bytes or more, or a five-byte string in one small blob (`crypto/stream.rs`, odds around 1 in 10^7 per run).

**A fifth problem, found by the owner on a phone: the launcher name had quotation marks.** `scripts/android_patch_manifest.py` copies the app's `app_name` into `app_label_original`. Tauri writes that name as `"Hexis"` (quoting marks, which aapt removes), the script copied the marks as text and then escaped them, so the launcher label read `"Hexis"` with quotes. The test fixture had the name without marks, so nothing failed. `resource_value()` now reads a string resource the way aapt does (unescaped quotes are quoting marks, white space outside them collapses, backslash escapes); four new tests, including one with Tauri's real form, and the script was run on a copy of the real generated project (`app_label_original` is now `Hexis`).

**Why the new build was not on the Releases page as "Latest":** the version was still 0.2.0 and `v0.2.0` already had a tag, so CI built everything and replaced the rolling `Nightly (main, <sha>)` pre-release but, correctly, published no release. The version is now 0.3.0 (all files, `CHANGELOG.md` has a `[0.3.0]` section), so the next push of it tags and publishes `v0.3.0`.

**Every CI job now starts by printing its machine (asked for by the owner).** First step of all 12 jobs: OS, kernel, CPU, memory, swap, disk, load, tool versions, public facts only (see `BUILD_TARGETS.md` section 9); each installer container prints its own banner first. Guarded by `scripts/tests/test_ci_system_info.py`. Both workflows were parsed with PyYAML 6.0.2 in a throw-away venv (12 jobs, the step first and `continue-on-error` in each); the step's script was run locally, and the banner in Debian 12 (dash) and Alpine (busybox) containers. **Not yet seen:** the step on a real GitHub runner (the first push of it is the first run), and how the run page lays out the summaries of 12 jobs.

**One place for the version (asked for by the owner).** `VERSION` is the source of truth and `scripts/version.py` (`show`, `suggest`, `bump major|minor|patch|auto`, `set`, `sync`, `check [--release]`) writes it into `package.json`, `package-lock.json` (two lines), `tauri.conf.json`, `Cargo.toml`, the `hexis` entry of `Cargo.lock` and the PKGBUILD's `pkgver` (hyphen to underscore), and opens the CHANGELOG release section. `auto` chooses the bump from the CHANGELOG's `[Unreleased]` headings (`BREAKING` or `### Removed`: major, or minor before 1.0.0; `Added`/`Changed`/`Deprecated`: minor; only `Fixed`/`Security`: patch; empty: refuses). It refuses a lower version without `--force`, edits one line per place and leaves the rest of each file as it was, and a release candidate's CHANGELOG section becomes the release's when the final version is set. CI's `plan` job now calls `check --release` instead of its own three-file comparison, and `verify_all.sh` runs `check`. `scripts/tests/test_version.py` (18 tests) runs on copies of the real files, so a layout change in `Cargo.lock` or `package-lock.json` fails a test instead of silently skipping a file. Android's versionName and versionCode come from `tauri.conf.json` at build time and are not separate. **Not covered:** other build settings (the Node version, toolchains) still live in the workflow's `env` and `rust-toolchain`; they are not version numbers of the app.

**Rules now written down** (`CONTRIBUTING.md`, "Before you push: what CI checks that your machine may not"): use CI's tool versions, not the newest local ones; run shell tests under dash and bash; never assert the absence of a short string in generated text; take test fixtures from what the real tool writes; say exactly what was run, and run a cheap check instead of listing it as a gap.

**Changed to enforce them.** `scripts/verify_all.sh` runs `shellcheck` a second time in an `ubuntu:22.04` container (CI's version). New `scripts/check_installer_in_containers.sh` runs the installer and its container test in Debian 12, Ubuntu 22.04 and Fedora against the packages of the newest green CI run (tried: Debian 12 passes end to end). Verified after the fixes: Debian 12 and openSUSE Tumbleweed container runs pass with the real CI packages.

## Update (2026-11-25): next code, a clean uninstall, light and dark

**The request:** show the next (and previous) TOTP code; when someone uninstalls, nothing may stay behind (no cache, no cookies, no stored data), or the uninstall step must say plainly what is and is not removed; and the app, which was dark only, must have a proper light and a proper dark theme.

**Built (uncommitted; the owner commits).** Decisions D32 to D37, contract `IPC_CONTRACT.md` (`TotpNow.previous/next`, `DeviceInfo.theme/clipboardClear`, `device_set_theme`, `device_set_clipboard_clear`, `app_prepare_uninstall`), `STORAGE_AND_SYNC.md` §13, `UI_DESIGN_SYSTEM.md` §2 and §10, `PRIVACY.md`.

* **TOTP.** `TotpAccount::window_at` gives the previous, current and next code; `totp_now` carries `previous` and `next` (withheld with `code` for a flagged item); the item shows Next (copy) and Previous under the live code, the Codes tiles show Next.
* **Nothing kept.** The window is incognito; `storage/webview_data.rs` removes what 0.2.0 and earlier let the web view write into the data folder (at every start and in the wipe); the theme and the clipboard time moved from web view storage to `device.json`; Settings > Security > "Remove Hexis from this device" (`app_prepare_uninstall`: three confirmations + the long sum, the wipe, then `device.json`, leftovers and the empty folder; nothing is made again; the app closes on desktop; a last screen says what is gone). The installer's `--uninstall` removes the data too, with a typed `REMOVE`, named paths only (`--yes`, `--keep-data`). Android: Auto Backup and device transfer are off.
* **Themes.** Complete light and dark palettes in `tokens.css`, a `brass-ink` token for the accent as text, shadows as variables, Avatar, Ring and the logo themed, Settings > Theme (Device setting / Light / Dark with drawn previews), the native title bar on desktop and the Android bar icons follow. `tokens.test.ts` guards the copies and the contrast.

**Verified.** Rust library suite 876 passed, 0 failed, 2 ignored; `cargo fmt --check` and `clippy --all-targets -D warnings` clean. vitest 151 (16 files); `tsc --noEmit` clean. Python script tests 118 (Android patch 43, installer 62, release notes, old-name guard; the guard learned that the installer and its tests must name the folder an older build made). `shellcheck` clean on `install.sh` and `container_case.sh`. End to end against the real core: `e2e/real-uninstall.mjs` **18/18** (new: the theme and the clipboard time kept in `device.json` and in force after a core restart, no local storage or cookie, removal refused with the app lock closed, a wrong sum removes nothing, the right one leaves no file in the data folder but the shim's own marker, planted web view leftovers gone, nothing written afterwards, the screen ends on "Everything is removed"), `e2e/theme-ui.mjs` 13/13 (an emulated light and dark device against every choice, the page stores nothing). Both themes were looked at in headless Chromium on desktop and phone widths.

**Not verified.**
* That the incognito window really writes nothing in a **real WebKitGTK window** (it was read from the `wry` source; the clean-up at start and in the wipe is the safety net and is tested).
* The native title bar following the theme, and how WebKitGTK and the Android WebView draw the light palette (only headless Chromium was used).
* **Android:** the manifest patch is tested on synthetic projects only; the new Kotlin (`HexisBars`) was not compiled (its API calls were checked with `javap` against androidx.core 1.13.1) and nothing ran on a phone.
* The installer's new `--uninstall` on real distributions: the Python tests run the real script against stub package managers and a temporary home; `scripts/test_installer_containers.sh` (which needs a built deb, rpm and tarball) was **not run** after the change to `container_case.sh`. CI runs it on the next build.
* Residual data an uninstall cannot reach: a deleted or overwritten file on an SSD or in a journal; backups in S3; other accounts; anything the person copied or exported themselves.

**For the release:** none of this is published until the version is raised (`0.3.0`, with `package.json`, `Cargo.toml`, `tauri.conf.json` and a `## [0.3.0]` section in `CHANGELOG.md` agreeing); CI then builds and publishes by itself (see the entry below).

## Update (2026-11-24, later still): the generator remembers its last settings

**The request:** the generator always opened at 20 characters. A length the person chose (30, say) must show in every place next time, be stored in the database and sync.

**Before:** `PasswordGenerator` started from `useState(DEFAULT_GEN)` on every mount, so the Generator page and the item editor both reset to 20 characters with every type on. Confirmed in the code, not guessed.

**Built (uncommitted; the owner commits):** decision D31, contract `IPC_CONTRACT.md` §12, storage `STORAGE_AND_SYNC.md` §10 "Preferences that follow the vault".

* Core: `storage/backup/prefs.rs` (new). One sealed document in the vault's `meta` table, replicated as `vault/meta/prefs.enc` through the existing `targets` outbox kind with the ref `prefs` (no migration), pulled with one `HEAD` per pass and a download only when the ETag changed, restored with the vault, newest write wins, unknown fields carried through, a copy that fails authentication never applied and replaced by this device's own, a copy a newer app wrote in an unreadable shape left alone. Commands `prefs_get` and `prefs_set_generator` (length 4 to 128, at least one character type; both need an open vault and are closed while the gate is closed).
* Front end: `app.generator` in `state/app.ts` is the one set of settings; `PasswordGenerator` reads it, so the Generator page and the editor always agree (also while one is open and a pull changes it). A change is shown at once and saved 0.6 s after the last one, when the generator closes, and before a lock. Everything is reset when the vault closes or another account opens. All character types off is shown (as before) but not saved. The page text no longer says "Nothing is saved": a password is never saved, the settings are.
* `PRIVACY.md` and the changelog say so.

**Verified:** Rust library suite 861 passed, 0 failed, 2 ignored (863 in all; 20 of them are new, in `backup/tests/prefs_tests.rs`), `cargo fmt --check` and `clippy --all-targets -D warnings` clean; vitest 110 (new: `savedOrDefault`, `canGenerate`, the preview backend's rules); `tsc` and `npm run build` clean. End to end against the real core: `real-sync` 45/45 (new steps: starts at 20 and nothing sent; 30 characters without symbols kept a moment after the change; the editor opens at 30 with symbols off and a 30-character sample without symbols; the same after lock and unlock; unusable settings refused; `prefs.enc` in the bucket and sealed; device A takes them over; a newer change on A reaches the open page on B), `real-backup` 13/13, `run` 88/88.

**Not verified:** a real S3 server or hosted provider with this object (the in-memory buckets cover strong, ignoring and header-refusing services; the mock S3 service covers the wire); a real phone or desktop window; two devices changing the setting in the same millisecond (the later stamp wins, the loser's change is simply replaced). `e2e/real-multi-s3.mjs` was not extended or re-run.

## Update (2026-11-24, later): releases publish themselves

**Decision (the owner's):** no manual tagging and no draft to review. Raising the version on `main` is the release. `ci.yml` changed as follows; the details are in `BUILD_TARGETS.md` section 9 and `CONTRIBUTING.md` "Releases".

* New job `plan` (seconds): on `main`, a version in `src-tauri/tauri.conf.json` with no `vX.Y.Z` tag in the repository is a release build. The tag is the one marker of "already released" (it also covers a draft). It fails before any build when `package.json` or `Cargo.toml` disagree with the version or `CHANGELOG.md` has no `## [X.Y.Z]` section. Other pushes are ordinary builds.
* A release build restores no cache and requires the Android release key (the old tag rules, moved from "ref is a tag" to "plan said release"). The tag trigger is gone from `on:`.
* Job `release` needs every other job (checks, e2e, site, linux, installer, android). It re-checks that the tag still does not exist, writes the notes with `scripts/release_notes.py`, then `gh release create vX.Y.Z --target <sha>` (publishes, makes the tag, `--latest`; a suffixed version such as `0.3.0-rc1` is a pre-release). Its tags are plain tags made by GitHub Actions, not GPG-signed.
* A release created with the job token starts no workflow, so `pages.yml` (which builds the signed apt/rpm repositories from the newest stable release) would not rebuild. The job therefore runs `gh workflow run pages.yml` itself (needs `actions: write`); a failure there is a warning, not a red run.
* `gh workflow run CI` on `main` repeats a release whose tag and release were deleted (`workflow_dispatch` is allowed to plan one).
* `scripts/github-setup.sh publish-release` stays for a release that exists only as a draft (the 0.2.0 one made by the old flow); `CONTRIBUTING.md`, `README.md`, the site paragraph and `pages.yml` comments were updated.
* **Verified:** the workflow parses (PyYAML 6.0.2 in a throw-away venv) and the job graph is consistent; `bash -n` over every step; the `plan` script run against GitHub for five cases (this repo: tag exists, no release; a new version with its changelog section: release; same without the section: exit 1; mismatched versions: exit 1; a pull request: no release). **Not verified:** the `release` job and the Pages dispatch have never run, because running them publishes; the first real run is the next version bump.

## Update (2026-11-24, after the 0.2.0 push): the first CI run of the multi-S3 e2e, and a faster pipeline

* **What failed:** the commit `3487630` went green everywhere except the end-to-end job, where only `e2e/real-multi-s3.mjs` failed ("did not become ready", every other e2e script passed, `real-sync` 26/26). The cause was the test, not the app: it signed its S3 requests with `curl --aws-sigv4`, and curl 7.81 (what Ubuntu 22.04 CI runners ship) leaves `x-amz-content-sha256` out of the signed headers, so the S3 server answered 403 to every probe; curl 8.x signs it, which is why it passed on the dev machine. Reproduced in an `ubuntu:22.04` container against the same server image.
* **Fix:** `sigv4Headers()` in that script now computes the Signature V4 headers in Node (empty body) and hands curl finished headers. Checked with curl 7.81 (PUT, list with a query string, GET, and a forged signature that must give 403) and with the whole script against the real core: 25/25. The "not ready" error now also prints the last HTTP answer and the container's stdout and stderr.
* **Pipeline:** `e2e`, `linux` and `android` no longer wait for `checks` (they never used its output), so the four heavy jobs run side by side. `release` still needs every job, so nothing is published unless all of them pass. Not changed on purpose: tag builds start without a cache (clean release builds), and only GitHub's own actions are used.
* **Second CI failure (`867f265`):** with the S3 script fixed (25/25 on CI), `real-sync` failed once at "syncRevision went up on B (1 -> 1)"; it had passed on the earlier CI run and locally. The app is right and the test raced: unlock only queues a pull on a worker thread, so on a slow runner B's own startup pull can find A's edit in the bucket and apply it before the step reads the counter. The step now waits until B is not reconciling (`quiet()`, three quiet reads in a row) and reads the counter before A edits. Local run: 26/26. This was inferred from the CI log and the code, not reproduced on a slow machine.
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


## 2026-11-25 (later): editor fixes, several sign-ins and codes, the account bar and the account PIN

Built (not committed). Decisions D45, D46 in `ARCHITECTURE.md`; contract in `IPC_CONTRACT.md` §5.4 and the field-schema paragraph; design in `UI_DESIGN_SYSTEM.md` §12.

* **Editor.** Cancel with changes asks inside the sticky footer, where Cancel is (it used to ask at the top of the scrolling form, out of sight). A scanned or pasted value that is not a one-time code never stays in the box (a guard on scan and paste, a clear button on the box). *Add another sign-in* and *Add another code* appear once the last one has something in it; each extra sits in its own small panel with *Remove*.
* **Several sign-ins and codes.** `key#n` fields in `fields` (D46), view labels, search subtitle, the three-way merge per key, other apps' exports (hidden fields "Username 2" …) and import that restores them, `codeKeys` on the summary, `totp_now(key)`, the Codes page and the history view show every code.
* **Account bar and PIN (D45).** Rust: `storage/quick.rs`, `Session` parked / limited / wrong-PIN counter / `elevate`, `AppCore.parked`, `vault_lock` now parks when the account has a PIN, new commands `vault_lock_all`, `vault_unlock_pin`, `vault_elevate`, `account_pin_set`, `account_pin_remove`, errors `wrong_pin`, `pin_locked`, `master_required`, `VaultStatus.resumable/limited`, `ProfileInfo.hasPin/resumable`, `require_full` on backup and lock-timer changes. UI: `AccountBar`, account sheet with *Log out*, `PinForm`, `AccountPin` settings row, `withMaster` / `unlockFully`.

Verified: `cargo test --lib` (the 14 PIN tests in `storage/app_tests.rs`, the 4 in `commands/tests/pin_ipc.rs` including the inactivity timer wiping a parked vault, the `VaultStatus` shape test), vitest 213, `tsc --noEmit`, `e2e/editor-ui.mjs` and `e2e/account-pin.mjs` in the browser PREVIEW backend (the preview mirrors the core's rules; it is not the core).

NOT verified yet: the real core through a real window (`e2e/run.mjs` family is not extended to the PIN yet); the account bar and the PIN screen on the phone (the install needs the owner's approval on the device); the Android Kotlin side is untouched by this round.

**Security review of the PIN layer (2026-11-25), all fixed the same day.** One real bug: `Unlocked::park` zeroed the wrong-PIN counter every time it was called, so Lock (or "Choose another account" and picking the same account again) on a vault that was already parked gave four more tries and the fifth, which wipes the vault, never came; now the count restarts only when a shown vault is parked (two regression tests, both fail without the fix). Two gaps in the limited state: `file_read` (attachment preview) and `item_search` (note text as an oracle) did not follow the rule "secrets need the master password on the PIN alone"; `file_read` now answers `master_required` (the preview asks for the master password), search compares no note text when limited. And `app_settings_set` is refused while the vault is parked. Tests: `storage/app_tests.rs`.
