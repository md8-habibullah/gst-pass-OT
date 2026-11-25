# REFERENCES for the camera and passkey plan: what was read, how, and how to repeat it

Created 2026-11-25 by the session that wrote [`PLAN_NATIVE_CAMERA_AND_PASSKEYS.md`](PLAN_NATIVE_CAMERA_AND_PASSKEYS.md).
Purpose: a future session (or person) must be able to see, for every claim in the plan, **where it came from, how it was obtained,
how much it can be trusted, and how to check it again**. Nothing here is hidden behind "I read it somewhere".

Not published: neither this file nor the plan is listed in `scripts/build-site.mjs`, so the website does not include them.

## 0. How to use this file

1. Find the claim in the plan, find its `R-nn` reference here (the plan quotes `R-nn` where the grade is weak).
2. Look at the **grade**. Treat grade A and D as facts of 2026-11-25; B and C as leads that must be verified before building on them.
3. To re-check a fact, use the command or URL in the same row (section 6 has all commands in one block).
4. Re-run the version checks (section 3) on the day work starts. The numbers move, and the standing rule is "latest stable, read the real source".

### Grades

| Grade | Meaning |
|---|---|
| **A** | Page opened and read through WebFetch in this session (the fetch tool summarises the page with a small model, so quotes of exact code/commands should be re-read on the page before they are copied into source) |
| **D** | Measured by a command or read from a file on this machine (crate source, Maven metadata, JSON, repository file) |
| **B** | Seen only as a **search-result snippet or search summary**; the page itself was not opened. A lead, not a fact |
| **C** | My own background knowledge, not researched in this session. Must be verified |

## 1. Web pages

