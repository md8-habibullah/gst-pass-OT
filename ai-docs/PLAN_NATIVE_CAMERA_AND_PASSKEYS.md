# Plan: native camera on Android, and Hexis as a passkey provider

Written 2026-11-25. Status: **plan only, nothing implemented, no source file touched.**
Audience: the Hexis owner (decisions and owner steps are in sections 9 and 10).
Rule of this file: every "fact" has a source in section 12 or was read in this repository; everything else is marked
**UNVERIFIED** and has a test in section 8 that turns it into a fact.
**Provenance of every source (what was opened, what was only a search snippet, what was downloaded, hashes, commands to repeat it):
[`REFERENCES_CAMERA_AND_PASSKEYS.md`](REFERENCES_CAMERA_AND_PASSKEYS.md). Read it before trusting or extending any claim here.**
Neither file is listed in `scripts/build-site.mjs`, so neither is published on the website.

---

## 0. Short version

> **UPDATE 2026-11-25 (phone attached, section 3.0 and `REFERENCES_CAMERA_AND_PASSKEYS.md` section 10).** Items 1 and 3 below were written
> before the phone was attached. Measured since then: Hexis 0.3.0 **does open the camera** on your phone (no crash, no camera error), so "camera not opening" is
> **not reproduced** and P1 (own scanner) is now an optional privacy/robustness step, not a bug fix. The phone reports `android.software.credentials = true`,
> and **Bitwarden is registered and selected as the passkey provider on this HyperOS 3.0 / Android 16 phone**, so the feared HyperOS block does not apply here.
> The phone also resolves `FIDO:/` links to Google Play services (`QRBounceActivity`). Passkeys (P2, P3) are now the main work.

1. **Camera.** The Tauri barcode-scanner plugin (`tauri-plugin-barcode-scanner` 2.5.3) swallows camera start-up errors
   (`catch (e: Exception) { // TODO }` in `BarcodeScannerPlugin.kt`) and draws the preview *behind a transparent web view*.
   So "camera does not open" leaves no error anywhere. I cannot name the cause without a log from your phone (none is
   attached now). Plan: first read the log (15 minutes, section 3.1), and in any case replace the plugin with **our own
   full-screen native scan screen** (CameraX + ZXing). That is how BitTorrent-style and password-manager apps do it, it shows real
   errors, and it also removes Google ML Kit / Play services from the app, which closes the open privacy item in
   `PRIVACY.md`.
2. **Passkeys.** What you describe is an Android **credential provider** (Android Credential Manager, `CredentialProviderService`).
   Once Hexis is enabled in *Settings > Passwords, passkeys & autofill*, Chrome, Firefox, other apps, and the QR "use a phone" sign-in
   sheet list Hexis next to Google Password Manager. Hexis creates, stores (in the encrypted vault, synced to your S3 buckets),
   and signs with the passkey. It needs **Android 14 or newer**.
3. **The biggest risk is your own phone.** It is a Xiaomi (HyperOS). Several reports say HyperOS blocks or overrides third-party
   passkey providers (section 6.6). One `adb` command tells us in 1 minute whether the phone supports it at all (section 3.1).
   If it does not, we test on another Android 14+ phone (or fall back to the desktop and in-app paths).
4. **"All types of passkeys, modern and old"** is possible for most things and **impossible for a few**, and I list both
   honestly in section 4 (for example: no hardware attestation, no U2F/CTAP1 on a phone, no reading Google's or Apple's own stored passkeys directly; they must be moved by
   the standard export format).
5. Order of work: **P0 diagnose, P1 native scanner, P2 passkey engine in Rust, P3 Android provider, P4 move passkeys in and out
   (standard format), P5 desktop, P6 hardening.** Rough size in section 7.

---

## 1. What is in the project today (read from the repository)

| Fact | Where |
|---|---|
| QR scanning = `@tauri-apps/plugin-barcode-scanner` 2.5.3 JS + `tauri-plugin-barcode-scanner` 2.5.3 Rust, registered for Android/iOS only, called with `scan({ windowed: true, formats: [QRCode] })` | `src/lib/scan.ts`, `src-tauri/src/lib.rs:27-31`, `src-tauri/Cargo.toml` |
| While scanning, the page sets `data-scanning` on `<html>`; CSS makes `html`/`body` transparent and hides everything except the overlay | `src/styles/base.css:195-204`, `src/components/scan/ScanOverlay.tsx` |
| The plugin brings CameraX 1.5.1 and `play-services-mlkit-barcode-scanning` 18.1.0 (Google ML Kit through Play services); R8 keep rules were needed or the first frame crashed | `scripts/android/hexis.pro`, `ai-docs/BUILD_TARGETS.md` §4.6, §7.2 |
| Open privacy item: ML Kit may send usage metrics to Google; decision "keep / replace with on-device decoder / strip transport" was never made | `PRIVACY.md` lines 49-63, `ai-docs/CHANGELOG_AND_STATE.md` |
| The camera was **never verified on a device**; `adb install` was refused (`INSTALL_FAILED_USER_RESTRICTED`, Xiaomi) | memory + `ai-docs/BUILD_TARGETS.md` §4.4-4.5 |
| Android build: `compileSdk 37`, `targetSdk 37`, `minSdk 28`; one `MainActivity` (`launchMode singleTask`), 12 launcher aliases (disguise), `FLAG_SECURE` plugin, biometric plugin for Stage 1 only | `src-tauri/gen/android/app/...`, `scripts/android_patch_manifest.py` |
| Item types today: login, card, identity, sshKey, note, media, totp. Payload `schema 1`; an unknown schema is rejected on read | `src-tauri/src/storage/model.rs` |
| Rust core is built by `commands::prepare_state(&dir, picker, opener)` inside Tauri's `setup`; `AppCore` owns the data dir | `src-tauri/src/lib.rs`, `storage/app.rs` |
| Decisions that bind this work: D10 (per-item `reprompt`), D13 (fingerprint opens Stage 1 only; **never wraps a vault key**), D39 (3-way merge across devices) | `ai-docs/ARCHITECTURE.md` |
| Nothing mentions passkeys/WebAuthn anywhere in source or docs (only the word "passkey" in the Stage-1 *passcode* sense) | grep |

