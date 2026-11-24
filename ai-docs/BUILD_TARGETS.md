# BUILD_TARGETS — Linux (deb / rpm / AppImage / Arch) and Android (API 28 → 36+)

Verified against the official Tauri v2 prerequisites page (checked 2026-11-23). Items marked **[unverified]** have not been run on this machine (no Android SDK, no webkit2gtk-4.1 installed yet) and must be confirmed on first use.

## 1. Common toolchain

* Rust stable (**1.100.0** at time of writing; tauri 2.12 needs ≥ 1.90): `rustup default stable`. Components: `clippy`, `rustfmt`.
* Node.js + a package manager (`npm` is what the scripts assume).
* `npm install` then `npm run tauri dev` (desktop dev) / `npm run tauri build` (release bundles).
* Android Rust targets (only for mobile): `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`.

## 2. Linux system dependencies (from the Tauri docs)

**Arch / CachyOS / Manjaro** (the dev machine; `webkit2gtk-4.1` 2.54.3 and the rest were installed on 2026-11-23; on a fresh machine run this once):
```
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg xdotool
```
**Debian / Ubuntu:**
```
sudo apt update && sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```
**Fedora / RHEL:**
```
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel
sudo dnf group install "c-development"
```

## 3. Bundles

Configured in `src-tauri/tauri.conf.json` → `bundle.targets = ["deb", "rpm", "appimage"]`.

| Artifact | Command | Notes |
|----------|---------|-------|
| `.deb` | `npm run tauri build -- --bundles deb` | Output in `src-tauri/target/release/bundle/deb/`. Build on the **oldest** distro you support (glibc is forward-compatible only). Ubuntu 22.04 or newer ships `libwebkit2gtk-4.1`. |
| `.rpm` | `npm run tauri build -- --bundles rpm` | Output in `…/bundle/rpm/`. |
| `.AppImage` | `npm run tauri build -- --bundles appimage` | Downloads linuxdeploy at build time (network needed). On Arch/CachyOS, if the bundler fails in the `strip` step, retry with `NO_STRIP=true` (known issue with recent toolchains) **[unverified here]**. Build on an older glibc for wide compatibility. |
| Arch package | `makepkg -si` in `packaging/arch/` (see below) | Tauri has no pacman bundler. |
| `.tar.gz` | `npm run tauri build -- --no-bundle` then `tar czf hexis-<ver>-linux-x86_64.tar.gz -C src-tauri/target/release hexis` (+ desktop file/icons) | Plain binary archive; needs webkit2gtk-4.1 at runtime. CI builds it (`hexis-<version>-linux-x86_64.tar.gz`). |

### Arch `PKGBUILD` (`packaging/arch/PKGBUILD`, written 2026-11-24)

Tauri has no pacman bundler, so the package compiles the binary itself. It builds the *committed* files of the clone it sits in (`git archive HEAD` into `$srcdir`; no download,
so no checksum to maintain), refuses to build when `pkgver` differs from `tauri.conf.json`, runs `npm ci` and `npm run tauri build -- --no-bundle`, and installs the binary,
`packaging/hexis.desktop`, the 128 px icon and the licence. Use: `cd packaging/arch && makepkg -si`. **Syntax and `makepkg --printsrcinfo` were checked; a full `makepkg` build was
not run.** For an AUR upload, switch `source` to the release tarball and fill in its checksum. The `tar.gz` that CI publishes contains the same binary, desktop entry and icon.

### Wayland / X11 notes

* WebKitGTK runs natively on Wayland (Hyprland, Niri, GNOME, KDE) and X11. If a window renders blank or crashes on some GPU/compositor combinations, try `WEBKIT_DISABLE_DMABUF_RENDERER=1 hexis` **[unverified here]**.
* Clipboard (Phase 3): `arboard` with its Wayland data-control support, falling back to `wl-copy`/`wl-paste` when the compositor lacks it; auto-clear timer 15 / 30 / 60 s, clearing only if the clipboard still holds our value.
* System tray/appindicator libraries listed above are needed only if a tray icon is added later.

## 4. Android (minSdk 28 / Android 9; the Tauri template compiles and targets API 37)

