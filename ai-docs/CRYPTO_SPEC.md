# CRYPTO_SPEC — primitives, key hierarchy, wire formats

Source of truth for `src-tauri/src/crypto/`. If code and this file disagree, fix one of them in the same change.
Suite version: **v1**. All crypto runs in Rust. The webview never sees a key, a seed or a master password hash.

Status legend: **[impl]** implemented and tested in Phase 1 · **[plan]** designed, not yet implemented.

## 1. Suite identifiers (written into every envelope header)

| Field  | Value                  | Meaning                                                        |
|--------|------------------------|----------------------------------------------------------------|
| `v`    | `1`                    | Suite/format version. Unknown values are rejected, never guessed. |
| `kem`  | `ML-KEM-1024+X25519`   | Hybrid KEM that wraps the VaultRootKey (§6).                   |
| `kdf`  | `Argon2id`             | Password KDF (§4).                                             |
| `aead` | `XChaCha20-Poly1305`   | Object encryption (§7).                                        |

`kem` and `kdf` in an item envelope describe the vault's suite (the key hierarchy the object key came from), not
an operation performed on that object. A future suite change bumps `v` and adds a migration path (re-wrap, not re-encrypt, where possible).

## 2. Crates (verified against crates.io on 2026-11-23, API read from source)

| Purpose          | Crate (version)              | Notes |
|------------------|------------------------------|-------|
| Password KDF     | `argon2 0.6.0`               | `zeroize` feature; we supply and wipe the 64 MiB work memory ourselves (the crate's internal buffer is freed without wiping). |
| PQ KEM           | `ml-kem 0.3.4`               | FIPS 203 final. Keys stored as the **64-byte seed** (`d‖z`), not the 3168-byte expanded key (deprecated upstream). `zeroize` feature. |
| Classical KEM    | `x25519-dalek 3.0.0`         | `static_secrets` + `zeroize`. Reject non-contributory shared secrets. |
| AEAD             | `chacha20poly1305 0.11.0`    | `XChaCha20Poly1305`, 24-byte random nonce, 16-byte tag. `zeroize` feature. |
| KDF / combiner   | `hkdf 0.13.0` + `sha2 0.11.1` + `hmac 0.13.0` | HKDF-SHA512. `zeroize` features on hmac/sha2. |
| Randomness       | `getrandom 0.4.3` (+ `rand_core 0.10.1`) | OS CSPRNG only. `UnwrapErr(SysRng)` adapts it for ml-kem/x25519 (panics if the OS RNG fails, which is acceptable: no safe fallback exists). |
| Wipe             | `zeroize 1.9.2`, `subtle 2.6.1` | `subtle` for constant-time equality of secrets. |
| TOTP             | `totp-rs 6.0.0`              | Features `std`, `otpauth`, `zeroize`. New 6.x API (`Totp`, `Builder`), not the 5.x `TOTP`. |
| Password text    | `unicode-normalization 0.1.26` | NFKC (§3). |

## 3. Master password handling [impl]

1. Reject empty input. Reject results longer than 1024 bytes (DoS guard).
2. Normalize to **Unicode NFKC**, then UTF-8 encode. This makes the same passphrase unlock identically on Linux and Android keyboards
   (composed vs decomposed characters). Side effect: compatibility characters fold (`ﬁ`→`fi`, full-width digits → ASCII). This is part of format v1 and cannot change later.
3. Normalization writes into a pre-sized buffer (length counted in a first pass), so no reallocation leaves an unwiped copy.
4. The result lives in `Zeroizing<Vec<u8>>` (`MasterPassword`). No `Debug` output, no `Clone`, no `Serialize`.

## 4. Password key derivation [impl]

`PasswordKey = Argon2id(version 0x13, password_nfkc, salt, m, t, p) → 32 bytes`

| Parameter | Default | Allowed range (enforced on create **and** on parse) |
|-----------|---------|------------------------------------------------------|
| `mCostKiB` | 65536 (64 MiB) | 19456 – 1048576 |
| `tCost`    | 4              | 2 – 64 |
| `pCost`    | 4              | 1 – 16 |
| salt       | 32 random bytes | exactly 32 |

* Upper bounds protect against a tampered bundle forcing multi-GiB allocations. Lower bounds follow OWASP's minimum (19 MiB, t=2).
* The `parallel` feature of `argon2` is **not** enabled: lanes run sequentially (same total work, no rayon dependency). Revisit if unlock latency on low-end phones is a problem.
* The raw `PasswordKey` is never used to encrypt data directly; it feeds HKDF with label `keyring` (§5).

## 5. Sub-key derivation [impl]

```
derive_subkey(root, label, context) =
  HKDF-SHA512( salt = "passman/v1/hkdf",  ikm = root (32 B),
               info = "passman/v1" 0x00 label 0x00 context )  →  32 bytes
```

`label` is a fixed enum, `context` is a validated id (`[A-Za-z0-9_.:-]{1,64}`, no NUL) or empty.

| Label        | Root         | Context        | Used for |
|--------------|--------------|----------------|----------|
| `keyring`    | PasswordKey  | (empty)        | seals the secret keyring (§9) |
| `vrk-commit` | VaultRootKey | (empty)        | commitment stored in the keyring (§9) |
| `item`       | VaultRootKey | item UUID      | `vault/items/<id>.enc` |
| `version`    | VaultRootKey | item UUID      | `vault/versions/<id>/*.enc` |
| `attachment` | VaultRootKey | attachment UUID| `vault/attachments/<id>.bin` |
| `meta`       | VaultRootKey | setting name   | local settings, S3 target configs |

Per-object keys give domain separation (an attachment can never be opened as an item, even with a forged header) and limit the blast radius of a leaked sub-key.
Ids starting with `_` are reserved for system objects (`_keyring`, `_vrk`).

## 6. Hybrid KEM (ML-KEM-1024 + X25519) [impl]

Long-term recipient keys are created once per vault: ML-KEM-1024 decapsulation key (from a random 64-byte seed) and an X25519 static secret.

**Encapsulate(recipient ek_M, recipient pk_X):**
1. `(ct_M, ss_M) = ML-KEM-1024.Encaps(ek_M)`
2. ephemeral X25519 secret `e`; `pk_E = X25519(e, base)`; `ss_X = X25519(e, pk_X)`; abort if `ss_X` is non-contributory (all-zero).
3. `wrap_key = HKDF-SHA512( salt = "passman/v1/hybrid-kem-salt",`
   `                         ikm  = ss_M ‖ ss_X,`
   `                         info = "passman/v1/hybrid-kem" 0x00 ‖ ct_M ‖ pk_E ‖ pk_X ) → 32 bytes`
4. output `(ct_M [1568 B], pk_E [32 B], wrap_key)`.

**Decapsulate** mirrors it with the recipient's secrets (`ss_M = Decaps(dk, ct_M)`, `ss_X = X25519(sk_X, pk_E)`); `pk_X` is recomputed from `sk_X`, never read from untrusted input.

Security properties: the result stays secret if **either** ML-KEM-1024 or X25519 holds. Binding `ct_M`, `pk_E` and `pk_X` into `info` follows the X-Wing/hybrid-KEM practice and closes X25519 ciphertext-malleability. ML-KEM implicit rejection means a corrupted `ct_M` yields a pseudorandom (wrong) key, which then fails AEAD authentication: the error is always `Authentication`/`Unlock`, never a distinguishable "bad ciphertext".
The combiner is project-specific (no external test vectors); it is covered by a known-answer test generated with an independent Python HKDF implementation.

**What the KEM layer is for (honest note).** The master password is the only secret protecting an offline copy of the bundle: an attacker who steals it must guess the password through Argon2id, then everything else follows. The KEM layer does not add resistance to password guessing. It adds: (a) the spec'd PQ+classical hybrid for the root-key wrap, (b) the ability to wrap new key material to the public keys **without the password** (future device/recovery recipients, key rotation by a background worker), (c) crypto-agility. The symmetric layers (Argon2id → XChaCha20) are already quantum-resistant at ~128-bit post-Grover strength.

## 7. Envelope (authenticated object container) [impl]

### 7.1 Header (also the AAD)

Compact JSON, UTF-8, fields in this exact order, no whitespace:

```json
{"v":1,"kem":"ML-KEM-1024+X25519","kdf":"Argon2id","aead":"XChaCha20-Poly1305","itemId":"<id>","version":1,"timestamp":1732360000000}
```

* `itemId`: `[A-Za-z0-9_.:-]{1,64}`. `version`: u64 ≥ 1. `timestamp`: UTC ms, u64.
* The **exact bytes** stored in the file are the AAD; the reader never re-serializes before authenticating, so no canonicalization problem exists.
* Parsers reject unknown fields, duplicate fields, wrong `v`/`kem`/`kdf`/`aead`, and `itemId`/`version` that differ from what the caller expects.

### 7.2 Binary layout

```
offset      size   field
0           4      magic "PMV1"  (0x50 0x4D 0x56 0x31)
4           4      header_len, u32 big-endian, 1..=1024
8           H      header JSON (= AAD)
8+H         24     nonce (fresh random per seal)
32+H        N+16   XChaCha20-Poly1305 ciphertext ‖ tag
```

Plaintext limit: 64 MiB per envelope (one-shot AEAD). Larger attachments need a chunked envelope (new `v`), planned with attachments in a later phase.

### 7.3 Rules

* Nonce: always fresh from the OS CSPRNG, never derived or counted. 192 random bits make collisions negligible for any realistic volume per key.
* `open()` requires the caller to state the expected `itemId` (and optionally `version`), so one object cannot be substituted for another.
* All decrypt failures (wrong key, tampered header, tampered ciphertext, truncated tag) surface as the single opaque error `Authentication`. Structural problems detected **before** any key use (bad magic, bad lengths) are `MalformedEnvelope`.
* Decrypted plaintext is returned in `Zeroizing<Vec<u8>>`; decryption happens in-place in a zeroizing buffer.

## 8. Vault key bundle (public file, safe to sync) [impl]

Stored locally and at `vault/meta/keybundle.json` on S3. JSON, camelCase, binary fields standard Base64 with padding.

```json
{
  "format": 1,
  "createdAt": 1732360000000,
  "kdf": { "alg": "Argon2id", "version": 19, "mCostKiB": 65536, "tCost": 4, "pCost": 4, "salt": "<32 B>" },
  "publicKeys": { "mlKem1024": "<1568 B>", "x25519": "<32 B>" },
  "sealedKeyring": "<envelope, itemId=_keyring, sealed with derive_subkey(PasswordKey,'keyring')>",
  "wrappedRootKey": { "mlKemCt": "<1568 B>", "x25519Eph": "<32 B>", "envelope": "<envelope, itemId=_vrk, sealed with wrap_key>" }
}
```

## 9. Secret keyring plaintext (inside `sealedKeyring`) [impl]

Exactly 128 bytes: `mlkem_seed[64] ‖ x25519_secret[32] ‖ vrk_commit[32]`, where `vrk_commit = derive_subkey(VaultRootKey, "vrk-commit", "")`.

**Why the commitment:** the recipient public keys are public, so anyone who can write to the bucket could encapsulate a *different* root key to them. The commitment lives inside the password-sealed keyring, so after unwrapping the root key the client checks `commit == derive_subkey(VRK, ...)` in constant time and aborts with `KeyMismatch` on a substituted wrap. Unlock also checks that `publicKeys` equals the keys re-derived from the secrets.

## 10. Procedures [impl]

**create(password, params)** → `(VaultKeyBundle, VaultRootKey)`
1. validate params; normalize password; `salt ← 32 random bytes`; `PK ← Argon2id`.
2. `seed ← 64 random bytes` → ML-KEM-1024 key; `sk_X ← 32 random bytes` → X25519 key; `VRK ← 32 random bytes`.
3. hybrid-encapsulate to `(ek_M, pk_X)` → `wrap_key`; seal VRK as `_vrk` (version 1).
4. seal keyring `(seed ‖ sk_X ‖ commit)` as `_keyring` (version 1) under `derive_subkey(PK,'keyring')`.

**unlock(bundle, password)** → `VaultRootKey`: validate bundle; `PK`; open keyring (fail ⇒ `Unlock`, deliberately indistinguishable between "wrong password" and "damaged file"); rebuild keys; check public keys; hybrid-decapsulate; open `_vrk`; verify commitment; return VRK. Secret keys are dropped (wiped) before returning.

**verify_password(bundle, password)**: same as the first half of unlock (Argon2id + open keyring). Used by the Master-Password Re-Prompt Guard. Costs one Argon2id run by design.

**change_password(bundle, old, new, new_params)** → new bundle: open keyring with old password; new salt; new `PK'`; re-seal the **same** keyring contents with keyring `version+1`. `wrappedRootKey` and `publicKeys` are unchanged, so no vault data is re-encrypted.

**Not implemented yet [plan]:** VRK rotation, recovery key recipient, quick-unlock blob (Phase 3: `wrapKey = HKDF(Argon2id(PIN, salt) ‖ keystoreSecret)` sealing the VRK, with attempt limiting outside the crypto layer).

## 11. TOTP [impl]

* RFC 6238 via `totp-rs 6.0.0`. Algorithms SHA1/SHA256/SHA512, digits 6–8, period 1–86400 s (default 30), skew 1 for verification helpers.
* `otpauth://totp/…` parser (query `secret`, `algorithm`, `digits`, `period`, `issuer`). `hotp` and `steam` are rejected with a clear error.
* Base32 secrets are normalized before parsing (case, spaces, hyphens, padding). Secrets as short as 80 bits are accepted, because real services issue them; the library's strict builder (≥128 bits) would reject valid accounts. We validate `digits`, `period` and non-empty secret ourselves before using the library's non-compliant builder (which would otherwise panic on `period = 0` or `digits ≥ 10`).
* The secret stays inside Rust (`ZeroizeOnDrop`). The UI only ever receives `{ code, secondsRemaining, period }` and runs the 1 Hz countdown locally; it asks for a new code when the period rolls over.

## 12. Memory hygiene rules

* Every type holding key material, passwords or decrypted secrets implements `Zeroize` + `ZeroizeOnDrop` (or is wrapped in `Zeroizing`). Keys are heap boxed so moves do not copy them around the stack.
* No `Debug`/`Display` of secrets (redacted `Debug` only), no `Clone` on key types, no `Serialize` of secrets except the explicit sealed forms.
* Best effort, stated plainly: Rust cannot rule out copies from register spills, the allocator, or swap. Hardening backlog (Phase 3): `mlock` key pages, disable core dumps / `PR_SET_DUMPABLE=0`, Android `FLAG_SECURE`, wipe on lock.
* Errors never contain secret or user-supplied data.

## 13. Verification status

See `CHANGELOG_AND_STATE.md` for the latest run and the list of what is *not* verified. What exists today:

* **Known-answer tests** (`crypto/kat.rs`, vectors in the generated `crypto/kat_vectors.rs`): Argon2id, NFKC, HKDF-SHA512 sub-keys, the hybrid combiner, XChaCha20-Poly1305 (first vector is the IETF draft one) and a complete envelope, all produced by `scripts/gen_crypto_kat.py` from independent implementations (Python `cryptography` / stdlib). The script first asserts its own HChaCha20 and XChaCha20 code against the IETF draft-irtf-cfrg-xchacha vectors, so a mistake there cannot create wrong expectations. `python3 scripts/gen_crypto_kat.py --check` fails if the committed vectors are stale.
* **TOTP**: RFC 6238 Appendix B vectors for SHA-1/256/512, recomputed with a stdlib HOTP.
* **Interop**: `scripts/verify_bundle_fixture.py` unlocks the Rust-written `crypto/fixtures/bundle_v1.json` in pure Python, including ML-KEM-1024 and X25519 decapsulation, and decrypts the stored item. This proves §6, §8, §9 and §10 are implementable from this document alone.
* **Format regression**: `kat::bundle_v1_fixture_still_unlocks` opens the stored v1 bundle and item with the current code. If it ever fails, the on-disk format changed: bump the suite/version instead of editing the fixture. (`kat::regenerate_bundle_fixture` is `#[ignore]` and only for intentional format changes.)
* **Property/negative tests** in every module: tampered header/nonce/ciphertext, swapped objects (wrong `itemId`/version), wrong password, root-key substitution (keyring commitment), non-contributory X25519 points, bad Argon2 parameters, redacted `Debug`, wiped buffers.
* **Not covered:** no third-party audit, no side-channel testing beyond using constant-time primitives and `subtle` comparisons, no fuzzing yet (candidate for Phase 5: envelope and URI parsers).