---

## 2. Verified versions (looked up 2026-11-25; your rule: latest stable, read the real source)

| Thing | Use | Note |
|---|---|---|
| `androidx.credentials:credentials` | **1.6.0** (stable, 2026-04-08) | 1.7.0-alpha03 exists; pre-release, not used. `CredentialProviderService` lives in this artifact, needs API 34. Google's provider guide shows the alpha in its snippet; stable is enough |
| `androidx.credentials.providerevents:providerevents` | **1.0.0-beta01** (2026-09-09) | **No stable release exists.** Needed only for moving passkeys between apps (P4). Decision D-6 |
| CameraX (`androidx.camera:*`) | **1.6.2** now; 1.7.0 after 2026-12-02 | 1.7.0 was released 2026-11-18, only 7 days ago |
| `com.google.zxing:core` | **3.5.4** (Apache-2.0, pure Java, no Google services) | QR decoder for the native scanner |
| `androidx.biometric:biometric` | **1.1.0** stable (1.4.0-rc01 is a release candidate) | For the "verify it is you" step of a passkey |
| Rust `passkey` family (1Password, MIT/Apache) | **0.6.0** (2026-10-01); `passkey-crypto` 0.1.1 | I downloaded and read `passkey-authenticator`, `passkey-types`, `passkey-crypto` (section 5.1) |
| `p256` 0.14.0, `ed25519-dalek` 3.0.0, `rsa` 0.10.0, `coset` 0.4.2, `jni` 0.22.4, `ml-dsa` 0.1.2 | as listed | Only if we write parts ourselves; `passkey-crypto` already pulls p256/ed25519/ml-dsa |
| Test-only RP server: `webauthn-rs` 0.6.2 | dev-dependency | Verifies our output offline in `cargo test` |
| Google's browser allowlist | `https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json` | 73 apps today (Chrome, Firefox, Brave, Edge, Samsung Internet, Vivaldi, Yandex, GMS, ...) |

Re-check all of these on the day work starts (the numbers move).

---

## 3. Camera (P0 and P1)

### 3.0 Result of P0 (phone attached 2026-11-25; read-only commands only, nothing installed, no input sent)

| Check | Result |
|---|---|
| Phone | Redmi `25062RN2DA` (product `creek_global`), **Android 16 (SDK 36), HyperOS `OS3.0`**, security patch 2026-09-01 |
| Google Play services / store | both present; Play services `26.45.31` |
| `android.software.credentials` | **true** |
| Credential providers registered | Google (`com.google.android.gms/.auth.api.credentials.credman.service.PasswordAndPasskeyService`) and **Bitwarden** (`com.x8bit.bitwarden/.Autofill.CredentialProviderService`) |
| Selected provider | `credential_service` and `credential_service_primary` are both Bitwarden: **a third-party provider is active on this HyperOS 3.0 phone** |
| Handler of a `FIDO:/` link | `com.google.android.gms/.fido.authenticator.ui.QRBounceActivity` (found with `cmd package resolve-activity`; nothing was launched) |
| Installed Hexis | `dev.habibullah.hexis` **0.3.0** (versionCode 3000, targetSdk 37), installed 03:19 today by the system installer. The source tree is 0.4.0, so the phone runs older code than the repository |
| Camera permission | granted (`CAMERA: granted=true`; app-ops mode `allow`) |
| **Does the camera open?** | **Yes.** The camera service log shows Hexis connecting camera 0 at 03:41:22 (4 s) and twice at 11:40:30 and 11:40:38 (CameraX on Camera2, "Camera 0: Opened", clean disconnects). No crash, ANR or camera exception from Hexis after the 0.3.0 install. The only Hexis crashes in the log are the ML Kit R8 `NullPointerException` of 2026-11-24 15:45 and 15:48 (the v0.1.0 build, already fixed) and the Back-key mutex abort (also fixed) |
| **Not verified** | That the preview was visible and that a QR code was decoded: ML Kit logs no result. You said the scan is "working, maybe" |

What this changes:

* **"Camera does not open" is not reproduced.** The most likely explanations are the older v0.1.0 build that crashed on 2026-11-24, or a misread of the screen. If it happens again, capture the log with the commands in 3.1 at that moment.
* **P1 is no longer urgent.** Reasons that remain for replacing the plugin later: it ships Google ML Kit and needs Play services (the open `PRIVACY.md` exception), failures are silent, and the keep-rule patch (`hexis.pro`) is fragile. Do it after the passkey work unless the camera misbehaves again.
* **Forwarding `FIDO:/` to the system does not need a new scanner.** The existing plugin already returns the scanned text to the page; a small Kotlin command that fires `ACTION_VIEW` for `FIDO:/` links is enough, and this phone resolves such links (above). The end-to-end sign-in is still untested (T-12).
* **The HyperOS block reported for other phones does not apply here** (section 6.6). It is still not proven that **Hexis** will appear in the picker; spike S-3 proves that.
* **Testing caution for P3:** Bitwarden is currently your selected provider. Android also allows additional providers next to the preferred one; use that (or note your current settings first) so your Bitwarden passkeys stay usable while Hexis is tested.

### 3.1 P0 commands (kept for repeating the diagnosis later; needs the phone unlocked and USB debugging approved)

Why: the plugin hides the reason. Candidates, in the order I would check them:

1. **Permission / state**: camera permission never reaches "granted" (HyperOS asks in its own way, or "ask every time").
2. **Camera bind fails silently** (`bindToLifecycle` exception is swallowed) because of an OEM camera policy or another app holding the camera.
3. **Transparent web view stays opaque**: the plugin sets `webView.setBackgroundColor(TRANSPARENT)`, but any opaque layer (page background, a WebView hardware layer on that ROM) shows black/blank instead of the camera. (Tauri's scanner page describes windowed mode as making the web view transparent; I only saw this in a search snippet, ref R-18 in `REFERENCES_CAMERA_AND_PASSKEYS.md`.)
4. **`scan()` never settles** after cancel (open upstream issue #3560), leaving the page transparent with the overlay stuck.
5. ML Kit model/Play services missing: the preview would still show, but nothing is ever decoded.

Commands (the package is `dev.habibullah.hexis`, or `dev.habibullah.hexis.dev` for the side-by-side build):

```
adb devices                                  # phone must say "device", not "unauthorized"
adb shell getprop ro.product.model
adb shell getprop ro.build.version.release   # Android version
adb shell getprop ro.mi.os.version.name      # HyperOS marker (empty on non-HyperOS)
adb shell pm has-feature android.software.credentials     # passkey provider support: true/false  <-- key question for P3
adb shell cmd package query-services --brief -a android.service.credentials.CredentialProviderService
adb shell settings get secure credential_service
adb shell settings get secure credential_service_primary
adb shell pm list packages | grep -E "com.google.android.gms|com.android.vending"   # Play services present?
adb shell dumpsys package dev.habibullah.hexis | grep -B1 -A2 "android.permission.CAMERA"
adb logcat -c
#  -> now open Hexis, tap the QR scan button, wait 10 seconds, then:
adb logcat -d -v time | grep -i -E "hexis|CameraX|Camera2|mlkit|vision|AndroidRuntime|FATAL|chromium|WebView" > ~/scan-log.txt
```

Paste me the output (the log has no vault data; it contains app and camera messages only). With it I can say which of
the five candidates it is. **Even if it turns out to be something small, P1 is still recommended** (reasons below).

### 3.2 P1, our own native scan screen (recommended)

Options compared:

| Option | What opens | Google/Play services | Effort | Verdict |
|---|---|---|---|---|
| A. **Own `ScanActivity`: CameraX + ZXing core 3.5.4** | Full-screen native camera, our UI | **None** | 1-2 days | **Recommended** |
| B. Google code scanner (`play-services-code-scanner` 16.1.0) | Google's own scanner screen, no camera permission needed | Required (also breaks on phones without Play services) | 0.5 day | Fallback only |
| C. Keep the Tauri plugin and patch it (`windowed:false`, own error handling) | Plugin view over the page | ML Kit + Play services stay | 1 day, but we maintain a fork | Not recommended |
| D. ML Kit *bundled* (`com.google.mlkit:barcode-scanning` 17.3.0) | Our screen, Google's decoder library | No Play services, still a Google SDK with a data transport | 1-2 days | Only if ZXing reads codes worse in tests |

Design of A:

* `ScanActivity` (Kotlin, own theme, portrait/any, `FLAG_SECURE` on, because a TOTP QR code *is* a secret). CameraX `Preview` +
  `ImageAnalysis` (`STRATEGY_KEEP_ONLY_LATEST`); each frame's luminance plane goes to ZXing `QRCodeReader` (hints: `TRY_HARDER`, `POSSIBLE_FORMATS=QR_CODE`; second pass with
  `InvertedLuminanceSource`). First valid text wins; the activity returns it and finishes.
* The activity asks for `CAMERA` itself (`ActivityResultContracts.RequestPermission`). "Denied" and "denied forever" get a plain
  explanation screen with a button to the app settings. **Every failure shows a real sentence** (no camera, camera in use, permission denied,
  policy block) and returns a typed error to the page. Log tag `HexisScan`, never logging the scanned text.
* Controls: torch, pinch-zoom, tap-to-focus, "choose an image" (the existing jsQR image path stays), Cancel, Back.
* Hand-over to the page: a small local Tauri plugin (same pattern as `SecureScreenPlugin.kt` and `DisguisePlugin.kt`): Kotlin `ScanPlugin` with command `scan`
  using `startActivityForResult`, Rust `register_android_plugin(...)`, one capability line `hexis-scan:allow-scan`.
  `src/lib/scan.ts` keeps its public API (`scanWithCamera`, `cancelCameraScan`, `canScanWithCamera`, `ScanError`), so `TotpInput.tsx` does not change.
* **Remove**: `tauri-plugin-barcode-scanner` (Rust), `@tauri-apps/plugin-barcode-scanner` (npm), `capabilities/mobile.json` scanner entries,
  the R8 rules for ML Kit in `scripts/android/hexis.pro`, `ScanOverlay.tsx` and the `data-scanning` CSS. Smaller APK, and
  `PRIVACY.md` loses its Android exception (a real improvement to document).
* **Bridge to passkeys**: if the scanned text starts with `FIDO:/` (the QR a computer shows for "use a phone"), the scanner does not treat it as an error
  or as TOTP; it hands it to the system with `Intent.ACTION_VIEW` so Android's own passkey flow takes over (and then lists Hexis, section 6.5). Whether the system
  accepts that intent from an app is **UNVERIFIED** (test T-12).
* Tests: a JVM unit test generates QR bitmaps with ZXing's writer (plain, inverted, small, rotated, damaged, with `otpauth://` text) and runs them through our decoder (Gradle `testDebugUnitTest`,
  no device). Everything involving the real camera needs your hands (T-1).
* Files: `scripts/android/ScanActivity.kt`, `ScanPlugin.kt`, layout/theme XML, extend `scripts/android_patch_manifest.py` (+ its tests in `scripts/tests/test_android_patch.py`),
  `src-tauri/src/commands/scan.rs` (plugin registration only), `Cargo.toml`, `package.json`, docs.

---

## 4. What "all types of passkeys" can and cannot mean