**Built for real on 2026-11-24** (debug APK, arm64, Android 16 device attached; the install on that device was blocked by
the phone's own settings, see "Installing on a Xiaomi phone" below). What follows is what was actually run, not a plan.

### 4.1 Toolchain (user level, no sudo)

`scripts/setup_android_toolchain.sh phase1` downloads the command-line tools and Temurin JDK 21 (both checksum-verified) and
adds the Rust target; `phase2` installs the SDK pieces with `sdkmanager`. **Google's current command-line tools deprecate
`sdkmanager --licenses` (it prints a notice and records nothing); the replacement is the bundled `android` CLI, which shows
the SDK terms on first run and writes `licenses/android-sdk-license`.** The pieces that ended up installed: `platform-tools`,
`platforms;android-36` and `platforms;android-37.0` (the generated Gradle project uses `compileSdk = 37` and `targetSdk = 37`;
Gradle fetches the second itself once the licence file exists), `build-tools;36.0.0`, `ndk;28.2.13676358`.
`eval "$(scripts/setup_android_toolchain.sh env)"` exports `JAVA_HOME`, `ANDROID_HOME`, `NDK_HOME` and `PATH`.
JDK 21 is required: Gradle and the Android Gradle plugin do not run on newer JDKs.

### 4.2 The generated project is not committed

`src-tauri/gen/android/` is created by Tauri and is git-ignored. It is recreated by exactly these steps (CI runs the same):

```
npm ci
npm run tauri android init -- --ci --skip-targets-install       # creates src-tauri/gen/android
python3 scripts/android_patch_manifest.py src-tauri/gen/android # disguise aliases + icons + DisguisePlugin.kt + manifest markers
mkdir -p src-tauri/gen/android/app/src/main/java/dev/habibullah/hexis/securescreen
cp scripts/android/SecureScreenPlugin.kt src-tauri/gen/android/app/src/main/java/dev/habibullah/hexis/securescreen/
python3 scripts/check_no_notifications.py                       # no POST_NOTIFICATIONS, no foreground service, no tray crate
npm run tauri android build -- --debug --apk --target aarch64   # debug APK (or without --debug for the release APK)
```

The patch script (idempotent, 37 tests) also adds two manifest *removal* markers after the real build showed what the merge produces:
`VIBRATE` (declared by the QR-scanner plugin for a command Hexis never grants) is removed, and `android.hardware.camera.any` becomes
`required="false"` so the app installs on devices without a camera. The merged manifest was inspected with `apkanalyzer`: permissions
`INTERNET`, `ACCESS_NETWORK_STATE`, `CAMERA`, `USE_BIOMETRIC`, `USE_FINGERPRINT` (plus an AndroidX-internal receiver permission); no
`POST_NOTIFICATIONS`, no `FOREGROUND_SERVICE*`. Library `<service>` entries exist (CameraX `MetadataHolderService`, disabled; ML Kit and
Google data-transport discovery services); none is a foreground service. `DisguisePlugin` and `SecureScreenPlugin` are both in the dex.

### 4.3 Signing and the release APK

Gradle produces `app-universal-release-unsigned.apk`. CI aligns it with `zipalign` and signs it with `apksigner` using the repository
secrets `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`
(`scripts/github-setup.sh android-key` creates the key outside the repository, stores the first three as secrets and the public
SHA-256 fingerprint of its certificate as the variable `ANDROID_CERT_SHA256`; the key password is the store password, so `ANDROID_KEY_PASSWORD`
is optional). Pull-request runs never receive the secrets. Without a key the APK is signed with a throw-away key made inside that CI run and its
file name ends in `-testkey`: it installs, but cannot be updated by a build signed with another key. A **version tag** build refuses that
fallback and fails (set the repository variable `ALLOW_TESTKEY_RELEASE` to `true` to override), so a release cannot silently ship a test key
again. The release notes and the Download page print the fingerprint so a user can check a file with `apksigner verify --print-certs`. The
keystore never lives in the repository (`*.jks`, `*.keystore`, `keystore.properties` are git-ignored).

### 4.4 Installing on a Xiaomi phone (what blocked the on-device check)

`adb install` returned `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user` on a Xiaomi (HyperOS) phone that was asleep and locked.
MIUI/HyperOS asks for approval on the screen and, in addition, has the developer option **Install via USB** (it can require a Mi account).
Unlock the phone, enable that option, then run the install again and accept the prompt:
```
adb -s <id> install -r -t src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb -s <id> shell am start -n dev.habibullah.hexis/.AliasOriginal
```

### 4.5 What has not been verified on a device

Everything that needs the running app on a phone: `FLAG_SECURE` (check with `adb exec-out screencap -p` while a drive file is open: the image must
be black, or `adb shell dumpsys window | grep -i secure`), locking on background, the launcher icon switching between the 12 aliases, the
biometric prompt, the camera QR scan, and `adb reverse` sync against `scripts/mock_s3.py`. The Android file dialog returns `content://` URIs,
so import, drive add and save-a-copy do not work there yet.

### 4.6 Biometrics, camera and the QR scanner

* API 28-29: `BiometricPrompt` cannot combine `BIOMETRIC_STRONG | DEVICE_CREDENTIAL`; offer biometric only and fall back to the gate credential or master password in our own UI.
* API 30+: combined authenticators are allowed. The fingerprint only opens the Stage 1 **gate**; it never wraps or caches a vault key (decision D13).
* **QR scanning uses Google ML Kit through Google Play services (`play-services-mlkit-barcode-scanning` 18.1.0). Google documents that ML Kit may send
  technical usage metrics, and the APK contains Google's data-transport runtime. This is the only known path by which an Android build might talk to a
  third party; it has not been measured. Open task: replace it with an on-device decoder (the webview already bundles `jsqr`) or remove the data-transport
  components and verify the scanner still works.** See `PRIVACY.md`.

Android memory note: Argon2id at 64 MiB x 4 passes is acceptable on typical devices; the allowed range (up to 1 GiB) is user-configurable, so the UI must warn on low-RAM devices.

## 5. Verifying crypto without GUI libraries

The full `src-tauri` crate needs `webkit2gtk-4.1` (installed on the dev machine, so `cargo test` there covers everything). On a machine without the GUI stack, the crypto module has no GUI dependency, so:
```
python3 scripts/crypto-harness.py            # builds a bare crate that includes src-tauri/src/crypto, runs cargo test
python3 scripts/crypto-harness.py clippy     # clippy on the same crate
```
The harness regenerates its `Cargo.toml` from `src-tauri/Cargo.toml` (dropping the tauri entries and, since Phase 4a, the network crates `rusty-s3`, `ureq` and `url`, which the crypto module does not use and whose rustls/ring build is slow), so dependency versions cannot drift. After installing the system packages, the same tests also run with `cd src-tauri && cargo test`.

Low-RAM tip (dev machine has ~7 GB): `CARGO_BUILD_JOBS=2`, and run builds one at a time. Dev-profile builds optimize only the crypto crates (see `[profile.dev.package.*]` in `Cargo.toml`) so Argon2id/ML-KEM are not painfully slow in tests.

## 6. Frontend and icons (verified 2026-11-23)

```
npm install                 # exact pinned versions from package.json; creates package-lock.json
npm run typecheck           # tsc --noEmit (TypeScript 7)
npm run build               # tsc --noEmit && vite build -> dist/ (what tauri.conf.json's frontendDist points at)
npm run tauri dev           # needs the system packages from §2; starts Vite on :1420 and the app
```
Icons: the artwork is `src/assets/logo.svg` (full icon), `logo-mark.svg` and `logo-mark-mono.svg`; `python3 scripts/gen_icons.py` renders them (rsvg-convert) and runs `tauri icon assets/icon-manifest.json`, which writes `src-tauri/icons/*` (desktop, `.icns`/`.ico`, Android, iOS). Replace `assets/app-icon.png` with real artwork any time and re-run the second command.
CSP consequence for UI code: no inline scripts and no inline `style` attributes (see `tauri.conf.json`).
`src-tauri/Cargo.lock` is deliberately **not** git-ignored: it should be tracked (this is an application, not a library). Update it deliberately with `cargo update -p <crate>` after checking the new version, never blindly.

## 7. Phase 4a: backup network stack and the mobile-only QR scanner

Written 2026-11-23. **Nothing in this section has been run on an Android device or emulator**: there is no Android SDK on the dev machine. What *was* checked: the desktop build, tests, clippy and the capability/ACL generation (below).

### 7.1 No OpenSSL for backups

Backups use `rusty-s3` (sans-IO; it only builds SigV4 presigned URLs) and blocking `ureq` with **rustls + ring and the bundled web roots** (`webpki-roots`). No OpenSSL is linked for the network; the `openssl` package in §2 is not needed for it (it stays in the list because other tooling on those distros may want it). Consequences:

* The web roots are compiled in, so a custom CA (a private MinIO with a self-made certificate) is **not** trusted. Use a public CA certificate, or `http://` on the loopback address for local testing. This is deliberate: there is no "ignore certificate errors" switch anywhere.
* `ring` compiles C and assembly. A desktop build needs a C compiler (already in `base-devel` / `build-essential`). For Android the NDK clang is used, which `tauri android build` wires up through `CC_<triple>` / `AR_<triple>`. **[unverified]** that `ring 0.17` builds for all four Android triples here; it is widely used on Android, but no one has run it on this machine.

### 7.2 `tauri-plugin-barcode-scanner` (QR codes for TOTP setup), phones only

* It sits under `[target.'cfg(any(target_os = "android", target_os = "ios"))'.dependencies]` in `src-tauri/Cargo.toml` and is registered in `src-tauri/src/lib.rs` behind the same `cfg`. On Linux/macOS/Windows the crate is not in the dependency graph at all (`cargo tree` shows no `barcode`), so the desktop build, `cargo test` and `scripts/crypto-harness.py` are unaffected.
* Its permissions are granted by `src-tauri/capabilities/mobile.json` (`"platforms": ["android", "iOS"]`): only `barcode-scanner:allow-scan`, `allow-cancel`, `allow-check-permissions`, `allow-request-permissions`, `allow-open-app-settings`. The plugin's default set also contains `allow-vibrate`; it is **not** granted. The desktop capability file (`default.json`) grants none of them. The build script skips a capability whose `platforms` do not include the build target, so the desktop build neither fails on nor applies it.
* **Android manifest** (what the plugin brings; the Tauri Gradle build merges it into the app's manifest automatically; checked in the plugin's source, 2.5.3): `<uses-permission android:name="android.permission.CAMERA"/>`, `<uses-permission android:name="android.permission.VIBRATE"/>` and `<uses-feature android:name="android.hardware.camera.any"/>`. Three things to review after `npm run tauri android init`:
  1. `VIBRATE` is a normal permission with no prompt. Hexis never calls the vibrate command (not granted), so remove it with `tools:node="remove"` in `src-tauri/gen/android/app/src/main/AndroidManifest.xml` if you want the smallest permission set.
  2. `uses-feature camera.any` is *required* by default, which hides the app on Google Play from devices without a camera. Hexis works without one (the setup key can be typed), so override it with `android:required="false"` and `tools:node="replace"`.
  3. `CAMERA` is a runtime permission: the plugin asks when the scan starts (`barcode-scanner:allow-request-permissions`) and `allow-open-app-settings` is how the UI sends the user to the settings screen after a "don't ask again" denial. Hexis must treat a denial as "scanning unavailable" and offer manual entry; scanning is never required.
* **minSdk:** the plugin declares `minSdk = 24`, `compileSdk = 36`; Hexis's `bundle.android.minSdkVersion = 28` is higher, so there is no conflict. CameraX 1.5.1 and **Google Play services ML Kit** (`play-services-mlkit-barcode-scanning` 18.1.0) are pulled in: on a phone **without Google Play services** (for example a de-Googled ROM) the scanner can be unavailable, so manual entry of the setup key stays a first-class path.
* **iOS** (not a target yet): a camera usage description (`NSCameraUsageDescription`) is required in `Info.plist`.
* **UNVERIFIED on a device:** the permission prompt, the camera preview, the decoding of an `otpauth://` QR code, the denied-forever path, the manifest merge and the Play-services dependence were not run anywhere. Treat this section as a checklist for the first Android build, not as a result.

### 7.3 How the backup network layer was verified without a real provider

`scripts/mock_s3.py` (stdlib only, loopback only) re-implements AWS Signature V4 *verification* independently of `rusty-s3` and serves path-style Put/Get/Head/Delete/ListObjectsV2 with `If-None-Match` / `If-Match`. `cargo test` spawns it (skipped with a printed `SKIPPED` line when `python3` is missing) and drives the real `S3Store` against it. Run it by hand with `python3 scripts/mock_s3.py --port 9333 --bucket hexis-test --access-key AKIAMOCKMOCKMOCK --secret-key mock-secret-access-key-not-real --region us-east-1` and point a target at `http://127.0.0.1:9333` (bucket `hexis-test`, those keys, "path style" on, any prefix). Flags: `--port` (`0` picks a free one and prints `MOCK_S3_PORT=<n>` as the first line), `--access-key`, `--secret-key`, `--region`, `--bucket`, `--data-dir <folder>` (**objects are kept in memory unless this is given**, then each object is one file in that folder, named by the hex of its key, and survives a restart), `--page-size N` (most keys per listing page, to exercise pagination), `--fail-after N` with `--fail-status 403|500|503`, `--verbose` (method, path, status; never a query string or a secret), `--selftest`. **Fail toggle while it runs:** `curl -s -X POST -H 'Content-Type: application/json' -d '{"status":403}' http://127.0.0.1:9333/__mock/fail` makes every request answer 403 (`500` and `503` also work; add `"after":3` to start with the 4th request); `-d '{"status":null}'` recovers; `POST /__mock/reset` forgets all objects; `GET /__mock/state` lists the keys. These control URLs refuse any request that carries an `Origin` header, so a web page cannot reach them. It listens on `127.0.0.1` only and prints no secret. With the dev shim: `e2e_server_r2 --data-dir <new folder> --port 1432 --fast-kdf --fast-backup` runs the real backend, and a `backup_add` through `POST /ipc` with the mock's endpoint works as in the app (the shim uses the real S3 client). **Not verified:** any real provider (AWS S3, R2, B2, MinIO, Wasabi, Storj), real TLS to a public CA, real clock skew, and the provider-specific quirks of conditional writes.

## 8. Leaving the app: the exit and background flush (R3a / Q0)

Written 2026-11-24. Trigger 3 of the sync engine is "app exit or backgrounding" (`STORAGE_AND_SYNC.md` §10): send what is still queued, for at most 8 seconds in total, without ever blocking the UI thread. The queue is durable (it lives in the vault file, in the same transaction as the change), so **nothing is lost when a flush does not finish**: the rest goes at the next unlock or change.

### 8.1 Linux (desktop): built and tested logic, real window path not driven

* `WindowEvent::CloseRequested` (`src-tauri/src/lib.rs`, `on_window_event`): if no vault is unlocked or nothing is queued, the window closes at once. Otherwise the close is prevented, the flush runs **on its own thread**, and the window is destroyed when the flush returns or after 8 s. A **second** close request while a flush is running closes at once (the person is never trapped). `RunEvent::Exit` then locks the vault (keys zeroized, worker stopped) however the app ends.
* Tested without a GUI: the decision logic (`ExitGate`, `flush_then` in `commands/mod.rs`) and `AppCore::flush_for_exit` against `MemStore` (a flush returns by its deadline even when a request hangs, and with an empty queue it makes no store call). **Not driven:** the real window's `CloseRequested` event (no GUI was started on the dev machine).
* A kill signal, a crash, a power cut or a `kill -9` run no flush. That is by design: the queue is durable.

### 8.2 Android: what is wired, what is guaranteed, what is not (**UNVERIFIED: not built, not run**)

The Android code path could not be compiled or run here (no Android SDK, §4). What exists is checked only by reading the sources of the pinned crates (tauri 2.12.7, tauri-runtime 2.12.4, tauri-runtime-wry 2.12.5, tao 0.37.3):

* **`onPause` is forwarded.** tao maps the Activity's `onPause` to `WindowEvent::Suspended`, and tauri-runtime-wry forwards it (`#[cfg(mobile)]`). `on_window_event` calls `commands::flush_in_background` for it: the flush runs on its own thread and never blocks the event loop. The vault **stays unlocked**; locking after inactivity is a separate concern (the lock timer, planned for Phase 3), not this hook.
* **`onStop` is not forwarded.** tao reports it as `WindowEvent::Stopped`, but tauri-runtime-wry 2.12.5 drops it (`_ => return Self(None)`), so Rust code cannot hook it without a custom plugin or Activity code. `onPause` fires before `onStop`, so the flush has already started by then.
* **The page's own hook.** The webview calls the IPC command `sync_flush` from `visibilitychange` (`document.visibilityState === "hidden"`). It does the same bounded flush and answers `{ complete }`. This is the second, independent route to the same code.
* **What is not guaranteed.** Android may freeze or kill a backgrounded process at any moment, throttle background network access (Doze, App Standby, data saver, battery restrictions), or take the connection away; `onPause` reaches Rust through tao's event loop (a wake-up message), not synchronously inside the Activity callback, so the handler can run a moment after the Activity has left the foreground. A flush that is cut short leaves the queue intact and is simply retried at the next unlock or change. No foreground service, no WorkManager job and no notification are used or wanted: the app has no notification of any kind (`scripts/check_no_notifications.py` exits non-zero if a notification or tray plugin, permission or manifest entry appears; it prints "no Android project generated yet" until `npm run tauri android init` has been run, and **must be run again after `android init`** to check the generated manifest).
* **Checklist for the first Android build:** (1) put the app in the background with a non-empty queue and watch the bucket (for example `scripts/mock_s3.py` on the host, made reachable from the device with `adb reverse`); (2) rotate and switch apps (an Activity recreate is not an exit; nothing may lock or flush wrongly); (3) swipe the app away from recents (expect no flush; the next unlock sends it); (4) confirm that `visibilitychange` really fires before `onPause` in the WebView of the target API levels; (5) run `python3 scripts/check_no_notifications.py` against `src-tauri/gen/android`.

## Q2 disguise: Android launcher aliases and the Linux launcher entry

Android disguise (Q2) **[unverified: no Android SDK was available]**. After `npm run tauri android init` (and again whenever `assets/disguise/` changes, and after any re-init that regenerates the manifest):
```
sudo pacman -S librsvg        # or: sudo apt install librsvg2-bin   (needs rsvg-convert)
python3 scripts/android_patch_manifest.py src-tauri/gen/android --dry-run   # print the plan, write nothing
python3 scripts/android_patch_manifest.py src-tauri/gen/android             # apply
python3 scripts/android_patch_manifest.py src-tauri/gen/android --check     # exit 1 when something is missing or stale (CI)
```
* It removes the LAUNCHER intent filter from the main activity (so there is one icon, not two) and adds `.AliasOriginal` (enabled) plus one `<activity-alias>` per preset (`.AliasKerf`, `.AliasTally`, ... disabled, `exported=true`, same intent filter as the original). Each alias has its own `@string/app_label_<id>` and `@mipmap/ic_launcher_<id>`.
* It writes `res/values/disguise_strings.xml`, `res/values/disguise_colors.xml`, `res/mipmap-anydpi-v26/ic_launcher_<id>.xml` (adaptive icons), legacy PNGs `res/mipmap-{mdpi..xxxhdpi}/ic_launcher_<id>.png` (48 to 192 px) and foregrounds `ic_launcher_<id>_foreground.png` (108 dp), all rendered from `assets/disguise/<id>.svg` and `<id>-fg.svg` with `rsvg-convert`; and `DisguisePlugin.kt` next to `MainActivity.kt` (package from `namespace` in `app/build.gradle.kts`, or `--package`).
* **`MainActivity.kt` and the R8 keep rules are written by the script too** (changed 2026-11-24). `MainActivity.kt` comes from `scripts/android/MainActivity.kt.tmpl`: edge-to-edge with native keyboard padding, Back sends the app to the background (after asking the page through `window.__hexisBack` whether it has a sheet to close) instead of ending the process, and the recents picture is blank on Android 13 and later. `app/hexis.pro` comes from `scripts/android/hexis.pro`: without it R8 removed the constructors of ML Kit's component registrars and the first camera frame crashed the app with a `NullPointerException`; the generated `app/build.gradle.kts` already feeds every `*.pro` file under `app/` to R8, so the script only has to put the file there. CI runs the same script, so both are reproduced on every build.
* **The disguise itself needs no `MainActivity` edit.** Tauri's `register_android_plugin("<package>", "DisguisePlugin")` (called from `commands::disguise::android_plugin()` in `lib.rs`) instantiates the class by JNI and registers it with the activity's plugin manager. The plugin's one command is `setAlias`; it enables the chosen alias first, then disables the others with `DONT_KILL_APP`. The script adds no `android.permission.*`.
* **Run the script before the first Android build.** If the class is missing, the plugin registration is non-fatal on the Rust side, but a pending JNI exception may still be raised by the failed lookup; the disguise setting then only changes the in-app title.
* It is idempotent (second run: `up to date`), keeps comments, the prolog and unused namespace declarations, repairs aliases that drifted, and only **warns** about stale `.Alias*` entries (for example a preset that was removed from `presets.json`); it never deletes them. Exit codes: 0 done or up to date, 1 `--check` found work, 2 bad input or a missing tool (nothing is written when `rsvg-convert` is missing).
* The manifest of a regenerated project must be patched again (`tauri android init` rewrites it).

Linux note for section 3: the disguise writes `$XDG_DATA_HOME/applications/<identifier>.desktop` (default `~/.local/share/applications/dev.habibullah.hexis.desktop`) and the icon under `$XDG_DATA_HOME/<identifier>/disguise/`. Nothing is installed system-wide and no root is needed. Packaged builds also ship their own `hexis.desktop`; both show up while a disguise is active.

## Q3 drive: Android FLAG_SECURE plugin (unverified)

`screen_secure(on)` needs a tiny local Tauri plugin so decrypted previews cannot be screenshotted, recorded or shown in the recents list.

1. Source: `scripts/android/SecureScreenPlugin.kt` (package `dev.habibullah.hexis.securescreen`, class `SecureScreenPlugin`, command `setSecure({ on })`, sets or clears `WindowManager.LayoutParams.FLAG_SECURE` on the UI thread; modelled on tauri 2.12.7 `mobile/android` and tauri-plugin-barcode-scanner 2.5.3; never compiled).
2. After `tauri android init`, copy it to `src-tauri/gen/android/app/src/main/java/dev/habibullah/hexis/securescreen/SecureScreenPlugin.kt` (the `gen/android` folder is generated, so the source is kept in `scripts/android/`).
3. Rust side, already in the tree: `commands::drive::secure_screen::init()` registers a plugin named `secure-screen` whose setup calls `register_android_plugin("dev.habibullah.hexis.securescreen", "SecureScreenPlugin")`; `lib.rs` adds it under `#[cfg(target_os = "android")]`; `screen_secure` calls `run_mobile_plugin("setSecure", {on})` and answers a fixed sentence on failure. Not compiled for an Android target here.
4. No manifest change, no Android permission, no Gradle dependency beyond what a Tauri app has.
5. Known gap: the Android file dialog answers `content://` URIs; the drive's picker interface works with local paths. Adding and saving files on Android needs a picker that hands back readers and writers; it is not built (the commands fail with a clear sentence instead of misbehaving).

## 9. CI/CD and releases (`.github/workflows/`)

Written 2026-11-24. **First run on GitHub, 2026-11-24 (the first push, `maga-v2`): every job passed**, including the Linux bundles, the Android APK (signed, `-testkey` suffix because no
release key was set) and the `nightly` pre-release with all five files and `SHA256SUMS`. The later run on the Dependabot pull request failed once because the first headless Chrome on
the runner needed more than the 10 seconds `e2e/cdp.mjs` waited (the other eight suites passed); the launch now waits 20 seconds and retries twice. Not yet observed on a runner: a tag build
(`release` job), and the `installer` and Pages-repository steps below (written after that run).

| Workflow / job | Trigger | What it does |
|---|---|---|
| `ci.yml` `checks` | every push, every PR | `scripts/verify_all.sh`: `tsc`, vitest, `vite build`, no dev-only strings in `dist/`, notification check, patch-script tests, `cargo fmt`, `clippy -D warnings`, `cargo test --locked` |
| `ci.yml` `e2e` | after `checks` | `scripts/ci_e2e.sh`: builds the dev shim, starts Vite, runs every `e2e/*.mjs` against the real core in headless Chrome (`CHROME_BIN`) |
| `ci.yml` `site` | every push, every PR | builds the website and fails on a broken link or anchor or a third-party request |
| `ci.yml` `linux` | after `checks` | `npm run tauri build -- --bundles deb,rpm,appimage` on `ubuntu-22.04` (old glibc), plus a `tar.gz`; artifacts kept 14 days |
| `ci.yml` `installer` | after `linux` | `scripts/test_installer_containers.sh`: builds signed apt and rpm repositories from that build with a throw-away key and runs the real `install.sh` in `ubuntu:22.04`, `ubuntu:24.04`, `debian:12`, `fedora:latest`, `opensuse/tumbleweed`, `archlinux:latest` (install, libraries resolve, starts under Xvfb, second run, uninstall) and in `alpine` and `ubuntu:20.04`, which it must refuse |
| `ci.yml` `android` | after `checks` | the steps of section 4.2 with the release build, then align and sign; artifact kept 14 days |
| `ci.yml` `nightly` | push to `main`, after `checks`, `linux`, `android` | deletes and recreates the `nightly` pre-release with all files and `SHA256SUMS` |
| `ci.yml` `release` | tag `v*`, after everything including `e2e`, `site` and `installer` | verifies the tag equals the version in `package.json`, `tauri.conf.json` and `Cargo.toml`, then creates a **draft** release |
| `pages.yml` | push to `main` touching docs, `site/` or the installer; a **stable release published**; manual | builds the site with `SITE_URL` from `actions/configure-pages`; if a stable release and the `REPO_SIGNING_KEY` secret exist, also builds the signed apt and rpm repositories into the site (section 11); deploys it |

Design choices: only GitHub's own actions are used (`checkout`, `setup-node`, `setup-java`, `cache`, `upload/download-artifact`, `configure-pages`,
`upload-pages-artifact`, `deploy-pages`), so no third-party action can alter a build; tag builds never restore a cache; `permissions` default to read-only and
only the publishing jobs get `contents: write`; a tag release is a draft so a human looks before anything is published. The `android` job accepts Google's SDK
licence on behalf of whoever owns the workflow (one commented line). Windows, macOS and iOS are not built: the Rust core has `#[cfg(unix)]` file-permission code
and has never been compiled or tested there. Not done: build provenance attestations (need a public repository or a paid plan), an `aarch64` Linux build
(arm runners are not free for private repositories), an AAB for Google Play, a scheduled dependency audit.

