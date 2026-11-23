# BUILD_TARGETS — Linux (deb / rpm / AppImage / Arch) and Android (API 28 → 36+)

Verified against the official Tauri v2 prerequisites page (checked 2026-11-23). Items marked **[unverified]** have not been run on this machine (no Android SDK, no webkit2gtk-4.1 installed yet) and must be confirmed on first use.

## 1. Common toolchain

* Rust stable (**1.100.0** at time of writing; tauri 2.12 needs ≥ 1.90): `rustup default stable`. Components: `clippy`, `rustfmt`.
* Node.js + a package manager (`npm` is what the scripts assume).
* `npm install` then `npm run tauri dev` (desktop dev) / `npm run tauri build` (release bundles).
* Android Rust targets (only for mobile): `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android`.

## 2. Linux system dependencies (from the Tauri docs)

**Arch / CachyOS / Manjaro** (the dev machine; `webkit2gtk-4.1` is currently **missing** here, run this once):
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

`src-tauri` cannot compile until `webkit2gtk-4.1` is installed. The crypto module has no GUI dependency, so:
```
python3 scripts/crypto-harness.py            # builds a bare crate that includes src-tauri/src/crypto, runs cargo test
python3 scripts/crypto-harness.py clippy     # clippy on the same crate
```
The harness regenerates its `Cargo.toml` from `src-tauri/Cargo.toml` (dropping tauri entries), so dependency versions cannot drift. After installing the system packages, the same tests also run with `cd src-tauri && cargo test`.

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
