//! Whether a due reminder shows a desktop notice. The file `config.toml` is the truth, read each
//! time it matters, so the daemon follows a change made by the CLI, the board or the picker at once.

use crate::config::{self, Config};
use crate::paths::Paths;
use anyhow::Result;

/// The `[remind] notify` setting now. A file that is missing or cannot be read leaves `loaded`, the
/// value the process started with.
pub fn is_on(paths: &Paths, loaded: bool) -> bool {
    std::fs::read_to_string(paths.config_path())
        .ok()
        .and_then(|text| Config::parse(&text).ok())
        .map_or(loaded, |cfg| cfg.remind.notify)
}

/// Turn the notices on or off, keeping the rest of the file as it is.
pub fn set(paths: &Paths, on: bool) -> Result<()> {
    paths.ensure_home()?;
    config::write_remind_notify(&paths.config_path(), on)
}

/// Can this machine show a notice at all?
pub fn available() -> bool {
    crate::platform::notify::available()
}

/// The line `eye remind notify` and `eye doctor` print.
pub fn describe(on: bool, available: bool) -> String {
    match (on, available) {
        (true, true) => "desktop notices for due reminders: on".into(),
        (true, false) => {
            "desktop notices for due reminders: on, but this machine has no program to show one"
                .into()
        }
        (false, _) => format!(
            "desktop notices for due reminders: off ({} remind notify on)",
            crate::pick::bin_name()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (tempfile::TempDir, Paths) {
        let t = tempfile::tempdir().unwrap();
        let p = Paths {
            home: t.path().join("home"),
        };
        (t, p)
    }

    #[test]
    fn without_a_file_the_loaded_value_stands() {
        let (_t, p) = home();
        assert!(!is_on(&p, false));
        assert!(is_on(&p, true));
    }

    #[test]
    fn a_change_in_the_file_is_seen_without_a_restart() {
        let (_t, p) = home();
        set(&p, true).unwrap();
        assert!(is_on(&p, false));
        set(&p, false).unwrap();
        assert!(!is_on(&p, true));
    }

    #[test]
    fn an_old_config_without_the_key_means_off() {
        let (_t, p) = home();
        p.ensure_home().unwrap();
        std::fs::write(p.config_path(), "[board]\nport = 7500\n").unwrap();
        assert!(!is_on(&p, false));
    }

    #[test]
    fn switching_with_a_config_that_does_not_parse_is_an_error() {
        let (_t, p) = home();
        p.ensure_home().unwrap();
        std::fs::write(p.config_path(), "[remind\nnotify = true").unwrap();
        let e = set(&p, false).unwrap_err();
        assert!(format!("{e:#}").contains("does not parse"), "{e:#}");
    }

    #[test]
    fn a_config_that_does_not_parse_leaves_the_loaded_value() {
        let (_t, p) = home();
        p.ensure_home().unwrap();
        std::fs::write(p.config_path(), "[remind\nnotify = true").unwrap();
        assert!(is_on(&p, true));
        assert!(!is_on(&p, false));
    }

    #[test]
    fn the_line_says_how_to_turn_notices_on_when_they_are_off() {
        assert!(describe(false, true).contains("remind notify on"));
        assert!(describe(true, true).ends_with(": on"));
        assert!(describe(true, false).contains("no program"));
    }
}
