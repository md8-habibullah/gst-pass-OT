//! Chunked streaming encryption of files ("drive blobs", `ai-docs/CRYPTO_SPEC.md` "Files (v1)").
//!
//! A file is encrypted in **1 MiB plaintext chunks** with XChaCha20-Poly1305 under a per-file key
//! (`derive_subkey(root, Attachment, <file uuid>)`). Reading and writing use one fixed buffer of a
//! chunk, so the peak memory is about two chunks, never the file.
//!
//! ```text
//! blob   = header(31) | chunk_0 | chunk_1 | ... | chunk_{n-1}
//! header = "PMDF" | version u8 (1) | suite u8 (1) | file uuid (16 raw bytes)
//!          | chunk size log2 u8 (20) | total chunks u64 BE
//! chunk  = nonce(24, fresh random) | ciphertext | tag(16)
//! AAD    = suite u8 | file uuid (16) | chunk index u64 BE | total chunks u64 BE | last flag u8
//! ```
//!
//! * `total chunks = max(1, ceil(size / 1 MiB))` is known up front. A zero-length file is one
//!   empty chunk. Every chunk but the last is exactly 1 MiB of plaintext.
//! * The AAD binds each chunk to its file, its position and the total count, so reordering,
//!   dropping, duplicating, truncating, appending or moving chunks to another file fails
//!   authentication. The header's other fields (magic, version, suite, chunk size) are validated
//!   strictly, so a flipped bit anywhere in the blob is rejected.
//! * The name, size and type of the file are **not** in the blob: they live in a sealed metadata
//!   envelope next to it.
//! * Every decrypt failure is the opaque `Authentication` error. Plaintext handed to the writer
//!   is always authenticated first, chunk by chunk; if a later chunk fails, the caller deletes
//!   what it wrote (nothing here can un-write bytes).

use super::error::CryptoError;
use super::rng;
use super::secret::SecretKey32;
use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use zeroize::Zeroizing;

pub const MAGIC: [u8; 4] = *b"PMDF";
pub const FORMAT_VERSION: u8 = 1;
/// The AEAD suite of format 1: XChaCha20-Poly1305 with 24-byte random nonces and 16-byte tags.
pub const SUITE_ID: u8 = 1;
pub const CHUNK_LOG2: u8 = 20;
/// Plaintext bytes per chunk (the last one may be shorter).
pub const CHUNK_SIZE: usize = 1 << CHUNK_LOG2;
/// The per-file limit of the drive: 50 MB (52,428,800 bytes), enforced up front and again while
/// streaming.
pub const MAX_FILE_BYTES: u64 = 52_428_800;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const HEADER_LEN: usize = 4 + 1 + 1 + 16 + 1 + 8;
/// Bytes a chunk adds around its plaintext.
pub const CHUNK_OVERHEAD: u64 = (NONCE_LEN + TAG_LEN) as u64;
/// The most chunks a file at the limit has.
pub const MAX_CHUNKS: u64 = MAX_FILE_BYTES.div_ceil(CHUNK_SIZE as u64);
/// The largest blob this format version can produce.
pub const MAX_BLOB_BYTES: u64 = HEADER_LEN as u64 + MAX_FILE_BYTES + MAX_CHUNKS * CHUNK_OVERHEAD;

const AAD_LEN: usize = 1 + 16 + 8 + 8 + 1;

/// Why a stream operation stopped. Messages are fixed text: no file names, no byte values.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// Wrong key, damaged or tampered data, or a blob that is not a valid drive blob.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("the file could not be read or written")]
    Io(#[from] std::io::Error),
    #[error("the file is larger than the 50 MB limit")]
    TooLarge,
    /// The source grew or shrank while it was being read.
    #[error("the file changed while it was being read")]
    SourceChanged,
    /// The vault was locked (or the operation dropped) while the stream was running.
    #[error("the operation was cancelled")]
    Cancelled,
}

fn malformed(what: &'static str) -> StreamError {
    StreamError::Crypto(CryptoError::MalformedEnvelope(what))
}

fn auth() -> StreamError {
    StreamError::Crypto(CryptoError::Authentication)
}

/// `max(1, ceil(plain_len / 1 MiB))`.
pub fn total_chunks(plain_len: u64) -> u64 {
    plain_len.div_ceil(CHUNK_SIZE as u64).max(1)
}