| ID | Source | Grade | What it established (used in plan section) |
|---|---|---|---|
| R-01 | https://developer.android.com/identity/sign-in/credential-provider | A (read in full) | Provider API is Android 14+; manifest entries (`BIND_CREDENTIAL_PROVIDER_SERVICE`, intent action `android.service.credentials.CredentialProviderService`, meta-data `android.credentials.provider`, capabilities `android.credentials.TYPE_PASSWORD_CREDENTIAL` and `androidx.credentials.TYPE_PUBLIC_KEY_CREDENTIAL`); two phases (begin/query, then selection by `PendingIntent`); `PendingIntent` must be `FLAG_MUTABLE`, not `FLAG_ONE_SHOT`, unique request code; `AuthenticationAction` for a locked provider; `PendingIntentHandler` calls; `settingsActivity`; settings intents; privileged-caller `getOrigin(allowlist)`, use `clientDataHash` and a placeholder `clientDataJSON`; Google's allowlist URL; **skip the origin-vs-RP-ID check (Related Origin Requests)**; DAL check for native callers; `createSettingsPendingIntent()`. (Sections 6.1-6.4, 6.7) |
| R-02 | https://developer.android.com/jetpack/androidx/releases/credentials | A (read in full) | `androidx.credentials` stable **1.6.0** (2026-04-08), alpha **1.7.0-alpha03** (2026-07-29); 1.2.0 introduced provider APIs; 1.5.0-alpha02 added `BiometricPromptData`; 1.6.0-alpha01 conditional create; 1.6.0-alpha05 credential-state signal APIs for relying parties. (Section 2, 6, P6) |
| R-03 | https://developer.android.com/identity/sign-in/credential-transfer | A (read in full) | Credential Transfer works on API 26+; artifact `androidx.credentials:providerevents` (the page's snippet says `1.0.0-alpha06`, Maven shows `1.0.0-beta01`); `registerExport`, `ExportEntry`, action `androidx.identitycredentials.action.IMPORT_CREDENTIALS`, `requireContentUriPermissionFromCaller="write"`, `ProviderEventsManager.importCredentials`; CXF v1.0; type list incl. passkey, basic-auth, totp, note. (Section 5.5, P4) |
| R-04 | https://developer.android.com/identity/sign-in/credential-manager-troubleshooting-guide | A (read in full) | Error table. Low relevance; confirms pre-Android-14 fallback needs `credentials-play-services-auth`. Says nothing about OEM blocks |
| R-05 | https://developer.android.com/jetpack/androidx/releases/camera | A **partial** (first 100,000 of 338,465 characters) | CameraX stable **1.7.0 released 2026-11-18**; 1.6.0 on 2026-03-25; 1.5.3 on 2026-01-28; minSdk 23 default since 1.5.0-rc01; 1.7.0 moved to a Java 11 target (may need desugaring). **The release date of 1.6.2 was not read.** (Section 2) |
| R-06 | https://github.com/Kunzisoft/KeePassDX/issues/2220 | A (read; via the fetch summariser) | Reports that on Xiaomi HyperOS 2/3 a third-party provider triggers a "turn on Xiaomi Password Manager" pop-up, passkey prompts fail, one user on a 14 Ultra (China, HyperOS 3.0.7) got "not supported with this device"; workarounds: toggle *Preferred service* away and back, open `com.android.settings/.Settings$AccountDashboardActivity`, `adb shell settings put secure credential_service ...`; Xposed module `io.github.howard20181.hyperpasskey`; a commenter cites Xiaomi docs saying HyperOS supports only Xiaomi's manager through Credential Manager (**not verified by me**); maintainer closed it as an OS issue (2025-10-24). (Section 6.6) |
| R-07 | https://github.com/aitiotekt/Android-Credential-Provider-Fixer | A (README read) | Third-party tool that pins a provider via adb; **the README names no settings keys**; says the original investigation involved Bitwarden on HyperOS. (Section 6.6; the setting names come from R-06, not from here) |
| R-08 | https://github.com/linux-credentials/credentialsd/issues/26 | A (read) | Third-party provider design for the Linux credentials daemon is a **draft**: `CredentialProvider` / `CredentialProviderManager` D-Bus interfaces, origin validation done by credentialsd, last comment 2026-11-01. (Section 6.8) |
| R-09 | https://community.bitwarden.com/t/please-add-a-unified-scan-qr-code-entry-point-to-bitwarden-for-android-ios/103610 | A (read) | The thread's author says scanning a cross-device QR code **can** invoke a third-party passkey provider "on some devices", unreliably; names Xiaomi/HyperOS, Huawei, Honor, OPPO, Vivo, Realme, Transsion ROMs and MDM work profiles as likely failures. **The fetch found no mention of `FIDO:/`, the system camera or Google Lens on this page**, and the author's claims are unverified and unsourced. A forum feature request, not an official statement. (Section 6.5) |
| R-10 | `https://crates.io/api/v1/crates/<name>` | D (curl) | Latest stable crate versions, section 3 |
| R-11 | `https://dl.google.com/android/maven2/<path>/maven-metadata.xml` | D (curl) | Latest Google Maven versions, section 3 |
| R-12 | `https://repo1.maven.org/maven2/com/google/zxing/core/maven-metadata.xml` | D (curl) | ZXing core 3.5.4 |
| R-13 | `https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json` | D (curl, hash in 4.3) | The 73-app privileged-browser allowlist |
| R-14 | `https://crates.io/api/v1/crates/<name>/<version>/download` | D (downloaded, hashes in 4.3, read by grep and by eye, never built) | The real source of `passkey-authenticator` 0.6.0, `passkey-types` 0.6.0, `passkey-crypto` 0.1.1 |
| R-16 | https://crates.io/api/v1/crates/passkey-transports | A | `passkey-transports` 0.2.0, "CTAP2 transport-specific bindings", repo 1Password/passkey-rs. (Used only to understand that `passkey-authenticator/src/linux.rs` is a **client** for USB keys, not a virtual device) |
| R-15 | `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-plugin-barcode-scanner-2.5.3/` | D (read locally) | The camera plugin's real code. See the line table in section 4. This is the strongest evidence for the camera findings |
| R-17 | tauri-apps/plugins-workspace issues #3560 and #2238 (`https://github.com/tauri-apps/plugins-workspace/issues/3560`, `.../2238`) | **B** | #3560: after `cancel()` the pending `scan()` promise is never settled (Android, plugin 2.4.5); #2238: "call scan but never returned". **Titles and summaries only**; open both pages before citing them as causes |
| R-18 | https://v2.tauri.app/plugin/barcode-scanner/ | **B** | Windowed mode makes the web view transparent and the UI must show what is underneath. A search-summary paraphrase |
| R-19 | https://xiaomi.eu/community/threads/third-party-passkey-provider-does-not-work.75276/ ; https://note.com/krem_noa/n/n9f30fc0c8d5a ; https://community.securid.com/s/article/Availability-of-Passkey-Feature-in-RSA-Authenticator-4-5-for-iOS-and-Android | **B** | Xiaomi forum report (Xiaomi 15 Ultra, xiaomi.eu ROM, creating works, signing in always uses Google); the RSA page gives the test command `adb shell pm has-feature android.software.credentials` and says the provider service relies on Google Play services (unavailable in China). Not opened |
| R-20 | https://github.com/bitwarden/mobile/issues/3253 | **B** | A search summary attributed to "a Bitwarden GitHub issue" the report that `has-feature android.software.credentials` returns false on Motorola Edge 40 / OnePlus / Nothing Phone 2. I am **not sure this is the issue that holds that report**; open it first |
| R-21 | https://www.corbado.com/blog/webauthn-passkey-qr-code | **B** | Source of the chain "the QR holds a `FIDO:/` link; scan it with the system camera or Google Lens; the OS handles it". Not opened, not found in Google's docs |
| R-22 | https://github.com/linux-credentials/credentialsd ; https://archive.fosdem.org/2026/events/attachments/838A8N-credentials-for-linux-bringing-passkeys-to-linux/slides/267430/fosdem_ravbuub.pdf | **B** | Release 0.4.0 on 2026-11-05; reference UI in GTK4; experimental extension for Firefox 140+ and Chrome/Edge 111+; Firefox upstream support in review (Mozilla bug 1983767). Snippets only |
| R-23 | https://github.com/android/identity-samples (`CredentialProvider/MyVault`) ; issue #123 | **B** | The official sample provider app exists (the **licence was not confirmed**); issue #123: passkeys do not work in the stock Play emulator image until a Google account is signed in |
| R-24 | heise.de "Password managers on Android: Switch without manual export"; xenospectrum.com credential-transfer article; corbado.com "Android 17 Passkeys" | **B** | Leads only: Google rolling out passkey import/export through Credential Manager, one article claiming Play services 26.21 from June 2026. **Not used as a fact in the plan** |
| R-25 | docs.rs/crate/passkey, 1password.com/blog/passkey-crates | **B** | Superseded by R-10 and R-14 (the real crate files) |
| R-26 | https://developer.android.com/identity/credential-manager/prerequisites | **B** | Digital Asset Links prerequisite for calling apps. A snippet said the passkey relation is `get_login_creds` but the page's own snippet showed `handle_all_urls`: **the exact relation strings must be read from the page before implementing the DAL check** |
| R-27 | Background knowledge | **C** | ZXing is Apache-2.0 and pure Java; Chrome Web Store needs a one-time developer fee; Firefox release channels need a signed add-on; Bitwarden's and KeePassDX's Android apps are GPL; KeePassXC integrates through browser extension + native messaging; Chrome on Android can route passkeys to third-party providers. **None of these was researched** |