`scripts/github-setup.sh` holds the one-time repository settings (Pages source, private vulnerability reporting, topics, the Android key secrets, the repository signing key). It is run by the owner;
it changes no visibility and contains no commit or push. A GitHub Pages site is public even when the repository is private, and Pages on a private repository needs a
paid plan.

## 10. The website (`scripts/build-site.mjs`, `site/site.css`)

A static site built from this repository's own Markdown (README, `ai-docs/`, CONTRIBUTING, SECURITY, PRIVACY, CHANGELOG, the code of conduct, the licence) plus a hand-written
home and download page. `npm run`-free: `node scripts/build-site.mjs` writes `site-dist/` (git-ignored). The only dependency is `marked` (pinned `18.2.0`, chosen over the
just-published 18.3.0 because it is 15 days old). The OFL fonts the app bundles (Fraunces, Instrument Sans, IBM Plex Mono) are copied from `node_modules` with their licences.
The script exits 1 on: an internal link or `#anchor` that does not resolve, a remote `src`/`href`, a `<script>` other than JSON-LD, or a title over 60 characters (warning).
Raw HTML in Markdown is shown as text; images are dropped (the README badge is a request to github.com). No JavaScript, no cookies, no analytics; light and dark follow the system.
Environment: `SITE_URL` (absolute base for canonical, sitemap and JSON-LD) and `SITE_REPO_URL` (where repository links point); CI passes both from `actions/configure-pages` and `github.repository`.

