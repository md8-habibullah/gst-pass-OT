# PASSKEYS — how Hexis stores and uses passkeys, and the scanner that fills them in

Status 2026-11-25 (branch `feature/passkeys-and-smart-scanner`, not yet merged). The plan this follows is
`PLAN_NATIVE_CAMERA_AND_PASSKEYS.md`; its recommended decisions D-1 … D-8 were taken as approved. The research behind the
plan is in `REFERENCES_CAMERA_AND_PASSKEYS.md`. Decisions are numbered in `ARCHITECTURE.md` §8 (D47 onward).

## 1. What exists, and what does not

| Part | State |
|---|---|
| A passkey is a field of a login (`passkey`, `passkey#2` …, at most 10) | built, tested in Rust and in the browser preview |
| Own WebAuthn authenticator (ES256, `none` attestation) in the Rust core | built, round-trip tested against an independent verifier |
| Android Credential Manager provider (Android 14+): list, sign in, create | built; Kotlin compiles and R8 minifies; **not run on a phone** |
| Settings > Passkeys (switch the provider on, website check on/off, open the system list) | built; the Android half is unverified on a phone |
| Camera button and smart scanner (one-time codes, passkey codes, links, text) | built, tested in the preview and with the real core |
| Bitwarden import and export of `fido2Credentials` | built, tested |
| Desktop passkeys (a browser extension or `credentialsd`) | **not built** (P5) |
| Other algorithms (RS256, EdDSA), PRF / hmac-secret, large blob | **not built** |
| FIDO Credential Exchange (CXF) and the Android Credential Transfer API | **not built** |
| Using a phone passkey for a desktop sign-in by scanning the browser's `FIDO:/` code | **not supported by Hexis itself**; the scanner recognises such a code and says to use the system camera (see §6) |

## 2. How a passkey is stored (D47)

A passkey is **not** a new item type. It is a field of a login, next to the usernames, passwords and one-time codes of D46:

* keys `passkey`, `passkey#2`, `passkey#3` … `passkey#10` (canonical spelling only, like `totp#n`); `FieldKind::Passkey`, always secret;
* the value is one JSON record (`passkey::record::PasskeyRecord`):
  `{ "v":1, "rpId", "rpName", "credentialId" (base64url), "userHandle" (base64url), "userName", "displayName", "alg":-7, "privateKey" (PKCS#8, base64url), "createdAt" }`;
* nothing changes in the payload schema (stored version stays 1) and nothing in the file format. **Caveat:** a build from before passkeys (0.4.0 and older) cannot save an edit to a login that
  holds one (it refuses the unknown field), and its pull-merge skips such a login instead of merging it (details in `STORAGE_AND_SYNC.md`, "Passkeys in a vault"). Update every device before adding passkeys; there is no
  minimum-version check between devices yet.

Rules the core enforces (all with tests):

* **The private key never leaves the core.** Views answer `FieldView.passkey` (`PasskeyInfo`: site, name, user, created, credential id prefix; no key). The editor receives a
  marker `{"keep":true, …info}` and `item_update` restores the stored record by `credentialId`. A marker for a credential the item does not hold is refused.
* `item_create` refuses passkey fields (`invalid_input`): a passkey is made only by a website's request (provider) or by an import.
* Copying an item (clone, copy to another account) strips its passkeys: a clone must not become a second holder of the same credential.
* `normalize` validates every record (`PasskeyRecord::parse`): rp id, lengths, algorithm, a key that parses as P-256.
* **Merge** (D39) treats the passkeys of a login as a set by `credentialId` (never "newer wins", never field by field): both sides' additions are kept, a removal by one side stands, and a slot taken by another credential moves to the next free one. An import that lands in an occupied slot goes to the next free slot, never over a passkey.
* Search finds a login by the site and the user name of its passkeys (`vault_search`), without opening the key.
* Export (plain-text, re-prompted): see §7.

## 3. The authenticator (`src-tauri/src/passkey/`)

An own, small WebAuthn authenticator, ES256 only (`p256` 0.14 with `pkcs8`; the byte layouts around it, CBOR and authenticator data, are written in `src/passkey/`). The plan's first idea, using
1Password's `passkey` crates, was not taken: they are heavier, async, and clashed on versions with the rest of the core, and ES256 is what every site accepts. No PRF, no RS256/EdDSA
creation (a documented limit).

* Registration: `none` attestation, all-zero AAGUID, signature counter 0 (synced credentials never diverge on it), flags `UP | UV? | BE | BS | AT`
  (UV only when the person was verified; BE and BS because the key is synced between devices).