| Kind / feature | Hexis can? | How / why not |
|---|---|---|
| **Synced passkeys** (discoverable credentials, the normal "passkey") | **Yes** | Stored in the vault (flags BE=1, BS=1), synced by your S3 sync, usable on every Hexis device |
| Non-discoverable credentials (site sends `allowCredentials`) | **Yes** | We look the credential up by id; we also store every credential so it can be found either way |
| Cross-device sign-in by QR ("hybrid", phone as the authenticator) | **Yes on Android 14+, via the system** | Google Play services runs the hybrid protocol and Credential Manager asks Hexis to sign (section 6.5). **UNVERIFIED on your phone** |
| Algorithms | ES256 (-7), EdDSA (-8/-19), ESP256 (-9) **yes**; **ML-DSA-44/65/87 (post-quantum) yes in the library, but no website accepts them yet**; RS256 (-257) **is not implemented in the 1Password crate's crypto backend** (it only appears in a doc comment of the types; checked by grep), add later with `rsa` 0.10.0 if a site needs it; ES384/ES512 later with `p384` | Sites list their accepted algorithms; nearly every site lists ES256 |
| Extensions | `credProps` yes; **`prf` (hmac-secret) yes**, the library supports it; `largeBlob`, `credBlob`, `devicePubKey`, `appid` **no** | |
| Attestation | **`none` only** (zero AAGUID) | Hexis is software. A site that *requires* a certified hardware authenticator (some banks/enterprises) will refuse it; same for Bitwarden/1Password/Google Password Manager |
| Hardware-key behaviour (PIN, resident-key slots, device-bound only) | **No** | Different product class; Hexis passkeys are software and synced by design |
| **U2F / FIDO1 / CTAP1** (old second-factor keys) | **Not on a phone** (Credential Manager has no U2F path; the 1Password crate contains no U2F protocol code, only an attestation-format enum value, checked by grep). Possible on desktop only, as a virtual USB key (section 6.7, optional) | State this plainly in the UI |
| Passkeys already stored in **Google Password Manager / iCloud** | **Cannot be read by Hexis** | They can only be *moved* with the standard exchange format (CXP/CXF, section 5.5) when the other app offers it, or re-created per site |
| Android 13 and older (your `minSdk` is 28) | **No provider** | Credential provider API starts at Android 14. On 9-13 Hexis still manages/exports passkeys in its own UI, but the system will not offer it for sign-in |
| Desktop browsers (Chrome/Firefox on Linux) | **Not directly** | No platform provider API exists on Linux yet; options in section 6.7 |

---

## 5. Passkey engine and storage (P2, in Rust, no UI yet)

### 5.1 Use 1Password's `passkey` crates (do not hand-write WebAuthn)

Read from the downloaded 0.6.0 sources:

* `passkey-authenticator` provides the CTAP2 *authenticator*: `make_credential`, `get_assertion`, `get_info`, hmac-secret/PRF, with two traits we implement over our vault:
  `CredentialStore` (`find_credentials`, `save_credential`, `update_credential`, `get_info`) and `PasskeyAccessor` (key, credential id, rp id, user handle, username, display name, counter, extensions). Our vault stays the single store.
* `passkey-crypto` default feature `rust-crypto` is **pure Rust** (p256, ed25519-dalek, ml-dsa, hmac, sha2), so it cross-compiles for Android like the rest of the core. Do **not** enable `aws-lc-rs` (C code).
* `passkey-types` brings CBOR/JSON/WebAuthn types (no U2F protocol; `largeBlobKey` is documented in its source as "currently unsupported by this library"). `passkey-client` (WebAuthn client side: builds `clientDataJSON`, checks origin) is useful for the desktop bridge (P5), not for Android where the OS already gives us `clientDataHash`.
* The traits are `async`; the core is synchronous. We run them with a tiny single-thread executor (no new runtime in the core).
* Licence MIT OR Apache-2.0 (compatible with the MIT repo). Bitwarden's and KeePassDX's Android apps are, to my knowledge, GPL (check each LICENSE): **read them for ideas, never copy code.** Google's `android/identity-samples` `CredentialProvider/MyVault` is the safest reference (check its LICENSE file before reusing any line).

### 5.2 Vault model

* New item type **`passkey`** (a login can *link* to its passkeys; a passkey item shows under the login that shares its site). Fields (all sealed like every other item, per-item sub-key):
  `rpId`, `rpName`, `userName`, `userDisplayName`, `userHandle` (bytes), `credentialId` (bytes), `privateKey` (**secret** field, never shown, never copied; only exported through the re-prompted export), `alg`, `signCount` (stays 0 for synced credentials),
  `flags` (BE/BS), `createdAt`, `lastUsedAt`, `prfSecret` (secret, only when the site used PRF), `transports`, optional `loginId`.
* **Compatibility trap (must be designed in P2, not found later):** payload `schema 1` is strictly checked and an unknown item type is an error on read. A vault that contains a passkey item and is opened by Hexis 0.4.0 must show
  "this item needs a newer Hexis" and must **never** drop or rewrite it during pull/merge/sync. Plan: bump to payload `schema 2` for passkey items only, add "unknown but preserved" handling in the reader and the D39 merge, and cover it with a test that syncs a 0.5 vault through a 0.4 core.
* Sync/merge (D39): private key, credential id, rp id are immutable; `lastUsedAt` merges by newest; deleting a passkey is an ordinary tombstone. A signature counter of 0 means two devices can never "diverge" on it.
* `reprompt` (D10) applies: a passkey item flagged re-prompt asks for the master password even inside the provider flow.
* Search/filters: add `rpId`, `userName` to the existing core-side `item_search` so the provider can match by site without a second index.

### 5.3 Core API (headless, usable without Tauri)