## 11. The one-line installer and the package repositories (`scripts/install.sh`, `scripts/make-repos.sh`)

Added 2026-11-24, modelled on how `https://dl.brave.com/install.sh` works: a short POSIX `sh` file that detects the system, **registers the vendor's signed package repository** and
lets the system package manager install and later update the program. Hexis adds a download-and-verify fallback for systems that have no repository.

**Flow of `install.sh`** (`--detect` prints what it found, `--dry-run` prints the plan and every command, `--help` lists the options):

1. Refuses early, with a reason: not Linux, not x86_64 (points to `--apk`), musl (Alpine), NixOS, glibc older than 2.34 (the CI binary's newest symbol; the AppImage needs 2.35).
2. Finds `curl` or `wget`, and `sudo`/`doas`/`run0`/`pkexec` unless root. Detects the distribution from `/etc/os-release` (`ID`, `ID_LIKE`), the package manager by the commands present (apt+dpkg, dnf, zypper, pacman, yum),
   and read-only systems (`/run/ostree-booted`, SteamOS), which are never changed with their package manager.
3. Stable channel, whole-machine install, apt/dnf/yum/zypper present, and the repository published (a probe of `InRelease` or `repomd.xml`): writes the keyring and a `.sources` (apt, deb822, `Signed-By`) or `.repo` file
   (`gpgcheck=1`, `repo_gpgcheck=1`), then `apt-get install -y hexis` / `dnf install -y hexis` / `zypper install hexis`. The package is named `hexis` in both formats (only the rpm file name is capitalised).
   On pacman systems: `pacman-key --add` and `--lsign-key` for the project key (fingerprint from `key-fingerprint.txt`), a marked `[hexis]` block appended to `/etc/pacman.conf` (`SigLevel = Required DatabaseRequired`),
   the repository database and its signature placed in pacman's sync folder **without `pacman -Sy`** (refreshing every repository before installing one package is the unsupported partial upgrade; a stale system fails with a
   hint to run `pacman -Syu`), then `pacman -S --needed --noconfirm hexis`.
