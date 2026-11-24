//! Lenient serde helpers for vendor JSON, plus time parsing. Exports written by different app
//! versions disagree about types (a number here, a string there, `null` elsewhere); these helpers
//! take whatever scalar is there and ignore containers instead of failing the whole file.

use super::limits::{unsupported, MAX_DEPTH, MSG_DAMAGED, MSG_NOT_TEXT, MSG_TOO_DEEP};
use crate::storage::StorageError;
use serde::de::{DeserializeOwned, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

struct LaxString;

impl<'de> Visitor<'de> for LaxString {
    type Value = Option<String>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a scalar")
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(Some(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(Some(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Some(v.to_string()))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(Some(v.to_string()))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
        Ok(Some(v.to_string()))
    }
    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(Some(v.to_string()))
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(LaxString)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(None)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(None)
    }
}

/// A text value: a string, or a number or boolean written as text; anything else is `None`.
pub fn lax<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    d.deserialize_any(LaxString)
}

/// An integer written as a number or as numeric text.
pub fn lax_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(lax(d)?.and_then(|s| {
        s.trim()
            .parse::<i64>()
            .ok()
            .or_else(|| s.trim().parse::<f64>().ok().map(|f| f as i64))
    }))
}

/// A boolean written as `true`, `1`, `"yes"`, ...
pub fn lax_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(lax(d)?.map(|s| {
        matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "y" | "on"
        )
    }))
}

/// A container (or any `Default` value) that may be missing or `null`.
pub fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// A scalar of any kind, kept as text.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop, Default)]
pub struct Scalar(#[serde(deserialize_with = "lax")] pub Option<String>);

/// A JSON object read as ordered `(name, scalar)` pairs. Members that are not scalars are kept as
/// `None`; at most 4096 members are read. `null` is an empty object.
#[derive(Zeroize, ZeroizeOnDrop, Default)]
pub struct Pairs(pub Vec<(String, Scalar)>);

struct PairsVisitor;

impl<'de> Visitor<'de> for PairsVisitor {
    type Value = Pairs;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object")
    }
    fn visit_unit<E>(self) -> Result<Pairs, E> {
        Ok(Pairs::default())
    }
    fn visit_none<E>(self) -> Result<Pairs, E> {
        Ok(Pairs::default())
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Pairs, A::Error> {
        let mut out = Vec::new();
        while let Some(k) = map.next_key::<String>()? {
            let v: Scalar = map.next_value()?;
            if out.len() < 4096 {
                out.push((k, v));
            }
        }
        Ok(Pairs(out))
    }
}

impl<'de> Deserialize<'de> for Pairs {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(PairsVisitor)
    }
}

impl Pairs {
    /// The value of the member called `name`, if it is a scalar.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.0.as_deref())
    }
}

/// Refuse a document that nests arrays or objects deeper than [`MAX_DEPTH`]. `serde_json` stops
/// recursion at 128 levels for values it builds, but it skips unknown values without any limit,
/// so the limit is enforced here for everything, in one pass that is aware of strings.
pub fn check_depth(bytes: &[u8]) -> Result<(), StorageError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &b in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(unsupported(MSG_TOO_DEEP));
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// Parse a JSON document (nesting limited by [`check_depth`]).
pub fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StorageError> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    check_depth(body)?;
    serde_json::from_slice(body).map_err(|e| {
        use serde_json::error::Category;
        match e.classify() {
            Category::Io => unsupported(MSG_NOT_TEXT),
            _ => unsupported(MSG_DAMAGED),
        }
    })
}

/// Text of an optional lenient string, trimmed of nothing (secrets keep their spaces).
pub fn s(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("")
}

