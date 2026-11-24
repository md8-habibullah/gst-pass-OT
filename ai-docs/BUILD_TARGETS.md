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
| Arch package | `makepkg -si` using the template below | Tauri has no pacman bundler. |
| `.tar.gz` | `npm run tauri build -- --no-bundle` then `tar czf passman-<ver>-linux-x86_64.tar.gz -C src-tauri/target/release passman` (+ desktop file/icons) | Plain binary archive; needs webkit2gtk-4.1 at runtime. |

### Arch `PKGBUILD` template (to be added under `packaging/arch/` in Phase 5)

```bash
pkgname=passman
pkgver=0.1.0
pkgrel=1
pkgdesc='Post-quantum zero-knowledge password vault'
arch=('x86_64' 'aarch64')
license=('MIT')            # placeholder: set the real license before release
depends=('webkit2gtk-4.1' 'gtk3' 'libappindicator-gtk3' 'openssl')
makedepends=('rust' 'cargo' 'nodejs' 'npm')
source=("$pkgname-$pkgver.tar.gz")   # fill in release URL + checksum
sha256sums=('SKIP')

build()   { cd "$pkgname-$pkgver"; npm ci; npm run tauri build -- --no-bundle; }
package() {
  cd "$pkgname-$pkgver"
  install -Dm755 src-tauri/target/release/passman "$pkgdir/usr/bin/passman"
  install -Dm644 packaging/passman.desktop "$pkgdir/usr/share/applications/passman.desktop"
  install -Dm644 src-tauri/icons/128x128.png "$pkgdir/usr/share/icons/hicolor/128x128/apps/passman.png"
}
```

### Wayland / X11 notes

* WebKitGTK runs natively on Wayland (Hyprland, Niri, GNOME, KDE) and X11. If a window renders blank or crashes on some GPU/compositor combinations, try `WEBKIT_DISABLE_DMABUF_RENDERER=1 passman` **[unverified here]**.
* Clipboard (Phase 3): `arboard` with its Wayland data-control support, falling back to `wl-copy`/`wl-paste` when the compositor lacks it; auto-clear timer 15 / 30 / 60 s, clearing only if the clipboard still holds our value.
* System tray/appindicator libraries listed above are needed only if a tray icon is added later.

## 4. Android (minSdk 28 / Android 9, targetSdk 36 / Android 16, API 37+ later)

Prerequisites (Tauri docs): Android Studio; `JAVA_HOME=/opt/android-studio/jbr` (path varies); SDK Manager → Android SDK Platform, Platform-Tools, NDK (Side by side), Build-Tools, Command-line Tools; then
```
export ANDROID_HOME="$HOME/Android/Sdk"
export NDK_HOME="$ANDROID_HOME/ndk/$(ls -1 $ANDROID_HOME/ndk)"
```

Steps (Phase 3 onwards, **[unverified]**):
1. `npm run tauri android init` → generates `src-tauri/gen/android/` (commit it; it is meant to be customized).
2. **minSdk 28**: `bundle.android.minSdkVersion = 28` in `tauri.conf.json` (already set; the schema key was verified).
3. **targetSdk / compileSdk 36**: edit `src-tauri/gen/android/app/build.gradle.kts` (`compileSdk = 36`, `targetSdk = 36`) if the generated template uses a lower value. Raise to 37+ (Android 17+) once the toolchain supports it and the app has been tested; Google Play requires targeting a recent API level.
4. **`FLAG_SECURE`** (blocks screenshots and the recents thumbnail): in `MainActivity.kt` `onCreate`, call `window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)`. Phase 3 may make it dynamic per screen through a small command.
5. Build: `npm run tauri android build -- --apk` (or `--aab`), dev: `npm run tauri android dev`. Release builds must be signed (keystore kept **outside** the repo).
6. **Biometrics** (Phase 3): `tauri-plugin-biometric` (mobile only). Compatibility plan:
   * API 28–29 (Android 9–10): `BiometricPrompt` exists but `BIOMETRIC_STRONG | DEVICE_CREDENTIAL` cannot be combined; offer biometric only, and fall back to Quick PIN / Master Password in our own UI.
   * API 30+ (Android 11+): combined authenticators are allowed; Class-3 biometrics can gate an Android Keystore key.
   * Always detect `canAuthenticate()`; on `NONE_ENROLLED` / `NO_HARDWARE` / lockout silently fall back to PIN, then Master Password.
   * The session key lives in the Android Keystore (hardware-backed, StrongBox where available, `setInvalidatedByBiometricEnrollment(true)`), never in app storage.