4. Otherwise a direct download from the GitHub release (`releases/latest/download`, `releases/download/nightly`, or `/v<version>/`), checked against that release's `SHA256SUMS` (a mismatch deletes the file and stops):
   Debian family `.deb`, rpm family `.rpm`, pacman systems without a published repository the tarball into `/usr/local` with `webkit2gtk-4.1 gtk3` from pacman, everything else a **user** install under `~/.local` (the tarball when WebKitGTK 4.1 and GTK 3 are
   present, else the AppImage, unpacked once when FUSE 2 is missing). A manifest (`share/hexis/install-manifest`) lists every file so `--uninstall` removes exactly those and never touches vault data.
5. With no stable release it warns and uses the `nightly` pre-release. A private repository answers 404 to anonymous downloads; the installer then tries `gh release download` if the GitHub CLI is signed in.
6. `--apk` (and Termux) downloads and verifies the Android APK instead.

**The repositories** (`scripts/make-repos.sh`; tools `gpg`, `apt-ftparchive`, `createrepo_c`, `rpmsign`; and `scripts/make-arch-repo.sh`, which runs `makepkg` and `repo-add` in an `archlinux` container and
needs only docker): `apt/dists/stable/{Release,InRelease,Release.gpg}` over `apt/pool/main/p/hexis/*.deb`, and `rpm/x86_64/` (signed rpm,
`repodata/`, `repomd.xml.asc`) plus `rpm/hexis.repo`, and `arch/x86_64/` (`hexis.db`, `hexis.files`, the `.pkg.tar.zst`, every one with a detached `.sig`; real files, because Pages refuses symlinks), the public key as `hexis-archive-keyring.gpg` (apt) and `.asc` (rpm), and `key-fingerprint.txt`. They are static files inside the Pages site. `pages.yml` rebuilds them on
every deploy from the **newest stable release** (Pages replaces the whole site on each deploy), signing with the `REPO_SIGNING_KEY` secret (`scripts/github-setup.sh repo-key` prints the commands that create it; optional
`REPO_SIGNING_PASSPHRASE`). Only stable releases are in the repositories; `nightly` keeps the same version number, so it is delivered by direct download only. Because a `release` event runs on the tag and the `github-pages`
environment only accepts the default branch, `pages.yml` answers the event by dispatching itself on `main`.