/// Exact size of the blob of a file with `plain_len` plaintext bytes.
pub fn blob_len(plain_len: u64) -> u64 {
    HEADER_LEN as u64 + plain_len + total_chunks(plain_len) * CHUNK_OVERHEAD
}

/// Parse a canonical lowercase hyphenated UUID into its 16 raw bytes.
pub fn uuid_bytes(uuid: &str) -> Option<[u8; 16]> {
    let b = uuid.as_bytes();
    if b.len() != 36 {
        return None;
    }
    let mut out = [0u8; 16];
    let mut n = 0usize;
    let mut i = 0usize;
    while i < 36 {
        if matches!(i, 8 | 13 | 18 | 23) {
            if b[i] != b'-' {
                return None;
            }
            i += 1;
            continue;
        }
        let hi = hex_val(b[i])?;
        let lo = hex_val(*b.get(i + 1)?)?;
        *out.get_mut(n)? = (hi << 4) | lo;
        n += 1;
        i += 2;
    }
    (n == 16).then_some(out)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// The plain (unauthenticated) blob header. Everything in it is also covered by the chunk AAD
/// or validated strictly, so tampering with it can only make the blob fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobHeader {
    pub file_id: [u8; 16],
    pub total_chunks: u64,
}

impl BlobHeader {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4] = FORMAT_VERSION;
        out[5] = SUITE_ID;
        out[6..22].copy_from_slice(&self.file_id);
        out[22] = CHUNK_LOG2;
        out[23..31].copy_from_slice(&self.total_chunks.to_be_bytes());
        out
    }

    /// Strict structural parse: magic, version, suite and chunk size must be exactly this
    /// format's, and the chunk count must be possible for a file within the limit.
    pub fn parse(bytes: &[u8; HEADER_LEN]) -> Result<Self, CryptoError> {
        if bytes[..4] != MAGIC {
            return Err(CryptoError::MalformedEnvelope("not a drive blob"));
        }
        if bytes[4] != FORMAT_VERSION || bytes[5] != SUITE_ID || bytes[22] != CHUNK_LOG2 {
            return Err(CryptoError::Unsupported("drive blob format"));
        }
        let mut file_id = [0u8; 16];
        file_id.copy_from_slice(&bytes[6..22]);
        let mut total = [0u8; 8];
        total.copy_from_slice(&bytes[23..31]);
        let total_chunks = u64::from_be_bytes(total);
        if total_chunks == 0 || total_chunks > MAX_CHUNKS {
            return Err(CryptoError::MalformedEnvelope("bad chunk count"));
        }
        Ok(Self {
            file_id,
            total_chunks,
        })
    }

    /// The smallest and largest blob length a valid blob with this many chunks can have.
    pub fn length_range(&self) -> (u64, u64) {
        let n = self.total_chunks;
        let min_plain = if n == 1 {
            0
        } else {
            (n - 1) * CHUNK_SIZE as u64 + 1
        };
        let max_plain = n * CHUNK_SIZE as u64;
        (blob_len_of(n, min_plain), blob_len_of(n, max_plain))
    }
}

fn blob_len_of(chunks: u64, plain: u64) -> u64 {
    HEADER_LEN as u64 + plain + chunks * CHUNK_OVERHEAD
}

/// Structural check of a stored blob **without any key**: header parses, belongs to `file_id`,
/// and the byte length is possible for its chunk count. Used by restore and before opening.
pub fn check_blob_structure(
    header_bytes: &[u8; HEADER_LEN],
    file_id: &[u8; 16],
    total_len: u64,
) -> Result<BlobHeader, CryptoError> {
    let header = BlobHeader::parse(header_bytes)?;
    if &header.file_id != file_id {
        return Err(CryptoError::MalformedEnvelope(
            "blob belongs to another file",
        ));
    }
    let (min, max) = header.length_range();
    if total_len < min || total_len > max {
        return Err(CryptoError::MalformedEnvelope(
            "blob length is not possible",
        ));
    }
    Ok(header)
}

fn aad(file_id: &[u8; 16], index: u64, total: u64, last: bool) -> [u8; AAD_LEN] {
    let mut out = [0u8; AAD_LEN];
    out[0] = SUITE_ID;
    out[1..17].copy_from_slice(file_id);
    out[17..25].copy_from_slice(&index.to_be_bytes());
    out[25..33].copy_from_slice(&total.to_be_bytes());
    out[33] = u8::from(last);
    out
}