7. Test matrix before release: Android 9 (API 28), 10, 11, 12, 13, 14, 15, 16 — at least one device/emulator image each of the oldest, the middle (API 33) and the newest.

Android memory note: Argon2id at 64 MiB × 4 passes is acceptable on typical devices; the allowed range (up to 1 GiB) is user-configurable, so the UI must warn on low-RAM devices.

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
Icons: `python3 scripts/gen_icon.py` draws the placeholder `assets/app-icon.png`; `npm run tauri icon assets/app-icon.png` generates `src-tauri/icons/*` (desktop, `.icns`/`.ico`, Android, iOS). Replace `assets/app-icon.png` with real artwork any time and re-run the second command.
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
  1. `VIBRATE` is a normal permission with no prompt. Passman never calls the vibrate command (not granted), so remove it with `tools:node="remove"` in `src-tauri/gen/android/app/src/main/AndroidManifest.xml` if you want the smallest permission set.
  2. `uses-feature camera.any` is *required* by default, which hides the app on Google Play from devices without a camera. Passman works without one (the setup key can be typed), so override it with `android:required="false"` and `tools:node="replace"`.
  3. `CAMERA` is a runtime permission: the plugin asks when the scan starts (`barcode-scanner:allow-request-permissions`) and `allow-open-app-settings` is how the UI sends the user to the settings screen after a "don't ask again" denial. Passman must treat a denial as "scanning unavailable" and offer manual entry; scanning is never required.
* **minSdk:** the plugin declares `minSdk = 24`, `compileSdk = 36`; Passman's `bundle.android.minSdkVersion = 28` is higher, so there is no conflict. CameraX 1.5.1 and **Google Play services ML Kit** (`play-services-mlkit-barcode-scanning` 18.1.0) are pulled in: on a phone **without Google Play services** (for example a de-Googled ROM) the scanner can be unavailable, so manual entry of the setup key stays a first-class path.
* **iOS** (not a target yet): a camera usage description (`NSCameraUsageDescription`) is required in `Info.plist`.
* **UNVERIFIED on a device:** the permission prompt, the camera preview, the decoding of an `otpauth://` QR code, the denied-forever path, the manifest merge and the Play-services dependence were not run anywhere. Treat this section as a checklist for the first Android build, not as a result.

### 7.3 How the backup network layer was verified without a real provider