`passkey::create(request, caller) -> RegistrationResponse`, `passkey::sign(request, credential, caller) -> AssertionResponse`, `passkey::find(rp_id, allow_list)`, all pure functions over an unlocked vault, so they are testable with `cargo test` and later callable from JNI (P3) and from the desktop bridge (P5).

Because the Android provider can start the app process **without the Tauri window** (the system binds the service cold), `AppCore` construction must not depend on a Tauri `AppHandle`. Plan: `static CORE: OnceLock<Arc<AppCore>>`, built by whichever entry point comes first (Tauri `setup` or JNI `init(dataDir)`), and reused by the other. Spike S-2 proves this before P3 starts.

### 5.4 Tests (offline, in CI)

* Round-trip against `webauthn-rs` 0.6.2 as a **test-only relying party**: register with our authenticator, verify attestation (`none`), authenticate, verify signature, counter, flags, RP-ID hash, for ES256, EdDSA, ML-DSA (if the RP lib accepts it; otherwise our own verifier).
* Negative tests: wrong RP ID, wrong origin, `excludeCredentials` hit, unsupported algorithm, UV required but not done, locked vault, tampered credential id.
* PRF vectors, zeroization checks, "private key never appears in logs/IPC/search results" test (like the existing secret-redaction tests).
* A real-core e2e script in `e2e/` (pattern of `real-sync`): create passkeys, sync to the local S3 test server, pull on a second core, sign there.

### 5.5 Move passkeys in and out (P4)

