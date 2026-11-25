//! The little CBOR (RFC 8949) a passkey needs, and nothing more: an encoder for the three things the
//! authenticator writes (the public key in COSE form, the attestation object, and byte strings), and, in tests only, a
//! decoder used to check the bytes by reading them back the way a website does.
//!
//! Only the "core deterministic" subset is written: the shortest form of every number and length, definite lengths,
//! and map keys in the order CTAP2 asks for (shorter encoded key first, then bytewise). Callers pass keys already in
//! that order (the tests read the bytes back and compare them with the RFC's own examples).

/// Writes CBOR into a byte vector.
#[derive(Default)]
pub struct Writer {
    out: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.out
    }

    fn head(&mut self, major: u8, n: u64) {
        let m = major << 5;
        if n < 24 {
            self.out.push(m | n as u8);
        } else if n <= 0xff {
            self.out.extend_from_slice(&[m | 24, n as u8]);
        } else if n <= 0xffff {
            self.out.push(m | 25);
            self.out.extend_from_slice(&(n as u16).to_be_bytes());
        } else if n <= 0xffff_ffff {
            self.out.push(m | 26);
            self.out.extend_from_slice(&(n as u32).to_be_bytes());
        } else {
            self.out.push(m | 27);
            self.out.extend_from_slice(&n.to_be_bytes());
        }
    }

    /// An integer, positive or negative.
    pub fn int(&mut self, v: i64) -> &mut Self {
        if v >= 0 {
            self.head(0, v as u64);
        } else {
            self.head(1, (-1 - v) as u64);
        }
        self
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.head(2, b.len() as u64);
        self.out.extend_from_slice(b);
        self
    }

    pub fn text(&mut self, s: &str) -> &mut Self {
        self.head(3, s.len() as u64);
        self.out.extend_from_slice(s.as_bytes());
        self
    }

    /// The header of a map with `pairs` entries; the caller writes key, value, key, value …
    pub fn map(&mut self, pairs: usize) -> &mut Self {
        self.head(5, pairs as u64);
        self
    }
}

/// The COSE_Key (RFC 9052) of an ES256 public key, in the canonical order CTAP2 uses:
/// `{1: 2 (EC2), 3: -7 (ES256), -1: 1 (P-256), -2: x, -3: y}`.
pub fn cose_es256_key(x: &[u8; 32], y: &[u8; 32]) -> Vec<u8> {
    let mut w = Writer::new();
    w.map(5);
    w.int(1).int(2);
    w.int(3).int(-7);
    w.int(-1).int(1);
    w.int(-2).bytes(x);
    w.int(-3).bytes(y);
    w.into_bytes()
}

/// The attestation object of "no attestation": `{"fmt": "none", "attStmt": {}, "authData": <bytes>}`.
/// (Canonical order: "fmt" (3 letters), "attStmt" (7), "authData" (8).)
pub fn attestation_none(auth_data: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.map(3);
    w.text("fmt").text("none");
    w.text("attStmt").map(0);
    w.text("authData").bytes(auth_data);
    w.into_bytes()
}

#[cfg(test)]
pub mod read {
    //! A strict reader for the subset above (definite lengths, shortest numbers), for tests.

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Item {
        Int(i64),
        Bytes(Vec<u8>),
        Text(String),
        Map(Vec<(Item, Item)>),
    }