* Assertion: signature over `authenticatorData || clientDataHash`, DER encoded.
* `excludeCredentials` is honoured (`passkey_excluded`); an algorithm list without `-7` answers `passkey_unsupported`.
* Caller kinds (`Caller`): **Browser** (the system names the origin and hands over the hash of the client data, only for browsers on Google's privileged list),
  **App** (no web origin; the website must list the app, see §5), **Own** (the app's own window; used by tests and reserved for the desktop bridge).
* `origin.rs` holds the rules for a caller's right to a website name: `rp_matches_origin` (the origin must be `https`, or `http` for `localhost`, carry no credentials, and end in the rp id; it does not know the
  public-suffix list, because a browser, which does, has checked before it asks) and the Digital Asset Links fetch and parse.

## 4. The provider (`src-tauri/src/provider/`, `scripts/android/*.kt`)

```text
system (Credential Manager)
   │ binds, API 34+
   ▼
HexisCredentialProviderService.kt   lists entries, builds the sheet's PendingIntents (never runs by itself)
   │ one JSON call
   ▼
ProviderNative.kt ──JNI──▶ provider/jni.rs ──▶ Provider::call(op, args)  ──▶ the same AppCore the app uses
                                                   state · unlock · lock · getOptions · sign · createOptions · create
PasskeyActivity.kt  the sheet: login choice, master password, fingerprint (BiometricPrompt), "Save here"
```

* **One entry**, `Provider::call(op, json) -> json`: `{"ok":true,"data":…}` or `{"ok":false,"code","message"}`. A panic is caught and becomes an error; nothing crosses JNI as an exception.
* **The app lock refuses everything.** While Stage 1 (app lock) is closed every op answers `app_locked`; the provider is no way around it.
* **D-1, how a vault opens here.** A locked vault opens with the master password only (a fingerprint never opens a vault, D13). With the vault open, each sign-in or new
  passkey needs a fresh `BiometricPrompt` (or screen lock), reported by Kotlin as `userVerified`, unless the master password was typed in the sheet for this request. Rust writes UV into the
  answer and refuses a request that requires UV when it is missing. A phone with neither a fingerprint nor a screen lock set up fails closed: the sheet asks for the master password for every request (found by the security review, `PasskeyActivity.askWithoutLock`). A login that asks for the master password (re-prompt) or a vault opened with the account PIN answers
  `password_required`, and the sheet asks for the master password and tries again.
* **D-3, which callers.** A *browser* is recognised through `CallingAppInfo.getOrigin(allowlist)` with Google's privileged-apps list, shipped as the raw resource
  `hexis_privileged_apps.json` (source `assets/passkey/privileged_apps.json`, fetched 2026-11-25, 73 apps; refresh before each release). An *app* has no web origin, so Hexis fetches
  `https://<rp id>/.well-known/assetlinks.json` (https only, 2 s, 64 KB, no redirects, and only for a name that can be a public website: at least two labels, a last label of letters, not `local`, `localhost`, `internal`, `lan` and similar) and accepts the app only when the file lists its package and its
  signing-certificate hash. That is the only request the provider ever makes. It is switchable (D-3, `passkeyAppCheck`, default on): when off, apps get nothing (`app_check_off`) and no request
  is made; browsers are not affected.
* **Off until asked.** The service is `android:enabled="false"` in the manifest and appears in the system's list only after Settings > Passkeys switches the component on
  (`PackageManager.setComponentEnabledSetting`). `PasskeyActivity` is not exported, `FLAG_SECURE`, and drops touches while another window is on top.
* One core per process (`provider::shared`): whichever starts first (the window or the system binding the service) builds the `AppCore`, the other reuses it. Data folder: `Context.dataDir/<identifier>`, the same one the window uses.
* Check that the manifest stays legal for Hexis's own rule (no notifications, no foreground service, no background service): `scripts/check_no_notifications.py` allows exactly one `<service>`, this one, and only in
  this shape (the platform-only bind permission, no foreground type, the Credential Manager action as its only entry point). A test covers each way of widening it.

Not verified on a phone: that the provider shows in the system list, the sheet itself, `BiometricPrompt`, creating a passkey from Chrome and from an app, the hybrid transport
(scanning a desktop browser's QR code with the system camera). The cases to try are listed in `e2e/README.md` ("Phone checks for passkeys").

## 5. The scanner and the smart match (`src-tauri/src/scan.rs`, `src/components/scan/`)

The camera button in the account bar (shortcut `S`, or `Ctrl/Cmd+Shift+S`; listed in Help) opens one scanner for everything. The desktop version takes a pasted text, a dropped image or a typed
key; the phone version uses the camera. What the text is decides the next step (`scan_classify`):

| Text | Class | What the person sees |
|---|---|---|
| `otpauth://totp/…` | `totp` | the issuer and account; "Add to" a login that matches, other logins, or "Create a new login" |
| `otpauth-migration://…` (an authenticator app's export of many codes) | `migration` | explained, with "Add them as separate codes", which opens the new-item screen with the text, where the existing multi-code split saves one code-only item per account |
| `FIDO:/…` (a browser's passkey code) | `fido` | explained: use the phone's own camera (see §6); on a computer, that a phone must answer it |
| `https://…` | `link` | nothing is saved; "Make a login for <host>" opens the new-login screen with that site |
| a bare base32 key | `totp` only when typed or pasted (`bare_key`), never from a camera | as `totp` |
| anything else | `text` / `invalid` | shown, nothing saved |

**Smart match** (`suggest_logins`, in Rust so the sheet and the scanner agree): for a one-time code the account name (the part before `@` is also tried), the issuer and the login's title are
compared with every login; for a passkey the website's rp id is compared with the logins' sites. Scores: account 100, site 60, name 40; the best eight are offered, strongest first, each with the reason
("same account", "same site", "same name"), how many codes and passkeys it already holds, whether this exact code is already on it, and whether it already holds a passkey for that website.
**Every choice always includes "Create a new login"**, and existing logins are listed even when nothing matches. Applying (`scan_apply`) adds the code as `totp#n` on the chosen login, or makes a new login
(account name as user, issuer as title, site from the issuer when it looks like a domain). A login with no free code slot is shown as full.

The master password is asked only when the chosen login is flagged re-prompt, exactly as for any other edit (`password_required`).

## 6. Using a passkey of Hexis on another device

* **Phone, in a phone browser or an app:** the provider (§4).
* **Phone, signing in on a computer** ("Use a passkey on a phone or tablet", the QR code): the system camera, not Hexis, reads the browser's code and hands the request to the chosen provider; Hexis only has to
  be the provider. The in-app scanner recognises a `FIDO:/` code only to tell the person this and not to fail silently. Whether it works on the user's phone: unverified.
* **Computer:** there is no desktop provider (P5 is not built). Passkeys are stored and synced; a computer can show them (site, user, created) and export them, not sign with them.

## 7. Moving passkeys in and out (`passkey/bitwarden.rs`, `importer/`)

* **Import** of a Bitwarden JSON export reads `login.fido2Credentials`: `credentialId` (a GUID becomes its 16 bytes, anything else is base64url), `keyValue` (PKCS#8, either base64 alphabet),
  `rpId`, `rpName`, `userHandle`, `userName`, `userDisplayName`, `creationDate`, `keyAlgorithm` / `keyCurve` / `keyType`. Only ECDSA with P-256 is accepted; every other kind is skipped
  with a warning that names the site, never silently. A passkey-only record imports as a login. Merge mode joins it to a matching login by site and account and never overwrites a held passkey.
* **Export** writes `fido2Credentials` in Bitwarden JSON (url-safe key value, `counter` `"0"`, `discoverable` `"true"`). CSV cannot carry them: choosing CSV skips them with a warning. Plain-text export is
  re-prompted with the master password and already says the file holds secrets; the page now says it holds private keys of passkeys.
* An earlier exporter would have put passkey #2 and later into hidden custom fields and dropped #1; both are fixed (`more_of_login`, tests in `app_import_tests`).

## 8. Tests and how to run them

| What | Command |
|---|---|
| Engine, storage, merge, scanner, provider, import | `cd src-tauri && cargo test --locked` (names: `passkey`, `provider`, `scan`, `app_import`) |
| Android manifest and sources patch | `python3 -m unittest discover -s scripts/tests -t .` |
| Scanner and passkey rows in the preview | `node e2e/scan-ui.mjs`, `node e2e/provider-ui.mjs` |
| The real core behind the scanner (no mock) | `node e2e/real-scan.mjs` (needs the `e2e_server` example) |
| A made-up website's request through the whole provider | `cargo run --example passkey_fixture` |
| Everything CI runs | `scripts/verify_all.sh` (add `--e2e-only` for the browser scripts) |

## 9. Security notes (what to look at in a review)

A read-only security review of this work (2026-11-25) found no exploitable issue at high confidence. Its lower-severity notes were fixed the same day: the phone with no screen lock (above), the origin host rule, the private-address guard, an exact class name and a single copy in `check_no_notifications.py`, and request codes in the provider seeded from the clock. Nothing was run on a phone.

* The only secrets that can reach the UI are the ones the person asked to reveal; passkeys are not among them (there is no reveal for a passkey).
* The private key is read only to sign (core), to export (core, re-prompted), and when an item is sealed, merged or synced (it stays inside the sealed item). A test (`the_private_key_is_in_no_view_no_editable_copy_no_list_no_history_and_no_search_result`) looks for the key in every view, editable copy, list, history entry and search result.
* The provider answers only to the system (`BIND_CREDENTIAL_PROVIDER_SERVICE`); its sheet cannot be started by another app; `FLAG_SECURE` keeps it out of screenshots and the recents screen.
* The assetlinks fetch is the only network call of the provider, made only after the person picked a passkey for an app; an attacker who controls the response can only make an app **not** match (the file is parsed, never executed, and only an exact package
  plus certificate hash match counts).
* `rp_matches_origin` and `host_of_origin` accept only host names made of `a-z`, `0-9`, `-` and `.`, so an origin a browser would read differently (`https://evil.com\.example.com`) never matches. Nothing untrusted reaches them today (browser origins come from Android already canonical); the rule is there for the desktop bridge.
* The one request of the provider cannot be pointed at a private address by an app that names a made-up `rpId`: single-label and local-use names are refused before any request.
* Known and by design: with the vault open, the sheet lists the matching logins' titles and user names before the fingerprint (the same exposure as the window while the app lock is open; no secret is shown). A new login for a passkey made in a PIN-opened vault needs no master password, as in the app.
* The privileged-apps list is a trust list: a package on it may name any origin. It is Google's own list, shipped with the app (fetched 2026-11-25) and refreshed by hand before each release; the changelog records the date.
