#!/usr/bin/env python3
"""Independent end-to-end unlock of a Hexis vault key bundle, written from CRYPTO_SPEC.md only.

Uses Python `cryptography` (Argon2id, HKDF-SHA512, ML-KEM-1024, X25519, ChaCha20-Poly1305) and
the XChaCha20 helper from gen_crypto_kat.py. If this succeeds on a bundle produced by the Rust
code, the Rust implementation is interoperable with a second implementation of every primitive
and of the wire formats (FIPS 203 seed/ciphertext encodings included).

    python3 scripts/verify_bundle_fixture.py            # checks the committed format fixture
"""
import importlib.util
import json
import pathlib
import struct
import sys
import unicodedata
import base64

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric.mlkem import MLKEM1024PrivateKey
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.kdf.argon2 import Argon2id
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

HERE = pathlib.Path(__file__).resolve().parent
FIX = HERE.parent / "src-tauri" / "src" / "crypto" / "fixtures"
spec = importlib.util.spec_from_file_location("kat", HERE / "gen_crypto_kat.py")
kat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(kat)  # runs its draft-vector self checks


def hkdf(ikm, salt, info):
    return HKDF(algorithm=hashes.SHA512(), length=32, salt=salt, info=info).derive(ikm)


def subkey(root, label, ctx=""):
    return hkdf(root, b"passman/v1/hkdf", b"passman/v1\x00" + label.encode() + b"\x00" + ctx.encode())


def open_envelope(key, blob, expect_id):
    assert blob[:4] == b"PMV1", "bad magic"
    (hl,) = struct.unpack(">I", blob[4:8])
    header_bytes, nonce, ct = blob[8:8 + hl], blob[8 + hl:32 + hl], blob[32 + hl:]
    header = json.loads(header_bytes)
    assert list(header) == ["v", "kem", "kdf", "aead", "itemId", "version", "timestamp"], "field order"
    assert json.dumps(header, separators=(",", ":")).encode() == header_bytes, "header is not compact canonical JSON"
    assert (header["v"], header["kem"], header["kdf"], header["aead"]) == (1, "ML-KEM-1024+X25519", "Argon2id", "XChaCha20-Poly1305")
    assert header["itemId"] == expect_id, header
    return header, kat.xchacha_decrypt(key, nonce, header_bytes, ct)


def main():
    b = json.loads((FIX / "bundle_v1.json").read_text())
    password = "fixture passphrase v1"
    d = lambda s: base64.b64decode(s, validate=True)

    # 1. password -> Argon2id -> keyring key -> keyring
    k = b["kdf"]
    assert (k["alg"], k["version"]) == ("Argon2id", 19)
    pk = Argon2id(salt=d(k["salt"]), length=32, iterations=k["tCost"], lanes=k["pCost"], memory_cost=k["mCostKiB"]).derive(
        unicodedata.normalize("NFKC", password).encode())
    _, keyring = open_envelope(subkey(pk, "keyring"), d(b["sealedKeyring"]), "_keyring")
    assert len(keyring) == 128
    seed, x_sk, commit = keyring[:64], keyring[64:96], keyring[96:]

    # 2. the public keys in the bundle belong to those secrets
    dk = MLKEM1024PrivateKey.from_seed_bytes(seed)
    assert dk.public_key().public_bytes_raw() == d(b["publicKeys"]["mlKem1024"]), "ML-KEM public key mismatch"
    x_priv = X25519PrivateKey.from_private_bytes(x_sk)
    pk_x = x_priv.public_key().public_bytes_raw()
    assert pk_x == d(b["publicKeys"]["x25519"]), "X25519 public key mismatch"

    # 3. hybrid decapsulation + combiner (CRYPTO_SPEC section 6)
    w = b["wrappedRootKey"]
    ct_m, pk_e = d(w["mlKemCt"]), d(w["x25519Eph"])
    ss_m = dk.decapsulate(ct_m)
    ss_x = x_priv.exchange(X25519PublicKey.from_public_bytes(pk_e))
    wrap_key = hkdf(ss_m + ss_x, b"passman/v1/hybrid-kem-salt", b"passman/v1/hybrid-kem\x00" + ct_m + pk_e + pk_x)

    # 4. root key, commitment, object key, stored item
    _, vrk = open_envelope(wrap_key, d(w["envelope"]), "_vrk")
    assert len(vrk) == 32
    assert subkey(vrk, "vrk-commit") == commit, "root-key commitment mismatch"
    item_key = subkey(vrk, "item", "fixture-item")
    assert item_key.hex() == (FIX / "bundle_v1.item_key.hex").read_text().strip(), "item key differs from Rust"
    header, plaintext = open_envelope(item_key, bytes.fromhex((FIX / "bundle_v1.item.hex").read_text().strip()), "fixture-item")
    assert plaintext == b"fixture plaintext" and header["version"] == 1
    print("OK: Python independently unlocked the Rust-written bundle and decrypted the stored item")


if __name__ == "__main__":
    sys.exit(main())