`scripts/mock_s3.py` (stdlib only, loopback only) re-implements AWS Signature V4 *verification* independently of `rusty-s3` and serves path-style Put/Get/Head/Delete/ListObjectsV2 with `If-None-Match` / `If-Match`. `cargo test` spawns it (skipped with a printed `SKIPPED` line when `python3` is missing) and drives the real `S3Store` against it. Run it by hand with `python3 scripts/mock_s3.py --port 9333 --bucket passman-test --access-key AKIAMOCKMOCKMOCK --secret-key mock-secret-access-key-not-real --region us-east-1` and point a target at `http://127.0.0.1:9333` (bucket `passman-test`, those keys, "path style" on, any prefix). Flags: `--port` (`0` picks a free one and prints `MOCK_S3_PORT=<n>` as the first line), `--access-key`, `--secret-key`, `--region`, `--bucket`, `--data-dir <folder>` (**objects are kept in memory unless this is given**, then each object is one file in that folder, named by the hex of its key, and survives a restart), `--page-size N` (most keys per listing page, to exercise pagination), `--fail-after N` with `--fail-status 403|500|503`, `--verbose` (method, path, status; never a query string or a secret), `--selftest`. **Fail toggle while it runs:** `curl -s -X POST -H 'Content-Type: application/json' -d '{"status":403}' http://127.0.0.1:9333/__mock/fail` makes every request answer 403 (`500` and `503` also work; add `"after":3` to start with the 4th request); `-d '{"status":null}'` recovers; `POST /__mock/reset` forgets all objects; `GET /__mock/state` lists the keys. These control URLs refuse any request that carries an `Origin` header, so a web page cannot reach them. It listens on `127.0.0.1` only and prints no secret. With the dev shim: `e2e_server_r2 --data-dir <new folder> --port 1432 --fast-kdf --fast-backup` runs the real backend, and a `backup_add` through `POST /ipc` with the mock's endpoint works as in the app (the shim uses the real S3 client). **Not verified:** any real provider (AWS S3, R2, B2, MinIO, Wasabi, Storj), real TLS to a public CA, real clock skew, and the provider-specific quirks of conditional writes.

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
* **No `MainActivity` edit is needed.** Tauri's `register_android_plugin("<package>", "DisguisePlugin")` (called from `commands::disguise::android_plugin()` in `lib.rs`) instantiates the class by JNI and registers it with the activity's plugin manager. The plugin's one command is `setAlias`; it enables the chosen alias first, then disables the others with `DONT_KILL_APP`. The script adds no `android.permission.*`.
* **Run the script before the first Android build.** If the class is missing, the plugin registration is non-fatal on the Rust side, but a pending JNI exception may still be raised by the failed lookup; the disguise setting then only changes the in-app title.
* It is idempotent (second run: `up to date`), keeps comments, the prolog and unused namespace declarations, repairs aliases that drifted, and only **warns** about stale `.Alias*` entries (for example a preset that was removed from `presets.json`); it never deletes them. Exit codes: 0 done or up to date, 1 `--check` found work, 2 bad input or a missing tool (nothing is written when `rsvg-convert` is missing).
* The manifest of a regenerated project must be patched again (`tauri android init` rewrites it).

Linux note for section 3: the disguise writes `$XDG_DATA_HOME/applications/<identifier>.desktop` (default `~/.local/share/applications/app.passman.vault.desktop`) and the icon under `$XDG_DATA_HOME/<identifier>/disguise/`. Nothing is installed system-wide and no root is needed. Packaged builds also ship their own `passman.desktop`; both show up while a disguise is active.

## Q3 drive: Android FLAG_SECURE plugin (unverified)

`screen_secure(on)` needs a tiny local Tauri plugin so decrypted previews cannot be screenshotted, recorded or shown in the recents list.

1. Source: `scripts/android/SecureScreenPlugin.kt` (package `app.passman.securescreen`, class `SecureScreenPlugin`, command `setSecure({ on })`, sets or clears `WindowManager.LayoutParams.FLAG_SECURE` on the UI thread; modelled on tauri 2.12.7 `mobile/android` and tauri-plugin-barcode-scanner 2.5.3; never compiled).
2. After `tauri android init`, copy it to `src-tauri/gen/android/app/src/main/java/app/passman/securescreen/SecureScreenPlugin.kt` (the `gen/android` folder is generated, so the source is kept in `scripts/android/`).
3. Rust side, already in the tree: `commands::drive::secure_screen::init()` registers a plugin named `secure-screen` whose setup calls `register_android_plugin("app.passman.securescreen", "SecureScreenPlugin")`; `lib.rs` adds it under `#[cfg(target_os = "android")]`; `screen_secure` calls `run_mobile_plugin("setSecure", {on})` and answers a fixed sentence on failure. Not compiled for an Android target here.
4. No manifest change, no Android permission, no Gradle dependency beyond what a Tauri app has.
5. Known gap: the Android file dialog answers `content://` URIs; the drive's picker interface works with local paths. Adding and saving files on Android needs a picker that hands back readers and writers; it is not built (the commands fail with a clear sentence instead of misbehaving).
