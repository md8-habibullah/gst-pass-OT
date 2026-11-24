//! The boot clock: milliseconds since the device started, **including time spent asleep**.
//!
//! This is the one clock a person cannot move by changing the date in the settings. A wait that is
//! measured on it (the lock after wrong passcodes) cannot be shortened by setting the date forward.
//!
//! * Linux and Android: the first number of `/proc/uptime` (kernel `CLOCK_BOOTTIME`, asleep time
//!   included) and the kernel's `boot_id`, which changes at every start of the device. No `unsafe`
//!   and no extra crate.
//! * Anywhere else, or where `/proc` cannot be read: milliseconds since this app run started and a
//!   random id for this run. A wait measured that way survives nothing but the run itself, so after a
//!   restart the core falls back to the checked UTC time (see `clock/mod.rs`).

use std::sync::OnceLock;
use std::time::Instant;

/// One reading of the boot clock. `id` names the boot (or the app run), `ms` counts from its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootStamp {
    pub id: String,
    pub ms: u64,
}

pub trait BootClock: Send + Sync {
    fn now(&self) -> BootStamp;
}

/// The real thing.
pub struct SystemBoot;

impl BootClock for SystemBoot {
    fn now(&self) -> BootStamp {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let (Some(ms), Some(id)) = (proc_uptime_ms(), kernel_boot_id()) {
            return BootStamp { id, ms };
        }
        run_local()
    }
}

/// Milliseconds since the app run started, with an id that only this run has.
fn run_local() -> BootStamp {
    static START: OnceLock<(Instant, String)> = OnceLock::new();
    let (start, id) = START.get_or_init(|| {
        let mut raw = [0u8; 8];
        let _ = crate::crypto::rng::fill(&mut raw);
        (
            Instant::now(),
            format!(
                "run-{}",
                raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
            ),
        )
    });
    BootStamp {
        id: id.clone(),
        ms: start.elapsed().as_millis() as u64,
    }
}

/// The first field of `/proc/uptime` ("12345.67 ..."), in milliseconds.
pub fn parse_uptime_ms(text: &str) -> Option<u64> {
    let first = text.split_whitespace().next()?;
    let (whole, frac) = first.split_once('.').unwrap_or((first, "0"));
    let secs: u64 = whole.parse().ok()?;
    let hundredths: u64 = format!("{frac:0<2}")[..2].parse().ok()?;
    secs.checked_mul(1000)?.checked_add(hundredths * 10)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn proc_uptime_ms() -> Option<u64> {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| parse_uptime_ms(&t))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn kernel_boot_id() -> Option<String> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let id = id.trim();
    (!id.is_empty() && id.len() <= 64).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_first_uptime_field_in_milliseconds() {
        assert_eq!(parse_uptime_ms("12345.67 54321.00\n"), Some(12_345_670));
        assert_eq!(parse_uptime_ms("0.05 0.00"), Some(50));
        assert_eq!(parse_uptime_ms("7 3"), Some(7000));
        assert_eq!(parse_uptime_ms("9.5 1"), Some(9500));
        assert_eq!(parse_uptime_ms(""), None);
        assert_eq!(parse_uptime_ms("abc 1"), None);
        assert_eq!(parse_uptime_ms("-5.0 1"), None);
    }

    #[test]
    fn the_system_clock_moves_forward_and_keeps_its_id() {
        let a = SystemBoot.now();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let b = SystemBoot.now();
        assert_eq!(a.id, b.id);
        assert!(b.ms >= a.ms);
        assert!(!a.id.is_empty());
    }
}
