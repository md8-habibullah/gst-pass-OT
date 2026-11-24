//! Asks a few large HTTPS servers what time it is.
//!
//! Why HTTPS and not NTP: the answer must be something the person cannot forge by running a fake
//! time server on their own network. A TLS connection to a well-known host proves who answered, so
//! the `Date` header of its reply is real time. (The price: a device whose own date is so wrong that
//! the certificates look expired cannot check at all. That is the safe direction; the core then
//! relies on the boot clock, see `clock/mod.rs`.)
//!
//! What leaves the device: a plain `HEAD` request to each host, nothing else. No cookie, no
//! identifier, no account name, no Hexis name in any header (the agent says only `ureq`). The hosts
//! see an IP address asking for their front page, like any browser or phone does all day.

use super::boot::{BootClock, BootStamp};
use super::{Sample, SyncError, TimeFetcher};
use std::time::Duration;

/// Hosts that answer a bare `HEAD` fast, from anywhere, with a fresh `Date`.
pub const SOURCES: [&str; 3] = [
    "https://www.google.com/generate_204",
    "https://www.cloudflare.com/cdn-cgi/trace",
    "https://aws.amazon.com/",
];
/// A round trip longer than this is too imprecise to use.
const MAX_ROUND_TRIP_MS: u64 = 5_000;
/// A cached reply (a CDN edge that answers from its memory) can carry an old `Date`. `Age` says how old.
const MAX_AGE_SECS: u64 = 5;
/// `Date` has whole-second resolution and is truncated, so the true time is half a second later on average.
const TRUNCATION_MS: u64 = 500;
/// 2024-01-01: an answer before this is garbage.
const FLOOR_MS: u64 = 1_704_067_200_000;

pub struct HttpsFetcher;

impl TimeFetcher for HttpsFetcher {
    fn fetch(&self, boot: &dyn BootClock) -> Result<Vec<Sample>, SyncError> {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .max_redirects(0)
                .http_status_as_error(false)
                .https_only(true)
                .timeout_resolve(Some(Duration::from_secs(4)))
                .timeout_connect(Some(Duration::from_secs(4)))
                .timeout_global(Some(Duration::from_secs(7)))
                .max_response_header_size(16 * 1024)
                .build(),
        );
        let samples: Vec<Sample> = std::thread::scope(|scope| {
            let jobs: Vec<_> = SOURCES
                .iter()
                .map(|url| scope.spawn(|| one(&agent, boot, url)))
                .collect();
            jobs.into_iter()
                .filter_map(|j| j.join().ok().flatten())
                .collect()
        });
        if samples.is_empty() {
            Err(SyncError::Offline)
        } else {
            Ok(samples)
        }
    }
}

fn one(agent: &ureq::Agent, boot: &dyn BootClock, url: &str) -> Option<Sample> {
    let before = boot.now();
    let res = agent
        .head(url)
        .header("cache-control", "no-cache")
        .call()
        .ok()?;
    let after = boot.now();
    if before.id != after.id || after.ms < before.ms {
        return None;
    }
    let round_trip = after.ms - before.ms;
    if round_trip > MAX_ROUND_TRIP_MS {
        return None;
    }
    if let Some(age) = res
        .headers()
        .get("age")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        if age > MAX_AGE_SECS {
            return None;
        }
    }
    let date = res.headers().get("date")?.to_str().ok()?;
    let utc = parse_http_date(date)?.checked_add(TRUNCATION_MS)?;
    if utc < FLOOR_MS {
        return None;
    }
    // The server stamped its reply about half a round trip before we got it.
    Some(Sample {
        utc_ms: utc,
        boot: BootStamp {
            id: after.id,
            ms: before.ms + round_trip / 2,
        },
    })
}

/// `Tue, 24 Nov 2026 12:27:03 GMT` (the only form HTTP/1.1 servers must send) to milliseconds since the epoch.
pub fn parse_http_date(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let weekday = parts.next()?;
    if !weekday.ends_with(',') || weekday.len() != 4 {
        return None;
    }
    let day: u64 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts.next()?.parse().ok()?;
    let clock = parts.next()?;
    if parts.next()? != "GMT" || parts.next().is_some() {
        return None;
    }
    let mut hms = clock.split(':');
    let hour: u64 = hms.next()?.parse().ok()?;
    let minute: u64 = hms.next()?.parse().ok()?;
    let second: u64 = hms.next()?.parse().ok()?;
    if hms.next().is_some() || hour > 23 || minute > 59 || second > 60 || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day as i64)?;
    let secs = days
        .checked_mul(86_400)?
        .checked_add((hour * 3600 + minute * 60 + second.min(59)) as i64)?;
    u64::try_from(secs).ok()?.checked_mul(1000)
}

/// Days since 1970-01-01 of a calendar date (proleptic Gregorian), or `None` for a date that does not exist.
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month_len = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day > month_len || !(1970..=9999).contains(&year) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_standard_date_header() {
        // 1994-11-06 08:49:37 UTC, the example from the HTTP specification.
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777_000)
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse_http_date("Tue, 24 Nov 2026 12:27:03 GMT"),
            Some(1_795_523_223_000)
        );
        // leap day, and a leap second folded into the last second of the minute
        assert_eq!(
            parse_http_date("Thu, 29 Feb 2024 23:59:60 GMT"),
            Some(1_709_251_199_000)
        );
        assert_eq!(
            parse_http_date("  Tue, 24 Nov 2026 12:27:03 GMT \n"),
            Some(1_795_523_223_000)
        );
    }

    #[test]
    fn refuses_everything_else() {
        for bad in [
            "",
            "garbage",
            "Tue, 24 Nov 2026 12:27:03 UTC",
            "Tue, 24 Nov 2026 12:27:03 GMT extra",
            "Tue 24 Nov 2026 12:27:03 GMT",
            "Tue, 31 Nov 2026 12:27:03 GMT",
            "Tue, 29 Feb 2026 12:27:03 GMT",
            "Tue, 24 Foo 2026 12:27:03 GMT",
            "Tue, 24 Nov 2026 24:00:00 GMT",
            "Tue, 24 Nov 2026 12:60:00 GMT",
            "Tue, 24 Nov 1969 12:00:00 GMT",
            "Tue, 00 Nov 2026 12:00:00 GMT",
            "Tuesday, 24-Nov-26 12:27:03 GMT",
        ] {
            assert_eq!(parse_http_date(bad), None, "{bad:?}");
        }
    }

    /// Needs the internet; run by hand with `cargo test clock::net::tests::live -- --ignored`.
    #[test]
    #[ignore]
    fn live() {
        use super::super::boot::SystemBoot;
        let samples = HttpsFetcher
            .fetch(&SystemBoot)
            .expect("reach a time server");
        let wall = crate::crypto::now_millis() as i64;
        for s in &samples {
            let skew = s.utc_ms as i64 - wall;
            eprintln!("sample: skew from this device's clock {skew} ms");
            assert!(
                skew.abs() < 120_000,
                "this device's clock is off by {skew} ms"
            );
        }
    }
}
