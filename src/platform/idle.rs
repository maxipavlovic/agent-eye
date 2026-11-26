//! How long the machine has had no keyboard or mouse input, for the steering clock's breaks. It
//! reads one number and nothing else: never which keys, which window or which app.
//!
//! macOS asks the HID system for its idle time (`ioreg`), Linux asks logind for the session's idle
//! hint. Where neither answers (no desktop session, a server, another system) the sensor is
//! unavailable and eye keeps taking breaks from the gaps between prompts. Both readers are compiled
//! everywhere and tested on fixed text; `Sys` is what they ask the machine.

use super::{retry_busy, MACOS};
use std::process::Command;

/// What the readers ask the machine.
pub trait Sys {
    fn macos(&self) -> bool;
    /// What a program prints on stdout, None when it is missing or fails.
    fn output(&self, program: &str, args: &[&str]) -> Option<String>;
    fn env(&self, key: &str) -> Option<String>;
    fn now_ms(&self) -> i64;
}

/// The real machine.
pub struct RealSys;

impl Sys for RealSys {
    fn macos(&self) -> bool {
        MACOS
    }

    fn output(&self, program: &str, args: &[&str]) -> Option<String> {
        let out = retry_busy(|| Command::new(program).args(args).output()).ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }

    fn now_ms(&self) -> i64 {
        crate::util::now_ms()
    }
}

/// Milliseconds since the last input, None when this system cannot say.
pub fn idle_ms(sys: &dyn Sys) -> Option<i64> {
    if sys.macos() {
        parse_hid_idle(&sys.output("ioreg", &["-c", "IOHIDSystem", "-d", "4"])?)
    } else {
        let id = sys.env("XDG_SESSION_ID")?;
        let text = sys.output(
            "loginctl",
            &["show-session", &id, "-p", "IdleHint", "-p", "IdleSinceHint"],
        )?;
        parse_idle_hint(&text, sys.now_ms())
    }
}

/// `"HIDIdleTime" = 4513072000` in the `ioreg` listing: nanoseconds, as milliseconds.
pub fn parse_hid_idle(text: &str) -> Option<i64> {
    let line = text.lines().find(|l| l.contains("\"HIDIdleTime\""))?;
    let ns: i64 = line.split('=').nth(1)?.trim().parse().ok()?;
    Some(ns / 1_000_000)
}

/// The `IdleHint` and `IdleSinceHint` lines of `loginctl show-session`. `IdleSinceHint` is the
/// start of the idleness in microseconds since the epoch. A session that does not keep the hint
/// (no `IdleHint` line, or idle with no start time) is unavailable.
pub fn parse_idle_hint(text: &str, now_ms: i64) -> Option<i64> {
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
    };
    match value("IdleHint")? {
        "no" => Some(0),
        "yes" => {
            let since_us: i64 = value("IdleSinceHint")?.parse().ok().filter(|v| *v > 0)?;
            Some((now_ms - since_us / 1000).max(0))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const IOREG: &str = r#"+-o IOHIDSystem  <class IOHIDSystem, id 0x1000002f2, registered, matched, active, busy 0 (0 ms), retain 12>
    {
      "HIDParameters" = {"HIDClickTime"=500000000}
      "HIDIdleTime" = 4513072000
      "HIDKeyRepeat" = 83333333
    }
"#;

    struct Fake {
        macos: bool,
        out: HashMap<&'static str, String>,
        env: Option<&'static str>,
    }

    impl Sys for Fake {
        fn macos(&self) -> bool {
            self.macos
        }
        fn output(&self, program: &str, _: &[&str]) -> Option<String> {
            self.out.get(program).cloned()
        }
        fn env(&self, _: &str) -> Option<String> {
            self.env.map(String::from)
        }
        fn now_ms(&self) -> i64 {
            1_000_000
        }
    }

    #[test]
    fn macos_idle_time_is_read_from_the_hid_listing() {
        assert_eq!(parse_hid_idle(IOREG), Some(4513));
        assert_eq!(parse_hid_idle("no such key here"), None);
        assert_eq!(parse_hid_idle("\"HIDIdleTime\" = soon"), None);
    }

    #[test]
    fn linux_idle_time_is_read_from_the_session_hint() {
        let idle = "IdleHint=yes\nIdleSinceHint=940000000\nIdleSinceHintMonotonic=0\n";
        assert_eq!(parse_idle_hint(idle, 1_000_000), Some(60_000));
        assert_eq!(
            parse_idle_hint("IdleHint=no\nIdleSinceHint=0\n", 5),
            Some(0)
        );
        // idle with no start time, no hint at all, and nonsense are all "cannot say"
        assert_eq!(parse_idle_hint("IdleHint=yes\nIdleSinceHint=0\n", 5), None);
        assert_eq!(parse_idle_hint("", 5), None);
        assert_eq!(parse_idle_hint("IdleHint=maybe\n", 5), None);
    }

    #[test]
    fn the_reader_follows_the_system() {
        let mac = Fake {
            macos: true,
            out: HashMap::from([("ioreg", IOREG.to_string())]),
            env: None,
        };
        assert_eq!(idle_ms(&mac), Some(4513));
        let linux = Fake {
            macos: false,
            out: HashMap::from([("loginctl", "IdleHint=no\nIdleSinceHint=0\n".to_string())]),
            env: Some("3"),
        };
        assert_eq!(idle_ms(&linux), Some(0));
    }

    #[test]
    fn a_system_that_cannot_say_is_unavailable() {
        let no_session = Fake {
            macos: false,
            out: HashMap::from([("loginctl", "IdleHint=no\n".to_string())]),
            env: None,
        };
        assert_eq!(idle_ms(&no_session), None);
        let no_tool = Fake {
            macos: false,
            out: HashMap::new(),
            env: Some("3"),
        };
        assert_eq!(idle_ms(&no_tool), None);
        let no_ioreg = Fake {
            macos: true,
            out: HashMap::new(),
            env: None,
        };
        assert_eq!(idle_ms(&no_ioreg), None);
    }
}