## 2. To read before building (not read yet)

These are the primary sources the implementation must follow. They were **not** opened in the research session.

| Needed for | Source |
|---|---|
| P3 | `CredentialProviderService`, `PendingIntentHandler`, `CallingAppInfo.getOrigin`, `BeginGetCredentialResponse`, entry classes in the androidx reference: https://developer.android.com/reference/androidx/credentials/provider/CredentialProviderService |
| P3 | Google's sample provider `CredentialProvider/MyVault` in https://github.com/android/identity-samples (read its LICENSE first) |
| P3 | Digital Asset Links statement list spec: https://developers.google.com/digital-asset-links/v1/statements (relation strings, matching rules) |
| P2/P3 | W3C WebAuthn Level 3 (authenticator data flags BE/BS, `clientDataJSON`, `attestationObject`, extensions `credProps`, `prf`): https://www.w3.org/TR/webauthn-3/ |
| P2 | FIDO CTAP 2.2 (hybrid transport, `hmac-secret`): https://fidoalliance.org/specs/fido-v2.2-ps-20250714/ (URL from memory, check) |
| P4 | FIDO Credential Exchange Format 1.0: https://fidoalliance.org/specs/cx/cxf-v1.0-ps-20250814.html (URL taken from the Android transfer guide, R-03; not opened) |
| P6 | AAGUID list of known providers: https://github.com/passkeydeveloper/passkey-authenticator-aaguids (from memory, check) |
| P1 | ZXing `QRCodeReader`/`InvertedLuminanceSource` API (Maven 3.5.4 sources) and CameraX `ImageAnalysis` docs |
| P5 | Chrome/Firefox native messaging manifest locations and MV3 `world: "MAIN"` content scripts; KeePassXC-browser source for the model (read for ideas only) |