* **Bitwarden JSON import** already exists: extend it to read `fido2Credentials` (key value is a PKCS#8 string).
* **FIDO Credential Exchange (CXF v1.0)** JSON import/export as a file (works on desktop and phone).
* **Same-device transfer on Android** with the Credential Transfer API (`providerevents`, **pre-release**, works from API 26): Hexis as *exporter* (registers an export entry, handles `androidx.identitycredentials.action.IMPORT_CREDENTIALS`
  with the secret entry id check and `requireContentUriPermissionFromCaller="write"`) and as *importer* (`ProviderEventsManager.importCredentials`). This is the clean path from Google Password Manager to Hexis once Google offers its passkeys for export. Whether your phone offers it: **UNVERIFIED**.
* Export always asks for the master password (like purge/exports today) and warns that the file contains private keys.

---

## 6. Android credential provider (P3, the high-risk phase)

### 6.1 Components

```
 website / app / browser            Android Credential Manager (system)                     Hexis (same app process)
 ---------------------------        ----------------------------------                     -------------------------------------
 navigator.credentials.get()  --->  asks every enabled provider            --->  HexisCredentialProviderService   (onBeginGet / onBeginCreate / onClear)
                                    shows ONE sheet with all entries                  returns only entries + a PendingIntent (no secrets)
 user taps the Hexis entry    --->  fires our PendingIntent                --->  PasskeyActivity (native Kotlin, translucent sheet)
                                                                                      unlock (if locked) -> verify user -> JNI -> Rust core
                                                                                      sign / create -> setResult(response JSON)
 gets the signed assertion    <---  returns it to the app/browser          <---  finish()
```

* `HexisCredentialProviderService : CredentialProviderService` with `android:permission="android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE"`, intent-filter
  `android.service.credentials.CredentialProviderService`, meta-data `android.credentials.provider` -> `res/xml/provider.xml` with capabilities
  `androidx.credentials.TYPE_PUBLIC_KEY_CREDENTIAL` (and `android.credentials.TYPE_PASSWORD_CREDENTIAL` as a cheap second step) and `settingsActivity`.
* **Disabled by default** (`android:enabled="false"`), enabled with `PackageManager.setComponentEnabledSetting` only when the person switches "Use Hexis for passkeys" on, exactly like the disguise aliases. So nobody sees Hexis in system settings unless they opted in
  (important for the disguise feature, see D-5). Enabling then opens `android.settings.CREDENTIAL_PROVIDER` / `CredentialManager.createSettingsPendingIntent()` because the person must still tick Hexis there.
* `PasskeyActivity`: not exported (system fires it through our own `PendingIntent`s, which must be `FLAG_MUTABLE`, not `FLAG_ONE_SHOT`, unique request code per entry), `excludeFromRecents`, own task affinity, translucent theme, `FLAG_SECURE`,
  overlay/tapjacking protection (`setHideOverlayWindows(true)` on API 31+, `filterTouchesWhenObscured`), no autofill on the password field. If `PendingIntentHandler` returns no request, it finishes immediately.
* **Why a native Kotlin sheet and not the Tauri web view:** `MainActivity` is `singleTask` and owns the only web view; launching it for a system sheet breaks the Credential Manager result flow, and a second Tauri activity in the same process is not supported. The sheet is tiny (pick account, unlock, confirm), so native views are fine.
* **Rust from Kotlin:** JNI exports in the same `libhexis_lib.so` (`jni` crate 0.22.4) with JSON in/out: `provider_init(dataDir)`, `provider_status()`, `provider_unlock(password)`, `provider_find(requestJson, caller)`, `provider_create(...)`, `provider_sign(...)`, `provider_lock()`. Kotlin gets only what it needs to render and to return; keys never leave Rust.
* Manifest/Kotlin/XML are written by `scripts/android_patch_manifest.py` from templates in `scripts/android/` (the existing pattern), with new tests in `scripts/tests/test_android_patch.py`. R8 keep rules for `androidx.credentials`.

### 6.2 Get flow (sign in)

1. `onBeginGetCredentialRequest`: if the vault is **locked** return one `AuthenticationAction` "Unlock Hexis" (it must not reveal which sites have passkeys). If unlocked: parse the request JSON, find credentials for `rpId` (+ `allowCredentials`), return one `PublicKeyCredentialEntry` per passkey (username, display name), each with its own `PendingIntent`.
2. `PasskeyActivity` (`retrieveProviderGetCredentialRequest`): unlock if needed, **verify the user** (6.3), call `provider_sign`, build the `PublicKeyCredential` JSON
   (`authenticatorData`, `signature`, `userHandle`, `clientDataJSON`), `setGetCredentialResponse`, `setResult(RESULT_OK)`.
3. Browsers (privileged callers): origin comes from `callingAppInfo.getOrigin(allowlistJson)`; then use the `clientDataHash` the system gives us and put a placeholder in `clientDataJSON` (Google's documented rule). Native apps: origin = `android:apk-key-hash:<base64url SHA-256 of signing cert>` and we build `clientDataJSON` ourselves.
4. Related Origin Requests: do **not** reject origin != rpId (Google's guide says this check wrongly breaks them).

### 6.3 User verification and unlock (decision D-1, the key security choice)

* Today's rule (D13): the fingerprint never wraps or caches a vault key; Stage 2 always needs the master password.
* Recommended default for passkeys, same rule: **locked vault -> master password** (typed in the native sheet; same throttle as the in-app unlock, so the provider is not a brute-force bypass). **Unlocked vault -> a fresh `BiometricPrompt`** (fingerprint or device screen lock) for each sign-in; that is the "UV" the site sees. No vault key is stored anywhere.
* Optional, off by default, opt-in with a clear warning (D-1 option 2): "Passkeys with fingerprint only". Passkey private keys are *additionally* wrapped by an Android Keystore key that needs biometrics for every use (StrongBox if present), so a passkey login does not need the master password while the app is locked. It is a deliberate exception to D13 and must be documented in `SECURITY.md`.

### 6.4 Create flow (save a new passkey)

`onBeginCreateCredentialRequest` -> one `CreateEntry` per vault/profile ("Save to Hexis: <vault name>"); `PasskeyActivity` -> unlock -> verify -> `provider_create` (honour `pubKeyCredParams` order, `excludeCredentials`, `residentKey`, PRF request) ->
`CreatePublicKeyCredentialResponse(registrationResponseJson)` (id, rawId, `authenticatorAttachment:"platform"`, `clientDataJSON`, `attestationObject` with `fmt:"none"`, `publicKeyAlgorithm`, `publicKey`, `transports:["internal","hybrid"]`,
`clientExtensionResults` with `credProps.rk=true`). The item is committed (and pushed by the normal sync trigger) **before** the response is returned, so a crash can never leave a registered credential without a stored key.
Passwords through the same service (`BeginCreatePasswordCredentialRequest` / `BeginGetPasswordOption`) reuse the existing login items: small, high value, a natural second step.

### 6.5 The QR / "scan with the phone" case you described

* The computer shows a QR code (`FIDO:/...`). Scanning it with the **system camera or Google Lens** opens Google Play services' hybrid flow, which asks Credential Manager, which shows the sheet with the enabled providers. Choosing Hexis runs section 6.2.
  **Source grade of this chain: secondary.** It comes from a search-result summary of a Corbado blog post (ref R-21, page not opened); I did not find it in Google's own documentation.
  Hexis implements **none** of the Bluetooth/tunnel protocol itself on Android; the system does. That also means this path depends on Google Play services and on the phone maker not blocking it.
* Reports say it works "on some devices" and fails on heavily customised ROMs and managed devices; I found **no official statement** that third-party providers are included in every hybrid flow. **UNVERIFIED on your phone** (T-9).
* Our own scanner (P1) forwards `FIDO:/` codes to the system, so Hexis becomes a single place to scan any QR code.
* If the phone does not allow any of this (HyperOS, Android < 14), the honest fallback is: use the passkey on the phone through Chrome's normal flow when Hexis is selectable; otherwise a desktop bridge (P5) or a password for that site. Implementing the hybrid protocol ourselves (BLE + the Google/Apple tunnel servers) is a large separate project and **is not part of this plan**.

### 6.6 Known platform problems to expect

* The device must report `android.software.credentials` (T-8). **Measured on your phone: `true`.** Some Android 14 phones do not (reported for Motorola, OnePlus, Nothing phones in a Bitwarden issue).
* **Xiaomi HyperOS 2/3** (**on your Redmi with HyperOS 3.0 / Android 16, Bitwarden's provider is registered and selected, so these reports did not reproduce here, section 3.0; they describe other phones**): users of KeePassDX and Bitwarden report a system pop-up asking to enable "Xiaomi Password Manager", passkey prompts not appearing, and "not supported with this device" on a 14 Ultra China ROM. Reported workarounds: switch *Settings > Language & input > Passwords & accounts > Preferred service* to another entry and back,
  open the provider settings through `com.android.settings/.Settings$AccountDashboardActivity`, or set `credential_service` / `credential_service_primary` with `adb shell settings put secure ...`. Some users say it works with Bitwarden on HyperOS 3.0. Everything here is community reports, **not verified by me**. Your phone's answer comes from P0.
* Where `settings put` pins another provider, it is an adb trick for testing only; Hexis must never write system settings.

### 6.7 Origin and app validation (privacy note: this is a new network use)

* Browsers: allowlist JSON (Google's `apps.json` as a **snapshot shipped inside the app**, refreshed with each release; no download at runtime, so no new network use). Hexis keeps its own copy under `src-tauri/assets/` with the fetch date.
* Native apps as callers: Google's guide says the provider should check the **Digital Asset Links** file of the site (`https://<rpId>/.well-known/assetlinks.json`) for the calling app's package and signing certificate. That is **one HTTPS request from Hexis to the website's own domain**, the first network traffic besides S3. Plan: only for native-app callers, only after the person has chosen the entry, https only, no redirects to other hosts,
  2 s timeout, 64 KB cap, no cookies, no identifying headers, result kept in memory only, switchable off (D-3) with the consequence stated. Update `PRIVACY.md` and `SECURITY.md`.

### 6.8 Desktop (P5; separate because Linux has no provider API yet)

| Option | What it is | Status/risk |
|---|---|---|
| **D1. Manage only** | See, search, delete, export passkeys in the Hexis desktop app (they sync from the phone) | Easy, ships with P2 |
| **D2. Browser extension + native messaging host** (Chrome/Edge and Firefox) | The extension replaces `navigator.credentials.create/get`, asks a small `hexis-host` binary, which asks Hexis; the prompt and unlock happen in the Hexis window. KeePassXC works this way. | Works today. Needs: an extension (MV3), the host binary, native-messaging manifests, **Firefox needs a signed add-on**, Chrome needs the Web Store or developer-mode sideload. Origin validated in the extension (top frame, Related Origin Requests). Owner steps in section 10 |
| D3. `credentialsd` provider API | The cross-desktop Linux passkey service (`linux-credentials/credentialsd`, v0.4.0 on 2026-11-05, from a search snippet, ref R-22) | **Third-party provider interface is still a draft** (issue #26). Revisit when it lands; no work now |
| D4. Virtual USB security key (`uhid`) | A fake FIDO2/U2F device the OS and any browser treat as a hardware key; the only route to **U2F/CTAP1 ("old") support** | Needs a `/dev/uhid` udev rule, an implementation of CTAPHID, shows up as "security key" in browsers. Optional; a spike decides |

---

## 7. Phases, order and rough size

(Sizes are working days of focused implementation, my estimate, not a promise.)

| Phase | Deliverable | Needs you | Size | Done when |
|---|---|---|---|---|
| **P0** (DONE 2026-11-25, section 3.0) | Diagnosis log + `has-feature` answer | Unlock phone, allow USB install, run section 3.1 | 0.5 d (mostly waiting) | We know why the camera fails and whether this phone can host a provider |
| **P1** (optional now, do after P3; section 3.0) | Native scanner, plugin removed, privacy exception removed | Test with a real QR code | 1-2 d | JVM decode tests green; your phone scans a TOTP QR; APK has no ML Kit/Play services entries (`apkanalyzer`) |
| **P2** | Passkey item, Rust engine, `schema 2` safety, search, desktop manage UI, Bitwarden passkey import | Decisions D-4, D-7 | 4-6 d | `webauthn-rs` round-trip tests green for all algorithms; old-core compatibility test green; e2e sync test green |
| **P3** | Android provider (create + get + unlock + verify + settings toggle + origin rules) | Phone with Android 14+ that supports providers; test accounts | 5-8 d | `adb` shows the service registered; **real passkey created and used on a test site, in Chrome, from a native app test, and via the QR/"phone" sign-in** (tests T-5 to T-11) |
| **P4** | CXF file import/export + Android Credential Transfer | Decision D-6 | 2-3 d | Round-trip of a CXF file; transfer with a second provider on a phone that offers it |
| **P5** | Desktop browser extension + host | Add-on signing / store account, D-2 | 6-10 d | Passkey created/used in Firefox and Chromium on Linux |
| **P6** | RS256/ES384, conditional create, RP signals, AAGUID decision, `/security-review` + `/security-audit`, docs, CHANGELOG, PRIVACY | none | 3-4 d | Audit findings closed or accepted in writing |

Spikes, before the phases they protect (each ends with a short written result):

* **S-1 (before P1):** can a CameraX+ZXing activity be started from a Tauri plugin and return a string? (Pattern exists in this repo: dialog and `SecureScreenPlugin`.)
* **S-2 (before P3):** headless core: `AppCore` built from JNI only, then reused by Tauri's `setup` in the same process; no deadlock, no second SQLite writer.
* **S-3 (before P3):** a minimal provider service that only returns a fake entry, installed on your phone: proves your ROM lists Hexis at all (this is the real answer to the HyperOS question).

Each Android-visible change also needs: `python3 scripts/android_patch_manifest.py ... --check`, the container installer tests only if the installer changes, `scripts/verify_all.sh` and the CI-equivalent checks **before** I hand you a push (your rule from 2026-11-25). UI parts follow `/frontend-design` and the existing Hexis design system.

---

## 8. Verification list (what turns UNVERIFIED into verified)

Needs your hands or phone: **T-1** camera scans a TOTP QR; **T-2** camera permission denied and denied-forever screens; **T-3** torch/zoom; **T-4** scan works with screen off/on cycles and with Back.
**T-5** Hexis appears in *Settings > Passwords, passkeys & autofill*; **T-6** create a passkey on a test site in Chrome (Android); **T-7** sign in with it; **T-8** `pm has-feature android.software.credentials` is true (**DONE: true on your phone**); **T-9** QR "use a phone" sign-in from a computer lists Hexis and completes;
**T-10** create/sign from a native app test (Digital Asset Links path); **T-11** sync: passkey created on the phone is usable after pull on the desktop core; **T-12** `FIDO:/` forwarded from our scanner opens the system flow (prerequisite measured: the phone resolves `FIDO:/` links to `com.google.android.gms/.fido.authenticator.ui.QRBounceActivity`; the end-to-end flow is still untested); **T-13** HyperOS toggle workaround (only if T-5 fails).
Offline test sites for me: a local RP (`webauthn-rs` example) on `localhost` with `adb reverse` so Chrome on the phone can use it without a real domain. Public test sites (for example `webauthn.io`) for you; I have not checked that they are online today.
An emulator is not recommended: this machine has little free RAM, and a known issue says passkeys on the stock Play emulator image need a Google sign-in (android/identity-samples issue #123, search snippet, ref R-23).

---

## 9. Decisions I need from you

| # | Question | My recommendation |
|---|---|---|
| **D-1** | Unlock/verification for passkeys: (1) master password when locked, fresh fingerprint/screen lock when unlocked, nothing cached; or (2) also an opt-in Keystore-wrapped passkey cache so a fingerprint alone works while the app is locked | **(1) now**, (2) later as an opt-in (it bends D13) |
| **D-2** | Desktop: which first, extension (D2), manage-only (D1), or wait for `credentialsd` (D3) | D1 with P2, D2 as P5 |
| **D-3** | Allow the one Digital Asset Links request to a site's own domain for native-app callers (switchable)? | Yes, on by default, switchable, documented |
| **D-4** | Passkey as its own item type linked to a login (vs. only a field inside a login) | Own item type + link |
| **D-5** | Provider is opt-in and hidden until enabled (so disguise users are unaffected) | Yes; the system settings will show the real app name, say so in the UI |
| **D-6** | Use the pre-release `providerevents` 1.0.0-beta01 for same-device transfer (no stable exists) | Yes, isolated behind one Kotlin file; CXF file import/export ships without it |
| **D-7** | AAGUID: all-zero (no vendor fingerprint) now, or register a Hexis AAGUID later (some sites show provider names and icons from a public list) | Zero now |
| **D-8** | Scanner: own ZXing screen (A) vs Google code scanner (B) vs keep the working plugin | **Changed after P0:** keep the plugin that works now; do A after the passkey work for the privacy reason (no ML Kit / Play services) |

---

## 10. What you need to do (owner checklist)

1. **Phone (done 2026-11-25 for diagnosis; needed again whenever an APK must be installed):** unlock it, Settings > Additional settings > Developer options: enable *USB debugging* and *Install via USB* (and *USB debugging (Security settings)* if shown), connect the cable, accept the "Allow USB debugging?" prompt. Then run section 3.1 and paste the output. (This is the step that failed on 2026-11-24 with `INSTALL_FAILED_USER_RESTRICTED`.)
2. Tell me which Android version and phone model, and whether you have a **second Android 14+ phone** that is not a Xiaomi (the safest way to prove P3 if HyperOS blocks providers).
3. Answer D-1 to D-8 (a one-line reply each is enough; "your recommendation" is a valid answer).
4. For P3 tests: a few throw-away accounts on sites that accept passkeys, and a computer for the QR test. Do not use accounts you cannot afford to lock yourself out of while we test.
5. For P5 only: decide whether the extension is **sideloaded** (free, developer mode; Firefox needs signing through an AMO account) or **published** (Chrome Web Store developer account, one-time fee; AMO listing). Publishing is your action; I will only prepare the build.
6. Give the go-ahead per phase. Nothing is committed or pushed by me; you review and commit.

---

## 11. Risks and honest limits

* **Provider support is up to the phone maker.** Hexis can be correct and still not be listed (HyperOS, some other ROMs, managed profiles). S-3 and T-8 find this out early and cheaply.
* **Android 13 and older cannot use Hexis for system passkey sign-in**, whatever we do.
* **The QR sign-in path depends on Google Play services** (system component); we cannot make it work on phones without them.
* **No attestation** means some enterprise/bank sites will refuse Hexis passkeys.
* **Private keys now live in the vault**: the same protection as passwords, but their theft is worse than a password leak (no rotation by the user). Hence: secret fields, never displayed, export only with the master password, zeroized in memory, and a security review in P6.
* **Old Hexis versions and `schema 2`** (section 5.2) is the main data-safety risk; it gets its own test before any release.
* **The pre-release `providerevents` library** can change; it is isolated and optional.
* Unknown until measured: whether ZXing reads damaged/low-light codes as well as ML Kit did (T-1; option D is the fallback), and whether Chrome on your phone routes to third-party providers by default.

---

## 12. Sources

* Android: [Integrate Credential Manager with your credential provider](https://developer.android.com/identity/sign-in/credential-provider) · [Credential transfer (CXF)](https://developer.android.com/identity/sign-in/credential-transfer) · [Credential Manager troubleshooting](https://developer.android.com/identity/sign-in/credential-manager-troubleshooting-guide) · [androidx.credentials releases](https://developer.android.com/jetpack/androidx/releases/credentials) · [CameraX releases](https://developer.android.com/jetpack/androidx/releases/camera) · [Google's privileged-apps allowlist](https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json) · [android/identity-samples](https://github.com/android/identity-samples)
* HyperOS reports: [KeePassDX #2220](https://github.com/Kunzisoft/KeePassDX/issues/2220) (opened and read) · [Xiaomi.eu thread](https://xiaomi.eu/community/threads/third-party-passkey-provider-does-not-work.75276/) (seen in search results only) · [Android-Credential-Provider-Fixer](https://github.com/aitiotekt/Android-Credential-Provider-Fixer) (README read) · [Bitwarden mobile #3253](https://github.com/bitwarden/mobile/issues/3253) (the source of the "`has-feature` returns false on Motorola/OnePlus/Nothing" report; seen in search results, **not opened by me**) · [Bitwarden forum: scanning the FIDO QR](https://community.bitwarden.com/t/please-add-a-unified-scan-qr-code-entry-point-to-bitwarden-for-android-ios/103610)
* Tauri scanner: [plugins-workspace #3560](https://github.com/tauri-apps/plugins-workspace/issues/3560) · [#2238](https://github.com/tauri-apps/plugins-workspace/issues/2238) · [Tauri barcode-scanner docs](https://v2.tauri.app/plugin/barcode-scanner/)
* Rust: [1Password/passkey-rs](https://github.com/1Password/passkey-rs) (read from the published 0.6.0 crates) · [webauthn-rs](https://crates.io/crates/webauthn-rs)
* Linux: [credentialsd](https://github.com/linux-credentials/credentialsd) · [third-party provider design, issue #26](https://github.com/linux-credentials/credentialsd/issues/26) · [FOSDEM 2026 slides](https://archive.fosdem.org/2026/events/attachments/838A8N-credentials-for-linux-bringing-passkeys-to-linux/slides/267430/fosdem_ravbuub.pdf)
* FIDO CXF 1.0: https://fidoalliance.org/specs/cx/cxf-v1.0-ps-20250814.html (linked from the Android transfer guide; not read in full)