**Verified (2026-11-24) against the CI-built nightly files in real containers**: Ubuntu 22.04 (signed apt repo, no missing library, program ran for 12 s under Xvfb), Debian 12 (apt repo; tarball system install; AppImage user
install on a GTK desktop without WebKit; tampered checksum refused with nothing installed; APK download), Fedora (signed rpm repo with `gpgcheck` and `repo_gpgcheck`, second run a no-op, uninstall), openSUSE Tumbleweed
(`zypper`), Arch (signed pacman repository: key trusted, package "Validated By SHA-256 Sum, Signature", ran under Xvfb, second run, `pacman -Rns` uninstall removes the `pacman.conf` block, the database and the key; the tarball path was verified too), and refusals on Alpine (busybox `ash`), Ubuntu 20.04 (`dash`) and an aarch64 override. 33 hermetic tests in `scripts/tests/test_install_sh.py` (stub package managers, a fake
root, a local HTTP server). **Not verified**: a real run of `pages.yml` with a signing key (needs the secret and a stable release), `pkexec`/`run0`/`doas` as the root helper, rpm-ostree and SteamOS hosts, aarch64 Linux (no build exists), and `make-arch-repo.sh` on a GitHub runner (it needs docker there; it ran only locally).

**What it trusts.** The script comes from the project's Pages site over HTTPS; the repositories are trusted through the signing key the installer downloads from the same site on first use (trust on first use, like Brave's), whose
fingerprint is printed on the Download page once published; direct downloads are checked against a `SHA256SUMS` from the same release, which catches corruption and a bad mirror but not a compromised release. Losing or leaking
the signing key means every user has to import a new key. Anyone can read the script first: `/download/install-script/` shows the exact text.