## 3. Versions looked up on 2026-11-25 (commands in section 6)

| Item | Result | Source / method |
|---|---|---|
| crates.io | `passkey` 0.6.0, `passkey-authenticator` 0.6.0, `passkey-types` 0.6.0, `passkey-client` 0.6.0 (all updated 2026-10-01); `passkey-crypto` 0.1.1 (2026-10-02); `passkey-transports` 0.2.0; `p256` 0.14.0; `p384` 0.14.0; `ed25519-dalek` 3.0.0; `rsa` 0.10.0 (2026-11-08); `ciborium` 0.2.2 (2024-01-24, old); `coset` 0.4.2; `minicbor` 2.3.1; `jni` 0.22.4; `ctap-hid-fido2` 3.6.1; `sha2` 0.11.1; `ecdsa` 0.17.0; `webauthn-rs` 0.6.2; `webauthn-rs-core` 0.6.2; `webauthn-authenticator-rs` 0.6.2; `ml-dsa` 0.1.2; `uhid-virt` 0.0.8 (2025-01-01) | D: `https://crates.io/api/v1/crates/<name>` (`max_stable_version`) |
| Google Maven | `androidx.camera:camera-core` 1.6.1, 1.6.2, **1.7.0** (stable list tail); `com.google.mlkit:barcode-scanning` 17.3.0; `androidx.biometric:biometric` latest **1.4.0-rc01**, latest stable **1.1.0**; `androidx.credentials:credentials` 1.6.0 and 1.7.0-alpha01..03; `androidx.credentials.providerevents:providerevents` 1.0.0-alpha04..06, **1.0.0-beta01**; `play-services-code-scanner` 16.1.0 | D: `https://dl.google.com/android/maven2/<group path>/<artifact>/maven-metadata.xml` |
| Maven Central | `com.google.zxing:core` 3.5.4 (release date **not** checked) | D: `https://repo1.maven.org/maven2/com/google/zxing/core/maven-metadata.xml` |
| Google browser allowlist | 73 apps; includes Chrome (several channels), Firefox family, Brave, Edge, Vivaldi, Samsung Internet, Yandex, GMS (`com.google.android.gms`, 4 signatures), Citrix Receiver | D: `curl https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json` |

Rule applied: 2-week-old minimum for new libraries is why the plan says CameraX 1.6.2 now and 1.7.0 after 2026-12-02 (1.7.0 is dated 2026-11-18). The age of 1.6.2 and ZXing 3.5.4 must be checked on the day.

## 4. Facts read from this repository and from downloaded sources

### 4.1 Repository (paths relative to the project root)

| Fact | Where |
|---|---|
| Camera call uses `scan({ windowed: true, formats: [QRCode] })` | `src/lib/scan.ts` (`scanWithCamera`) |
| Page goes transparent while scanning | `src/styles/base.css:195-204`, `src/components/scan/ScanOverlay.tsx` |
| Plugin registered for Android/iOS only | `src-tauri/src/lib.rs:27-31`, `src-tauri/Cargo.toml` (target-specific section) |
| Scanner capability list | `ai-docs/BUILD_TARGETS.md` §7.2 (I did not print `src-tauri/capabilities/mobile.json` itself) |
| ML Kit dependency, keep rules, privacy exception, "never verified on a device" | `ai-docs/BUILD_TARGETS.md` §4.4-4.6 and §7.2, `scripts/android/hexis.pro`, `PRIVACY.md:49-63`, `ai-docs/CHANGELOG_AND_STATE.md` (~lines 180-186, 352-356, 500-501) |
| `MainActivity` is `launchMode="singleTask"`; 12 disabled/enabled launcher aliases | `src-tauri/gen/android/app/src/main/AndroidManifest.xml` (line 11 and following) |
| `compileSdk 37`, `minSdk 28`, `targetSdk 37` | `src-tauri/gen/android/app/build.gradle.kts:18,23,24` |
| `MainActivity` content (edge-to-edge, keyboard padding, Back handling, recents blank) | `src-tauri/gen/android/app/src/main/java/dev/habibullah/hexis/MainActivity.kt` (generated from `scripts/android/MainActivity.kt.tmpl`) |
| Item types and strict payload schema | `src-tauri/src/storage/model.rs:31` (`enum ItemType`), `:558` (`if self.schema != SCHEMA`) |
| Core built inside Tauri `setup` via `commands::prepare_state`; data dir from `app.path().app_data_dir()` | `src-tauri/src/lib.rs` (setup closure), `src-tauri/src/storage/app.rs` (`AppCore`) |
| Decisions that bind the work | `ai-docs/ARCHITECTURE.md` D10, D13, D39 |
| The website builds only the files listed in the `PAGES` table | `scripts/build-site.mjs:44-76` |
| Phone not attached during the first research pass (`adb devices` printed an empty list; I started the adb server as a side effect). **It was attached later the same day: measurements in section 10** | session transcript, 2026-11-25 |
| Earlier phone facts (Xiaomi/HyperOS, serial, install refusal) | memory note `hexis-phone-qa-round`; `ai-docs/BUILD_TARGETS.md` §4.4 |