fn cipher(key: &SecretKey32) -> Result<XChaCha20Poly1305, CryptoError> {
    XChaCha20Poly1305::new_from_slice(key.as_bytes())
        .map_err(|_| CryptoError::InvalidInput("bad AEAD key length"))
}

fn xnonce(bytes: &[u8; NONCE_LEN]) -> Result<XNonce, CryptoError> {
    XNonce::try_from(&bytes[..]).map_err(|_| CryptoError::InvalidInput("bad nonce length"))
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), StreamError> {
    if cancel.load(Ordering::SeqCst) {
        Err(StreamError::Cancelled)
    } else {
        Ok(())
    }
}

/// Encrypt exactly `plain_len` bytes read from `src` into a blob written to `dst`.
///
/// Fails with `TooLarge` before reading anything when `plain_len` is over the limit, and with
/// `SourceChanged` if `src` ends early or still has bytes after `plain_len` (it grew or shrank
/// while being read). On any error the caller discards `dst`. Returns the blob length.
pub fn encrypt_stream<R: Read, W: Write>(
    key: &SecretKey32,
    file_id: &[u8; 16],
    plain_len: u64,
    src: &mut R,
    dst: &mut W,
    cancel: &AtomicBool,
) -> Result<u64, StreamError> {
    if plain_len > MAX_FILE_BYTES {
        return Err(StreamError::TooLarge);
    }
    let total = total_chunks(plain_len);
    let cipher = cipher(key)?;
    dst.write_all(
        &BlobHeader {
            file_id: *file_id,
            total_chunks: total,
        }
        .encode(),
    )?;
    let mut written = HEADER_LEN as u64;
    // One chunk buffer, reused: the plaintext is read into it, sealed in place (the tag is
    // appended inside the reserved capacity, so it never reallocates) and written out.
    let mut buf: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(CHUNK_SIZE + TAG_LEN));
    let mut remaining = plain_len;
    for index in 0..total {
        check_cancel(cancel)?;
        let want = usize::try_from(remaining.min(CHUNK_SIZE as u64)).unwrap_or(CHUNK_SIZE);
        buf.clear();
        buf.resize(want, 0);
        match src.read_exact(&mut buf[..]) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(StreamError::SourceChanged)
            }
            Err(e) => return Err(e.into()),
        }
        remaining -= want as u64;
        let last = index + 1 == total;
        let nonce_bytes = rng::array::<NONCE_LEN>()?;
        cipher
            .encrypt_in_place(
                &xnonce(&nonce_bytes)?,
                &aad(file_id, index, total, last),
                &mut *buf,
            )
            .map_err(|_| CryptoError::TooLarge)?;
        dst.write_all(&nonce_bytes)?;
        dst.write_all(&buf)?;
        written += NONCE_LEN as u64 + buf.len() as u64;
    }
    // The source must be exhausted: more bytes than announced means it grew while we read.
    let mut probe = [0u8; 1];
    loop {
        match src.read(&mut probe) {
            Ok(0) => break,
            Ok(_) => return Err(StreamError::SourceChanged),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    dst.flush()?;
    debug_assert_eq!(written, blob_len(plain_len));
    Ok(written)
}

/// Authenticate and decrypt a blob chunk by chunk, writing each verified chunk to `dst`.
/// Returns the number of plaintext bytes written.
///
/// Rejects a blob whose header belongs to another file, whose chunk count or lengths are not
/// possible, that is truncated, or that has anything after its last chunk. Every authentication
/// failure is the same opaque error. The caller must discard `dst` on error.
pub fn decrypt_stream<R: Read, W: Write>(
    key: &SecretKey32,
    file_id: &[u8; 16],
    src: &mut R,
    dst: &mut W,
    cancel: &AtomicBool,
) -> Result<u64, StreamError> {
    let mut head = [0u8; HEADER_LEN];
    read_all(src, &mut head)?;
    let header = BlobHeader::parse(&head)?;
    if &header.file_id != file_id {
        return Err(malformed("blob belongs to another file"));
    }
    let total = header.total_chunks;
    let cipher = cipher(key)?;
    let mut buf: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::with_capacity(CHUNK_SIZE + TAG_LEN));
    let mut plain_total = 0u64;
    for index in 0..total {
        check_cancel(cancel)?;
        let last = index + 1 == total;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        read_all(src, &mut nonce_bytes)?;
        buf.clear();
        if last {
            // Everything that is left, but never more than one chunk: more means trailing bytes.
            let limit = (CHUNK_SIZE + TAG_LEN + 1) as u64;
            src.by_ref().take(limit).read_to_end(&mut buf)?;
            if buf.len() > CHUNK_SIZE + TAG_LEN {
                return Err(malformed("data after the last chunk"));
            }
            // A multi-chunk file never ends in an empty chunk.
            let min = if total == 1 { TAG_LEN } else { TAG_LEN + 1 };
            if buf.len() < min {
                return Err(malformed("truncated"));
            }
        } else {
            buf.resize(CHUNK_SIZE + TAG_LEN, 0);
            read_all(src, &mut buf[..])?;
        }
        cipher
            .decrypt_in_place(
                &xnonce(&nonce_bytes)?,
                &aad(file_id, index, total, last),
                &mut *buf,
            )
            .map_err(|_| auth())?;
        plain_total += buf.len() as u64;
        if plain_total > MAX_FILE_BYTES {
            return Err(StreamError::TooLarge);
        }
        dst.write_all(&buf)?;
    }
    dst.flush()?;
    Ok(plain_total)
}