    impl Item {
        pub fn get(&self, key: &Item) -> Option<&Item> {
            match self {
                Item::Map(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }
        pub fn text_key(&self, key: &str) -> Option<&Item> {
            self.get(&Item::Text(key.into()))
        }
        pub fn int_key(&self, key: i64) -> Option<&Item> {
            self.get(&Item::Int(key))
        }
        pub fn as_bytes(&self) -> &[u8] {
            match self {
                Item::Bytes(b) => b,
                other => panic!("not bytes: {other:?}"),
            }
        }
    }

    /// Parse one item that must fill `input` exactly.
    pub fn parse(input: &[u8]) -> Item {
        let (item, rest) = take(input);
        assert!(rest.is_empty(), "{} bytes left over", rest.len());
        item
    }

    /// Parse one item and return what follows it.
    pub fn take(input: &[u8]) -> (Item, &[u8]) {
        let first = input[0];
        let (major, info) = (first >> 5, first & 31);
        let (n, mut rest) = match info {
            0..=23 => (info as u64, &input[1..]),
            24 => (input[1] as u64, &input[2..]),
            25 => (u16::from_be_bytes([input[1], input[2]]) as u64, &input[3..]),
            26 => (
                u32::from_be_bytes(input[1..5].try_into().unwrap()) as u64,
                &input[5..],
            ),
            27 => (
                u64::from_be_bytes(input[1..9].try_into().unwrap()),
                &input[9..],
            ),
            other => panic!("unsupported additional info {other}"),
        };
        // shortest form only
        let shortest = match info {
            24 => n >= 24,
            25 => n > 0xff,
            26 => n > 0xffff,
            27 => n > 0xffff_ffff,
            _ => true,
        };
        assert!(shortest, "number not in shortest form");
        match major {
            0 => (Item::Int(n as i64), rest),
            1 => (Item::Int(-1 - n as i64), rest),
            2 => {
                let (b, r) = rest.split_at(n as usize);
                (Item::Bytes(b.to_vec()), r)
            }
            3 => {
                let (b, r) = rest.split_at(n as usize);
                (Item::Text(String::from_utf8(b.to_vec()).unwrap()), r)
            }
            5 => {
                let mut pairs = Vec::new();
                for _ in 0..n {
                    let (k, r) = take(rest);
                    let (v, r) = take(r);
                    pairs.push((k, v));
                    rest = r;
                }
                (Item::Map(pairs), rest)
            }
            other => panic!("unsupported major type {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read::{parse, Item};
    use super::*;

    #[test]
    fn numbers_use_the_shortest_form() {
        // RFC 8949 appendix A examples
        let enc = |v: i64| {
            let mut w = Writer::new();
            w.int(v);
            w.into_bytes()
        };
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(23), [0x17]);
        assert_eq!(enc(24), [0x18, 0x18]);
        assert_eq!(enc(255), [0x18, 0xff]);
        assert_eq!(enc(256), [0x19, 0x01, 0x00]);
        assert_eq!(enc(65_536), [0x1a, 0x00, 0x01, 0x00, 0x00]);
        assert_eq!(enc(-1), [0x20]);
        assert_eq!(enc(-7), [0x26]);
        assert_eq!(enc(-257), [0x39, 0x01, 0x00]);
        assert_eq!(
            enc(1_000_000_000_000),
            [0x1b, 0, 0, 0, 0xe8, 0xd4, 0xa5, 0x10, 0x00]
        );
    }

    #[test]
    fn strings_and_maps_match_the_rfc_examples() {
        let mut w = Writer::new();
        w.text("IETF");
        assert_eq!(w.into_bytes(), [0x64, b'I', b'E', b'T', b'F']);
        let mut w = Writer::new();
        w.bytes(&[1, 2, 3, 4]);
        assert_eq!(w.into_bytes(), [0x44, 1, 2, 3, 4]);
        let mut w = Writer::new();
        w.map(0);
        assert_eq!(w.into_bytes(), [0xa0]);
        // a 24-byte string needs the one-byte length
        let s = "a".repeat(24);
        let mut w = Writer::new();
        w.text(&s);
        let b = w.into_bytes();
        assert_eq!(&b[..2], [0x78, 24]);
    }

    #[test]
    fn the_cose_key_has_the_ctap2_layout() {
        let key = cose_es256_key(&[7; 32], &[9; 32]);
        // a5 01 02 03 26 20 01 21 58 20 <x> 22 58 20 <y>
        assert_eq!(
            &key[..10],
            [0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20]
        );
        assert_eq!(key.len(), 10 + 32 + 3 + 32);
        assert_eq!(&key[42..45], [0x22, 0x58, 0x20]);
        let item = parse(&key);
        assert_eq!(item.int_key(1), Some(&Item::Int(2)));
        assert_eq!(item.int_key(3), Some(&Item::Int(-7)));
        assert_eq!(item.int_key(-1), Some(&Item::Int(1)));
        assert_eq!(item.int_key(-2).unwrap().as_bytes(), &[7; 32]);
        assert_eq!(item.int_key(-3).unwrap().as_bytes(), &[9; 32]);
    }

    #[test]
    fn the_attestation_object_is_fmt_attstmt_authdata() {
        let obj = attestation_none(&[1, 2, 3]);
        assert_eq!(
            obj,
            [
                0xa3, 0x63, b'f', b'm', b't', 0x64, b'n', b'o', b'n', b'e', 0x67, b'a', b't', b't',
                b'S', b't', b'm', b't', 0xa0, 0x68, b'a', b'u', b't', b'h', b'D', b'a', b't', b'a',
                0x43, 1, 2, 3
            ]
        );
    }
}