### 4.2 The camera plugin source (R-15)

File: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-plugin-barcode-scanner-2.5.3/android/src/main/java/BarcodeScannerPlugin.kt`

| Lines | What |
|---|---|
| 124-172 | `setupCamera`: adds a `PreviewView` and `GraphicOverlay` to the web view's parent |
| 146-150 | windowed: `webView.bringToFront()`, remembers the old background, `webView.setBackgroundColor(Color.TRANSPARENT)` |
| 160-170 | `ProcessCameraProvider` listener: `InterruptedException` and `ExecutionException` are caught and **ignored** (comment "ignored") |
| 183 | `setTargetResolution(Size(1280, 720))` |
| 191-199 | `bindToLifecycle(...)`; `catch (e: Exception) { // TODO }` at line 198: **a failed camera bind produces no error** |
| 338-341 | `cancel()`: rejects the saved invoke with "cancelled" (see R-17 for the reported ordering problem) |
| 347-361 | `scan()`: throws if the permission is not granted; calls `prepare(...)` then `configureCamera(...)`; the promise settles only on a scan or on cancel |
| `build.gradle.kts` | CameraX 1.5.1, `play-services-mlkit-barcode-scanning:18.1.0`, `minSdk 24`, `compileSdk 36` |

### 4.3 The passkey crates (downloaded, read, never built or run)

Location: `~/.cache/hexis-scratch/research/pk.avSJsF/` (outside the repo, outside `/tmp`; may be deleted, re-download with section 6).
Each `.crate` was extracted into its own new folder with `tar -xzf <file> -C <folder> --strip-components=1`; no build script, test or example was executed.

| File | SHA-256 |
|---|---|
| `passkey-authenticator.crate` (0.6.0) | `5fb7e1cb79a932ac3a5ceb0fa73b25a6028ef1bd69a007441e6cce9ecf2e6e0f` |
| `passkey-types.crate` (0.6.0) | `f4ed76f185c29d0d0c226ecf653531bb962e63eff3bf6d5cd5062288116138d2` |
| `passkey-crypto.crate` (0.1.1) | `e5108c58974bacdae414ee66edf0c009818509ccf6be4a911032147336e3da27` |
| `../apps.json` (Google allowlist, fetched 2026-11-25) | `0891ee2507894999ead3eb4f42e17936a2e8ed8f8fd95592535c9811ef319481` |

(`passkey-client` and `passkey-transports` were **not** downloaded or read.)

| Claim in the plan | Evidence (path inside the crate) |
|---|---|
| The authenticator is generic over a store we implement | `passkey-authenticator/src/credential_store.rs:50-80` (`trait CredentialStore`: `find_credentials`, `save_credential`, `update_credential`, `get_info`), `passkey-authenticator/src/passkey.rs:14` (`trait PasskeyAccessor`) |
| The traits are async | `#[async_trait::async_trait]` at `credential_store.rs:49`; `async-trait` and `tokio` (optional) in `Cargo.toml` |
| Default crypto backend is pure Rust; `aws-lc-rs` (C) is optional | `passkey-crypto/Cargo.toml` `[features]`: `default = ["rust-crypto"]`; `rust-crypto` = ed25519-dalek, hmac, ml-dsa, p256, sha2, signature, zeroize, rand |
| Algorithms implemented: ES256, ESP256, EdDSA, Ed25519, ML-DSA-44/65/87 | `grep -rhoE "iana::Algorithm::[A-Za-z0-9_]+" passkey-crypto/src --include=mod.rs`; test list at `passkey-crypto/src/tests.rs:422-442` |
| **RS256 is not implemented** | grep for `RS256|rsassa|RSA` in crypto, authenticator and types found only a doc comment in `passkey-types/src/webauthn/attestation.rs:87,95` and an unrelated test constant |
| No U2F protocol code | grep for `u2f`: only the attestation-format enum value `FidoU2f` (`attestation.rs:538-539`) and a doc comment |
| `largeBlobKey` unsupported | `passkey-types/src/ctap2/make_credential.rs:311` ("currently unsupported by this library"); `large_blob_key: None` in the authenticator's `make_credential.rs:175` and `get_assertion.rs:159` |
| Attestation is `none` | `passkey-authenticator/src/authenticator/make_credential.rs:171` (`fmt: "none".into()`) |
| PRF / hmac-secret supported | `passkey-authenticator/src/authenticator/extensions.rs` and `extensions/hmac_secret.rs` |
| `passkey-authenticator/src/linux.rs` is a **client for USB security keys** (hidraw), not a virtual authenticator | file header lines 1-20 |

## 5. What each weak claim needs to become a fact

| Weak claim (plan section) | Grade | How to verify |
|---|---|---|
| Camera does not open because of X (3.1) | **superseded**: not reproduced, section 10 | Measured: the camera opens on 0.3.0. Only whether the preview is visible and the QR code decoded is still unverified (needs the user's eyes) |
| The plugin's windowed mode shows black/blank on opaque pages (3.1) | B | Open R-18 and R-17; reproduce with the P0 log |
| Hexis can be listed as a provider on the user's phone (6.6) | **partly measured**: feature flag true, Bitwarden is registered and selected (section 10) | Spike S-3 (a service that returns a fake entry) and test T-5 still prove that **Hexis** appears |
| HyperOS blocks third-party providers (6.6) | B (R-06) **contradicted on this phone** by the measurement in section 10 | Not a general statement: other HyperOS builds/regions may differ |
| Scanning the computer's QR with the system camera reaches the provider sheet (6.5) | B (R-21) | Test T-9 on a real phone with a real site |
| Our scanner can hand `FIDO:/` to the system with `ACTION_VIEW` (3.2) | **prerequisite measured** (the phone resolves `FIDO:/` to `com.google.android.gms/.fido.authenticator.ui.QRBounceActivity`, section 10); the end-to-end flow is not | Test T-12 with a real QR code from a computer |
| DAL relation strings for the passkey case (6.7) | B (R-26) | Read the DAL spec and the prerequisites page (section 2 above) before coding |
| `providerevents` works on the user's phone (5.5) | none | Spike in P4 on a phone that offers an export source |
| Chrome on Android routes passkeys to third-party providers by default (11) | C | T-6 with Chrome |
| ZXing reads codes as well as ML Kit (3.2, 11) | none | JVM decode tests, then T-1 with the real camera; option D in plan §3.2 is the fallback |
| Extension/store/signing rules for Firefox and Chrome (6.8, 10) | C | Check the Mozilla and Chrome developer docs when P5 starts |

## 6. Commands that reproduce the research

Run from any directory; they only read from the network and write into folders you name. They need no `sudo`.

```bash
# --- versions (section 3) -------------------------------------------------
for c in passkey passkey-authenticator passkey-types passkey-client passkey-crypto passkey-transports \
         p256 p384 ed25519-dalek rsa coset minicbor jni sha2 ecdsa webauthn-rs ml-dsa uhid-virt; do
  printf "%-26s" "$c"
  curl -s -A "version-check (generic client)" "https://crates.io/api/v1/crates/$c" |
    python3 -I -c 'import sys,json; d=json.load(sys.stdin)["crate"]; print(d["max_stable_version"], "| updated:", d["updated_at"][:10])'
  sleep 1                                # crates.io asks for at most one request per second
done

M=https://dl.google.com/android/maven2
for a in androidx/camera/camera-core com/google/mlkit/barcode-scanning androidx/biometric/biometric \
         androidx/credentials/credentials androidx/credentials/providerevents/providerevents \
         com/google/android/gms/play-services-code-scanner; do
  echo "== $a"; curl -s "$M/$a/maven-metadata.xml" | grep -o "<version>[^<]*</version>" | sed 's/<[^>]*>//g' | tail -5 | tr '\n' ' '; echo
done
curl -s https://repo1.maven.org/maven2/com/google/zxing/core/maven-metadata.xml | grep -E "<latest>|<release>"

# --- Google's privileged-browser allowlist (snapshot to ship, plan §6.7) ------
curl -s https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json -o apps.json && sha256sum apps.json

# --- the passkey crates (never extract into the repo; use a fresh folder; never run their code) ---
W=$(mktemp -d ~/.cache/hexis-scratch/research/pk.XXXXXX); cd "$W"
for c in passkey-authenticator:0.6.0 passkey-types:0.6.0 passkey-crypto:0.1.1; do
  n=${c%%:*}; v=${c##*:}
  curl -sL -A "version-check (generic client)" "https://crates.io/api/v1/crates/$n/$v/download" -o "$n.crate"
  mkdir "$n" && tar -xzf "$n.crate" -C "$n" --strip-components=1
done
sha256sum ./*.crate              # compare with section 4.3

# --- the claims checked by grep (section 4.3) -----------------------------
grep -rniE "RS256|rsassa|\bRSA\b" passkey-crypto/src passkey-authenticator/src passkey-types/src
grep -rniE "u2f" passkey-types/src passkey-authenticator/src passkey-crypto/src
grep -rhoE "iana::Algorithm::[A-Za-z0-9_]+" passkey-crypto/src --include=mod.rs | sort -u

# --- the camera plugin (section 4.2) ----------------------------------------
P=~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tauri-plugin-barcode-scanner-2.5.3/android/src/main/java/BarcodeScannerPlugin.kt
grep -n "TODO\|ExecutionException\|setBackgroundColor(Color.TRANSPARENT)\|bringToFront\|setTargetResolution" "$P"

# --- the phone (needs the owner at the phone; plan §3.1 has the full list) ----
adb shell pm has-feature android.software.credentials
```

## 7. Search queries used (so the search can be repeated)

All with the WebSearch tool on 2026-11-25; results are snippets (grade B) unless a page was then opened (section 1).

1. `third-party credential provider passkey hybrid cross-device QR code scan Android Credential Manager Bitwarden 1Password`
2. `androidx.credentials latest stable release 2026 credentials-provider CredentialProviderService`
3. `Android Credential Manager import export passkeys CXP CXF ProviderImportCredentialsRequest credential provider implement`
4. `passkey-rs 1Password crate passkey-authenticator passkey-types crates.io latest version`
5. `Linux passkey credentialsd D-Bus platform API Firefox Chrome GNOME 2026 third-party credential manager`
6. `Android passkey provider "hybrid" QR code scan phone camera third-party provider Bitwarden "scan the QR code" passkey Android 14 select Bitwarden`
7. `"android.software.credentials" feature Xiaomi HyperOS MIUI Credential Manager third-party passkey provider not available`
8. `Android Credential Manager third-party credential provider requirements Digital Asset Links validation calling app provider responsibility assetlinks passkey native app`
9. `tauri-plugin-barcode-scanner android windowed black screen camera not showing transparent webview issue plugins-workspace`
10. `android identity-samples CredentialProvider sample app Apache passkey credential provider github`

## 8. Corrections made while writing (so nobody repeats the over-claims)

* The first draft of the plan said the QR-to-Credential-Manager chain as plain fact; it is a secondary-source claim (R-21) and is now labelled so (plan §6.5).
* The first draft said "Tauri's docs warn about exactly this" (opaque page background); only a search paraphrase was seen (R-18). Relabelled (plan §3.1).
* The first draft said the 1Password crate "dropped U2F in 0.6" (from a changelog snippet). The crate source shows there is simply no U2F protocol code; the claim now rests on grep (section 4.3).
* The first draft said RS256 was "not in the crate"; it exists as a doc comment only. Now stated precisely.
* The `has-feature ... false` report on Motorola/OnePlus/Nothing phones is attributed to Bitwarden issue #3253 only as a probable source (R-20).

## 9. Hygiene notes

* The research folder `~/.cache/hexis-scratch/research/` holds only public files (the allowlist JSON and three public crates). No vault data, no personal data, and no phone output was stored there.
* No library was added to the project, no `Cargo.toml`, `package.json` or Gradle file was changed, and no source file was edited during this research. Only the two documents in `ai-docs/` were created.
* The adb server was started once to look for a phone. When the phone was attached later (section 10) only **read-only** commands were run (`getprop`, `pm`, `dumpsys`, `settings get`, `cmd package query-services` / `resolve-activity`, `cmd appops get`, `logcat -d`). Nothing was installed, no setting was changed, no input or tap was sent, and the device log was read through filters only. The phone's serial number is deliberately not written into repository documents.

## 10. On-device measurements (phone attached, 2026-11-25 about 11:40 to 11:45, UTC+6)

Grade **D** (measured by command on the owner's phone). The phone's serial number is not recorded here. All commands were read-only (section 9).
The tool shell is zsh: wrap `adb -s <serial> shell "$@"` in a function; a variable holding the whole command does not word-split and fails with "command not found".

| What | Command | Result |
|---|---|---|
| Device | `adb devices -l`; `getprop ro.product.model`, `ro.product.brand`, `ro.build.version.release`, `ro.build.version.sdk`, `ro.build.version.security_patch`, `ro.mi.os.version.name`, `ro.mi.os.version.code`, `ro.miui.ui.version.name`, `ro.miui.region` | Model `25062RN2DA`, brand Redmi, product `creek_global`; Android 16, SDK 36; patch 2026-09-01; HyperOS `OS3.0` (code 3, `V816`); region property `CH` |
| Provider feature | `pm has-feature android.software.credentials` | `true` |
| Registered providers | `cmd package query-services --brief -a android.service.credentials.CredentialProviderService` | 2 services: `com.google.android.gms/.auth.api.credentials.credman.service.PasswordAndPasskeyService` and `com.x8bit.bitwarden/.Autofill.CredentialProviderService` |
| Selected provider | `settings get secure credential_service` / `credential_service_primary` / `autofill_service` | Bitwarden / Bitwarden / `null` |
| Play services | `pm list packages`; `dumpsys package com.google.android.gms` | `com.google.android.gms` and `com.android.vending` present; versionName `26.45.31`. No Xiaomi password-manager package matched `passwordmanager|passkey` |
| `FIDO:/` handler | `cmd package resolve-activity --brief -a android.intent.action.VIEW -d "FIDO:/123456"` | `com.google.android.gms/.fido.authenticator.ui.QRBounceActivity` |
| Credential service dump | `dumpsys credential` | printed nothing (not usable as evidence either way) |
| Installed Hexis | `dumpsys package dev.habibullah.hexis` | versionName `0.3.0`, versionCode 3000, minSdk 28, targetSdk 37, installed 2026-11-25 03:19:34 by `com.google.android.packageinstaller`; permissions INTERNET, CAMERA, USE_BIOMETRIC |
| Camera permission | same dump; `cmd appops get dev.habibullah.hexis CAMERA` | `CAMERA: granted=true`; app-ops mode `allow`, "foreground" |
| Camera history | `dumpsys media.camera` (client events); `logcat -d` filtered on `hexis` | Hexis `CONNECT`/`DISCONNECT` camera 0 at 03:41:22-03:41:26, 11:40:30-11:40:35 and 11:40:38-11:40:47. Log: `Camera 0: Opened. Client: dev.habibullah.hexis`, CameraX threads (`CameraX-core_camera_1`), `Open camera top activityName is dev.habibullah.hexis.MainActivity`. Other camera users in the same window: `com.miui.face` (face unlock) and `com.google.android.googlequicksearchbox` |
| Crashes | `logcat -b crash -d`; `dumpsys dropbox` | `data_app_crash` records 2026-11-24 15:45:12 and 15:48:17: `NullPointerException ... at app.tauri.barcodescanner.BarcodeScannerPlugin.a` (the v0.1.0 ML Kit R8 bug, fixed by the keep rules). Several `pthread_mutex_lock called on a destroyed mutex` aborts on 2026-11-24 and 2026-11-25 03:28 (the Back-key bug, fixed by `moveTaskToBack`). **No crash after 03:28** |

Interpretation (also in plan §3.0):

* Camera: not reproduced as "does not open"; Camera2 device 0 is opened and closed cleanly by Hexis 0.3.0. **Unverified:** preview visibility and QR decoding (no log line carries that).
* The phone runs **0.3.0**, the repository is **0.4.0**; anything tested on the phone says nothing about unreleased code.
* Passkeys: the HyperOS 3.0 / Android 16 phone supports third-party providers (Bitwarden is registered and selected). It does not prove that Hexis will be listed; spike S-3 does.
* Region property `CH` with a global build (`creek_global`) and Play services present: nothing here says the ROM lacks Google services.
* This single phone is one data point; it does not prove anything about other HyperOS versions (R-06 reports are for HyperOS 2 and for a China-region 14 Ultra).