/// `read_exact`, with a short source reported as a truncated blob instead of an I/O error.
fn read_all<R: Read>(src: &mut R, buf: &mut [u8]) -> Result<(), StreamError> {
    match src.read_exact(buf) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(malformed("truncated")),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Cursor;

    const MIB: usize = 1 << 20;
    const ID: [u8; 16] = [
        0x3f, 0x2b, 0x8c, 0x1e, 0x5d, 0x4a, 0x4e, 0x7b, 0x9c, 0x1d, 0x0a, 0x1b, 0x2c, 0x3d, 0x4e,
        0x5f,
    ];
    const ID_TEXT: &str = "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f";

    fn key() -> SecretKey32 {
        SecretKey32::from_slice(&[0x42; 32]).unwrap()
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// Deterministic, non-repeating-looking test data.
    fn data(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| (i.wrapping_mul(31).wrapping_add(i >> 7) & 0xff) as u8)
            .collect()
    }

    fn seal(plain: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let n = encrypt_stream(
            &key(),
            &ID,
            plain.len() as u64,
            &mut Cursor::new(plain),
            &mut out,
            &no_cancel(),
        )
        .unwrap();
        assert_eq!(n as usize, out.len());
        out
    }

    fn open_with(k: &SecretKey32, id: &[u8; 16], blob: &[u8]) -> Result<Vec<u8>, StreamError> {
        let mut out = Vec::new();
        decrypt_stream(k, id, &mut Cursor::new(blob), &mut out, &no_cancel())?;
        Ok(out)
    }

    fn open(blob: &[u8]) -> Result<Vec<u8>, StreamError> {
        open_with(&key(), &ID, blob)
    }

    fn is_auth_or_malformed(e: &StreamError) -> bool {
        matches!(
            e,
            StreamError::Crypto(
                CryptoError::Authentication
                    | CryptoError::MalformedEnvelope(_)
                    | CryptoError::Unsupported(_)
            )
        )
    }

    /// Byte range of chunk `i` (nonce + ciphertext + tag) in a blob of full chunks.
    fn chunk_range(i: usize, blob_len: usize, total: usize) -> std::ops::Range<usize> {
        let start = HEADER_LEN + i * (MIB + NONCE_LEN + TAG_LEN);
        let end = if i + 1 == total {
            blob_len
        } else {
            start + MIB + NONCE_LEN + TAG_LEN
        };
        start..end
    }

    #[test]
    fn constants_and_layout_helpers() {
        assert_eq!(HEADER_LEN, 31);
        assert_eq!(CHUNK_SIZE, 1_048_576);
        assert_eq!(MAX_FILE_BYTES, 52_428_800);
        assert_eq!(MAX_CHUNKS, 50);
        assert_eq!(total_chunks(0), 1);
        assert_eq!(total_chunks(1), 1);
        assert_eq!(total_chunks(MIB as u64), 1);
        assert_eq!(total_chunks(MIB as u64 + 1), 2);
        assert_eq!(total_chunks(MAX_FILE_BYTES), 50);
        assert_eq!(blob_len(0), 31 + 40);
        assert_eq!(blob_len(MIB as u64), 31 + MIB as u64 + 40);
        assert_eq!(blob_len(MIB as u64 + 1), 31 + MIB as u64 + 1 + 80);
        assert_eq!(blob_len(MAX_FILE_BYTES), MAX_BLOB_BYTES);
        assert_eq!(uuid_bytes(ID_TEXT), Some(ID));
        for bad in [
            "",
            "3F2B8C1E-5D4A-4E7B-9C1D-0A1B2C3D4E5F",
            "3f2b8c1e5d4a4e7b9c1d0a1b2c3d4e5f",
            "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5g",
            "3f2b8c1e-5d4a-4e7b-9c1d-0a1b2c3d4e5f0",
        ] {
            assert_eq!(uuid_bytes(bad), None, "{bad}");
        }
    }

    #[test]
    fn header_layout_is_as_documented() {
        let blob = seal(b"hello");
        assert_eq!(&blob[..4], b"PMDF");
        assert_eq!(blob[4], 1, "format version");
        assert_eq!(blob[5], 1, "suite id");
        assert_eq!(&blob[6..22], &ID);
        assert_eq!(blob[22], 20, "chunk size log2");
        assert_eq!(&blob[23..31], &1u64.to_be_bytes());
        assert_eq!(blob.len(), 31 + 24 + 5 + 16);
        // nothing about the file name or its size is in the header beyond the chunk count
        assert!(!blob.windows(5).any(|w| w == b"hello"));
    }

    #[test]
    fn round_trips_at_the_chunk_boundaries() {
        for size in [0usize, 1, MIB - 1, MIB, MIB + 1, 2 * MIB, 3 * MIB + 7] {
            let plain = data(size);
            let blob = seal(&plain);
            assert_eq!(blob.len() as u64, blob_len(size as u64), "size {size}");
            assert_eq!(open(&blob).unwrap(), plain, "size {size}");
        }
    }

    #[test]
    fn an_empty_file_is_one_empty_chunk() {
        let blob = seal(b"");
        assert_eq!(blob.len(), HEADER_LEN + NONCE_LEN + TAG_LEN);
        assert!(open(&blob).unwrap().is_empty());
    }

    #[test]
    fn every_region_of_a_small_blob_is_authenticated() {
        let plain = data(300);
        let blob = seal(&plain);
        // header, nonce, every ciphertext byte, every tag byte: flipping any bit fails
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x01;
            let e = open(&bad).expect_err(&format!("byte {i} flipped must fail"));
            assert!(is_auth_or_malformed(&e), "byte {i}: {e:?}");
        }
    }

    #[test]
    fn flipped_bits_in_a_multi_chunk_blob_fail_in_every_region() {
        let plain = data(3 * MIB + 7);
        let blob = seal(&plain);
        let total = 4;
        for i in 0..total {
            let r = chunk_range(i, blob.len(), total);
            let nonce = r.start;
            let ct_mid = r.start + NONCE_LEN + (r.end - r.start - NONCE_LEN - TAG_LEN) / 2;
            let tag = r.end - 1;
            for pos in [
                nonce,
                nonce + 23,
                r.start + NONCE_LEN,
                ct_mid,
                tag - 15,
                tag,
            ] {
                let mut bad = blob.clone();
                bad[pos] ^= 0x80;
                let e = open(&bad).expect_err("tampered chunk must fail");
                assert!(is_auth_or_malformed(&e), "chunk {i} byte {pos}: {e:?}");
            }
        }
        for pos in 0..HEADER_LEN {
            let mut bad = blob.clone();
            bad[pos] ^= 0x01;
            assert!(open(&bad).is_err(), "header byte {pos}");
        }
    }

    #[test]
    fn swapping_two_chunks_fails() {
        let blob = seal(&data(3 * MIB + 7));
        let total = 4;
        let (a, b) = (
            chunk_range(0, blob.len(), total),
            chunk_range(1, blob.len(), total),
        );
        let mut bad = blob[..HEADER_LEN].to_vec();
        bad.extend_from_slice(&blob[b.clone()]);
        bad.extend_from_slice(&blob[a]);
        bad.extend_from_slice(&blob[chunk_range(2, blob.len(), total)]);
        bad.extend_from_slice(&blob[chunk_range(3, blob.len(), total)]);
        assert_eq!(bad.len(), blob.len());
        assert!(matches!(
            open(&bad),
            Err(StreamError::Crypto(CryptoError::Authentication))
        ));
    }

    #[test]
    fn dropping_a_middle_chunk_fails() {
        let blob = seal(&data(3 * MIB + 7));
        let total = 4;
        let mut bad = blob[..HEADER_LEN].to_vec();
        for i in [0usize, 2, 3] {
            bad.extend_from_slice(&blob[chunk_range(i, blob.len(), total)]);
        }
        assert!(open(&bad).is_err());
    }

    #[test]
    fn truncating_the_last_chunk_fails_even_if_the_header_is_fixed_up() {
        let blob = seal(&data(3 * MIB + 7));
        let total = 4;
        // drop the last chunk entirely
        let short = &blob[..chunk_range(3, blob.len(), total).start];
        assert!(open(short).is_err());
        // and also lower the chunk count in the header: the AAD still says "4 chunks"
        let mut fixed = short.to_vec();
        fixed[23..31].copy_from_slice(&3u64.to_be_bytes());
        let e = open(&fixed).expect_err("count in the AAD must not match");
        assert!(matches!(
            e,
            StreamError::Crypto(CryptoError::Authentication)
        ));
        // cutting bytes off the tail
        for cut in [1usize, TAG_LEN, TAG_LEN + 3] {
            assert!(open(&blob[..blob.len() - cut]).is_err(), "cut {cut}");
        }
    }

    #[test]
    fn duplicating_a_chunk_fails() {
        let blob = seal(&data(3 * MIB + 7));
        let total = 4;
        let mut dup = blob[..HEADER_LEN].to_vec();
        dup.extend_from_slice(&blob[chunk_range(0, blob.len(), total)]);
        for i in 0..total {
            dup.extend_from_slice(&blob[chunk_range(i, blob.len(), total)]);
        }
        assert!(open(&dup).is_err());
    }

    #[test]
    fn appending_a_valid_looking_chunk_from_the_same_file_fails() {
        let blob = seal(&data(2 * MIB));
        let total = 2;
        // header says 2 chunks; a third one (a copy of chunk 0) hangs off the end
        let mut longer = blob.clone();
        longer.extend_from_slice(&blob[chunk_range(0, blob.len(), total)]);
        assert!(matches!(
            open(&longer),
            Err(StreamError::Crypto(CryptoError::MalformedEnvelope(_)))
        ));
        // and with the count raised to match, the AAD (total, last flag) no longer fits
        longer[23..31].copy_from_slice(&3u64.to_be_bytes());
        assert!(matches!(
            open(&longer),
            Err(StreamError::Crypto(CryptoError::Authentication))
        ));
        // a few stray bytes after the end are refused too
        let mut stray = blob.clone();
        stray.push(0);
        assert!(open(&stray).is_err());
    }

    #[test]
    fn moving_the_body_under_another_file_id_fails() {
        let blob = seal(&data(MIB + 10));
        let other = [9u8; 16];
        // the honest reader of the other file refuses the header
        assert!(open_with(&key(), &other, &blob).is_err());
        // rewriting the header's id still fails: the chunks are bound to the original id
        let mut moved = blob.clone();
        moved[6..22].copy_from_slice(&other);
        assert!(matches!(
            open_with(&key(), &other, &moved),
            Err(StreamError::Crypto(CryptoError::Authentication))
        ));
    }

    #[test]
    fn the_wrong_key_fails_with_the_opaque_error() {
        let blob = seal(&data(1000));
        let other = SecretKey32::from_slice(&[0x43; 32]).unwrap();
        assert!(matches!(
            open_with(&other, &ID, &blob),
            Err(StreamError::Crypto(CryptoError::Authentication))
        ));
        let text = open_with(&other, &ID, &blob).unwrap_err().to_string();
        assert!(!text.contains("1000"));
    }

    #[test]
    fn nonces_are_unique_across_chunks_and_runs() {
        let plain = data(3 * MIB + 7);
        let mut seen = HashSet::new();
        for _ in 0..3 {
            let blob = seal(&plain);
            for i in 0..4 {
                let r = chunk_range(i, blob.len(), 4);
                let nonce = blob[r.start..r.start + NONCE_LEN].to_vec();
                assert!(seen.insert(nonce), "a nonce repeated");
            }
        }
        assert_eq!(seen.len(), 12);
        // the same plaintext never yields the same blob
        assert_ne!(seal(&plain), seal(&plain));
    }

    #[test]
    fn the_size_limit_is_enforced_up_front() {
        let mut out = Vec::new();
        // a source that would produce data forever is never read when the size is over the limit
        let mut endless = std::io::repeat(7);
        let e = encrypt_stream(
            &key(),
            &ID,
            MAX_FILE_BYTES + 1,
            &mut endless,
            &mut out,
            &no_cancel(),
        )
        .unwrap_err();
        assert!(matches!(e, StreamError::TooLarge));
        assert!(out.is_empty(), "nothing is written before the check");
    }

    #[test]
    fn exactly_the_limit_is_accepted_and_a_header_beyond_it_is_refused() {
        // exactly 52,428,800 bytes: accepted, 50 chunks, and it decrypts
        let plain = vec![0x5au8; MAX_FILE_BYTES as usize];
        let mut blob = Vec::new();
        let started = std::time::Instant::now();
        encrypt_stream(
            &key(),
            &ID,
            plain.len() as u64,
            &mut Cursor::new(&plain),
            &mut blob,
            &no_cancel(),
        )
        .unwrap();
        assert_eq!(blob.len() as u64, MAX_BLOB_BYTES);
        let back = open(&blob).unwrap();
        assert_eq!(back.len(), plain.len());
        assert!(back == plain);
        let took = started.elapsed();
        // Dev/test builds instantiate the generic AEAD code at opt-level 0 (about 7 MB/s per
        // direction on an idle machine, about 60 s when the whole suite runs in parallel);
        // release builds are far faster. This only guards against a regression that would make
        // the test build unusable, so the bound is loose.
        assert!(
            took < std::time::Duration::from_secs(240),
            "50 MiB encrypt + decrypt took {took:?}"
        );
        // (a source that holds more than the announced size is refused while streaming: see
        // `a_source_that_grows_or_shrinks_is_detected`)
        // a decryptor refuses a header that promises more chunks than the limit allows
        let mut bad = blob[..HEADER_LEN].to_vec();
        bad[23..31].copy_from_slice(&(MAX_CHUNKS + 1).to_be_bytes());
        assert!(open(&bad).is_err());
    }

    #[test]
    fn a_source_that_grows_or_shrinks_is_detected() {
        let mut sink = Vec::new();
        // grew: more bytes than announced
        let e = encrypt_stream(
            &key(),
            &ID,
            100,
            &mut Cursor::new(data(101)),
            &mut sink,
            &no_cancel(),
        )
        .unwrap_err();
        assert!(matches!(e, StreamError::SourceChanged));
        // grew across a chunk boundary
        let e = encrypt_stream(
            &key(),
            &ID,
            MIB as u64,
            &mut Cursor::new(data(MIB + 1)),
            &mut sink,
            &no_cancel(),
        )
        .unwrap_err();
        assert!(matches!(e, StreamError::SourceChanged));
        // shrank
        let e = encrypt_stream(
            &key(),
            &ID,
            2 * MIB as u64,
            &mut Cursor::new(data(MIB + 5)),
            &mut sink,
            &no_cancel(),
        )
        .unwrap_err();
        assert!(matches!(e, StreamError::SourceChanged));
        // announced empty, but the source has bytes
        let e = encrypt_stream(
            &key(),
            &ID,
            0,
            &mut Cursor::new(data(1)),
            &mut sink,
            &no_cancel(),
        )
        .unwrap_err();
        assert!(matches!(e, StreamError::SourceChanged));
    }

    #[test]
    fn cancelling_stops_both_directions_between_chunks() {
        let cancel = AtomicBool::new(true);
        let mut sink = Vec::new();
        let e = encrypt_stream(
            &key(),
            &ID,
            10,
            &mut Cursor::new(data(10)),
            &mut sink,
            &cancel,
        )
        .unwrap_err();
        assert!(matches!(e, StreamError::Cancelled));
        let blob = seal(&data(10));
        let e =
            decrypt_stream(&key(), &ID, &mut Cursor::new(&blob), &mut sink, &cancel).unwrap_err();
        assert!(matches!(e, StreamError::Cancelled));
    }

    #[test]
    fn io_errors_surface_without_panicking() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
        }
        struct FullDisk;
        impl Write for FullDisk {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Vec::new();
        assert!(matches!(
            encrypt_stream(&key(), &ID, 5, &mut Failing, &mut sink, &no_cancel()),
            Err(StreamError::Io(_))
        ));
        assert!(matches!(
            encrypt_stream(
                &key(),
                &ID,
                5,
                &mut Cursor::new(data(5)),
                &mut FullDisk,
                &no_cancel()
            ),
            Err(StreamError::Io(_))
        ));
        assert!(matches!(
            decrypt_stream(&key(), &ID, &mut Failing, &mut sink, &no_cancel()),
            Err(StreamError::Io(_))
        ));
        let blob = seal(&data(5));
        assert!(matches!(
            decrypt_stream(
                &key(),
                &ID,
                &mut Cursor::new(&blob),
                &mut FullDisk,
                &no_cancel()
            ),
            Err(StreamError::Io(_))
        ));
        // error texts are fixed sentences
        for e in [StreamError::TooLarge, StreamError::SourceChanged] {
            assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn garbage_and_foreign_blobs_are_rejected_structurally() {
        assert!(open(&[]).is_err());
        assert!(open(&[0u8; 10]).is_err());
        assert!(open(&vec![0u8; 500]).is_err());
        let mut wrong_version = seal(b"x");
        wrong_version[4] = 2;
        assert!(matches!(
            open(&wrong_version),
            Err(StreamError::Crypto(CryptoError::Unsupported(_)))
        ));
        let mut wrong_log2 = seal(b"x");
        wrong_log2[22] = 19;
        assert!(open(&wrong_log2).is_err());
        let mut zero_chunks = seal(b"x");
        zero_chunks[23..31].copy_from_slice(&0u64.to_be_bytes());
        assert!(open(&zero_chunks).is_err());
    }

    #[test]
    fn the_structure_check_needs_no_key() {
        let blob = seal(&data(MIB + 10));
        let mut head = [0u8; HEADER_LEN];
        head.copy_from_slice(&blob[..HEADER_LEN]);
        let ok = check_blob_structure(&head, &ID, blob.len() as u64).unwrap();
        assert_eq!(ok.total_chunks, 2);
        assert!(check_blob_structure(&head, &[1u8; 16], blob.len() as u64).is_err());
        assert!(check_blob_structure(&head, &ID, blob.len() as u64 + 1_048_576).is_err());
        assert!(check_blob_structure(&head, &ID, HEADER_LEN as u64).is_err());
        // a one-chunk blob has room for an empty chunk, a two-chunk blob does not
        let (min1, _) = BlobHeader {
            file_id: ID,
            total_chunks: 1,
        }
        .length_range();
        assert_eq!(min1, (HEADER_LEN + NONCE_LEN + TAG_LEN) as u64);
        let (min2, _) = BlobHeader {
            file_id: ID,
            total_chunks: 2,
        }
        .length_range();
        assert_eq!(min2, blob_len(MIB as u64 + 1));
    }

    #[test]
    fn a_multi_chunk_blob_never_ends_in_an_empty_chunk() {
        // forge a structurally "valid" blob whose last chunk is empty by sealing it by hand
        let key = key();
        let cipher = cipher(&key).unwrap();
        let total = 2u64;
        let mut blob = BlobHeader {
            file_id: ID,
            total_chunks: total,
        }
        .encode()
        .to_vec();
        for (index, plain) in [(0u64, vec![1u8; CHUNK_SIZE]), (1, Vec::new())] {
            let nonce = rng::array::<NONCE_LEN>().unwrap();
            let mut buf = plain;
            cipher
                .encrypt_in_place(
                    &xnonce(&nonce).unwrap(),
                    &aad(&ID, index, total, index + 1 == total),
                    &mut buf,
                )
                .unwrap();
            blob.extend_from_slice(&nonce);
            blob.extend_from_slice(&buf);
        }
        assert!(matches!(
            open(&blob),
            Err(StreamError::Crypto(CryptoError::MalformedEnvelope(_)))
        ));
    }
}