// ------------------------------------------------------------------ time

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2024-03-05T10:20:30Z`, `...30.123+02:00`, `2024-03-05 10:20:30`, or just a date, to Unix
/// milliseconds. `None` for anything else or for a time before 1970.
pub fn parse_iso(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, rest) = match s.find(['T', ' ']) {
        Some(i) => (s.get(..i)?, s.get(i + 1..)?),
        None => (s, ""),
    };
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    if dp.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
        return None;
    }
    let (mut hh, mut mm, mut ss, mut ms, mut off) = (0i64, 0i64, 0i64, 0i64, 0i64);
    if !rest.is_empty() {
        let (clock, zone) = match rest.find(['Z', 'z', '+']) {
            Some(i) => (rest.get(..i)?, rest.get(i..)?),
            None => match rest.rfind('-') {
                Some(i) => (rest.get(..i)?, rest.get(i..)?),
                None => (rest, ""),
            },
        };
        let mut cp = clock.split(':');
        hh = cp.next()?.parse().ok()?;
        mm = cp.next().unwrap_or("0").parse().ok()?;
        let sec = cp.next().unwrap_or("0");
        let (whole, frac) = sec.split_once('.').unwrap_or((sec, ""));
        ss = whole.parse().ok()?;
        if !frac.is_empty() {
            let f: String = frac.chars().take(3).collect();
            ms = format!("{f:0<3}").parse().ok()?;
        }
        if !(0..24).contains(&hh) || !(0..60).contains(&mm) || !(0..=60).contains(&ss) {
            return None;
        }
        if zone.starts_with(['+', '-']) {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let z = zone.get(1..)?.replace(':', "");
            let (zh, zm) = (
                z.get(..2)?.parse::<i64>().ok()?,
                z.get(2..).unwrap_or("0").parse::<i64>().unwrap_or(0),
            );
            off = sign * (zh * 60 + zm);
        }
    }
    let secs = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - off * 60;
    u64::try_from(secs).ok().map(|s| s * 1000 + ms as u64)
}

/// `2024-03-05T10:20:30.123Z` for a Unix time in milliseconds (UTC).
pub fn format_iso(ms: u64) -> String {
    let secs = ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms % 1000
    )
}

/// A Unix time written in seconds or in milliseconds.
pub fn epoch_millis(n: i64) -> Option<u64> {
    let n = u64::try_from(n).ok()?;
    Some(if n < 100_000_000_000 { n * 1000 } else { n })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Default)]
    struct T {
        #[serde(default, deserialize_with = "lax")]
        a: Option<String>,
        #[serde(default, deserialize_with = "lax_i64")]
        n: Option<i64>,
        #[serde(default, deserialize_with = "lax_bool")]
        b: Option<bool>,
    }

    #[test]
    fn lenient_scalars() {
        let t: T = parse(br#"{"a": 5, "n": "7", "b": 1}"#).unwrap();
        assert_eq!((t.a.as_deref(), t.n, t.b), (Some("5"), Some(7), Some(true)));
        let t: T = parse(br#"{"a": null, "n": null, "b": "no"}"#).unwrap();
        assert_eq!((t.a, t.n, t.b), (None, None, Some(false)));
        let t: T = parse(br#"{"a": {"x": [1,2]}, "n": [1], "b": true}"#).unwrap();
        assert_eq!((t.a, t.n, t.b), (None, None, Some(true)));
        let t: T = parse(b"\xEF\xBB\xBF{\"a\": \"x\"}").unwrap();
        assert_eq!(t.a.as_deref(), Some("x"));
        assert!(parse::<T>(b"{not json").is_err());
        assert!(parse::<T>(b"").is_err());
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(parse::<T>(deep.as_bytes()).is_err());
        let deep_obj = format!("{}1{}", "{\"a\":".repeat(10_000), "}".repeat(10_000));
        assert!(parse::<T>(deep_obj.as_bytes()).is_err());
    }

    #[test]
    fn depth_check_counts_levels_and_ignores_strings() {
        let at = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(check_depth(at(MAX_DEPTH).as_bytes()).is_ok());
        assert!(check_depth(at(MAX_DEPTH + 1).as_bytes()).is_err());
        // brackets inside strings (even after escaped quotes) are not nesting
        let in_string = format!(r#"{{"a": "{}\" {}"}}"#, "[".repeat(500), "{".repeat(500));
        assert!(check_depth(in_string.as_bytes()).is_ok());
        // a closing bracket never drives the count below zero
        assert!(check_depth(b"]]]]{}").is_ok());
        assert!(check_depth(b"").is_ok());
        let err = check_depth(at(MAX_DEPTH + 1).as_bytes()).unwrap_err();
        assert!(!err.to_string().contains('['), "errors never echo the file");
    }

    #[test]
    fn iso_times() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2024-03-05T10:20:30Z"), Some(1_709_634_030_000));
        assert_eq!(
            parse_iso("2024-03-05T10:20:30.123Z"),
            Some(1_709_634_030_123)
        );
        assert_eq!(
            parse_iso("2024-03-05T12:20:30+02:00"),
            Some(1_709_634_030_000)
        );
        assert_eq!(parse_iso("2024-03-05 10:20:30"), Some(1_709_634_030_000));
        assert_eq!(parse_iso("2024-03-05"), Some(1_709_596_800_000));
        assert_eq!(
            parse_iso("2024-03-05T05:20:30-05:00"),
            Some(1_709_634_030_000)
        );
        assert_eq!(parse_iso("1969-12-31"), None);
        assert_eq!(parse_iso("garbage"), None);
        assert_eq!(parse_iso("2024-13-01"), None);
        assert_eq!(parse_iso(""), None);
    }

    #[test]
    fn iso_formatting_is_the_inverse_of_parsing() {
        assert_eq!(format_iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_iso(1_709_634_030_123), "2024-03-05T10:20:30.123Z");
        assert_eq!(format_iso(951_782_400_000), "2000-02-29T00:00:00.000Z");
        for ms in [0u64, 1, 86_399_999, 1_709_634_030_123, 4_102_444_799_999] {
            assert_eq!(parse_iso(&format_iso(ms)), Some(ms));
        }
    }

    #[test]
    fn epoch_units() {
        assert_eq!(epoch_millis(1_709_634_030), Some(1_709_634_030_000));
        assert_eq!(epoch_millis(1_709_634_030_000), Some(1_709_634_030_000));
        assert_eq!(epoch_millis(-1), None);
    }
}
