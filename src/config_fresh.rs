//! Reading `config.toml` again in a long-running part (the daemon, the board), so a change made
//! after it started counts. A file that stops parsing (an editor saving halfway, a word typed
//! wrong) leaves the last settings that parsed in place, so a switch such as `[board] quiet` is not
//! undone behind your back, and the daemon log says why once.

use crate::config::Config;
use crate::paths::Paths;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// What this process last read from one config file.
#[derive(Default)]
struct Seen {
    /// The last settings that parsed.
    good: Option<Config>,
    /// The parse error last written to the log, until the file parses again.
    logged: Option<String>,
}

static SEEN: LazyLock<Mutex<HashMap<PathBuf, Seen>>> = LazyLock::new(Default::default);

impl Config {
    /// The settings as the file says now, for a long-running part (the daemon, the board) that
    /// should see a change made after it started. What the file does not hold (the folder root and
    /// the night folder and a folder given on the command line, set by the process that loaded it)
    /// is kept from `self`. A file that cannot be read or parsed gives the last settings this
    /// process read from it, else `self`; a parse error goes to the daemon log once.
    pub fn fresh(&self, path: &Path) -> Config {
        let read = std::fs::read_to_string(path)
            .ok()
            .map(|t| Config::parse(&t));
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        let entry = seen.entry(path.to_path_buf()).or_default();
        let mut c = match read {
            Some(Ok(c)) => {
                entry.good = Some(c.clone());
                entry.logged = None;
                c
            }
            Some(Err(e)) => {
                let why = format!("{e:#}");
                if entry.logged.as_deref() != Some(why.as_str()) {
                    note(path, &why);
                    entry.logged = Some(why);
                }
                entry.good.clone().unwrap_or_else(|| self.clone())
            }
            None => entry.good.clone().unwrap_or_else(|| self.clone()),
        };
        c.folder_root.clone_from(&self.folder_root);
        c.night_dir.clone_from(&self.night_dir);
        for (id, a) in &self.agents.each {
            if a.dir_override.is_some() {
                c.agents.each.entry(id.clone()).or_default().dir_override = a.dir_override.clone();
            }
        }
        c
    }
}

/// Say in the daemon log (in the eye home the file is in) that the file does not parse.
fn note(path: &Path, why: &str) {
    if let Some(home) = path.parent() {
        let paths = Paths {
            home: home.to_path_buf(),
        };
        crate::daemon::log(
            &paths,
            &format!(
                "{}: {why}; keeping the settings read before",
                path.display()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_stops_parsing_keeps_the_last_settings_that_did_and_is_logged_once() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let logged = || {
            std::fs::read_to_string(tmp.path().join("logs/daemon.log"))
                .unwrap_or_default()
                .matches("keeping the settings read before")
                .count()
        };
        let start = Config::parse("").unwrap();
        let quiet = |c: &Config| c.board.quiet.clone();
        // broken before it ever parsed here: the settings the process started with
        std::fs::write(&path, "[board\n").unwrap();
        assert!(quiet(&start.fresh(&path)).is_empty());
        assert_eq!(logged(), 1);

        std::fs::write(&path, "[board]\nquiet = [\"app\"]\n").unwrap();
        assert_eq!(quiet(&start.fresh(&path)), ["app"]);
        // half saved: quiet stays on, however often it is read, and the log says so once
        std::fs::write(&path, "[board]\nquiet = [\"app\"]\n[night\n").unwrap();
        for _ in 0..3 {
            assert_eq!(quiet(&start.fresh(&path)), ["app"]);
        }
        assert_eq!(logged(), 2);
        // another error is logged once more
        std::fs::write(&path, "[board]\nquiet = 7\n").unwrap();
        assert_eq!(quiet(&start.fresh(&path)), ["app"]);
        assert_eq!(logged(), 3);
        // fixed, the file counts again; broken again, it is logged again
        std::fs::write(&path, "[board]\nquiet = []\n").unwrap();
        assert!(quiet(&start.fresh(&path)).is_empty());
        std::fs::write(&path, "[board]\nquiet = 7\n").unwrap();
        assert!(quiet(&start.fresh(&path)).is_empty());
        assert_eq!(logged(), 4);
    }

    #[test]
    fn what_the_file_does_not_hold_is_kept_from_the_process() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[board]\nport = 7500\n").unwrap();
        let mut start = Config::with_claude_dir(Path::new("/given/claude"));
        start.folder_root = Some(PathBuf::from("/given/root"));
        let c = start.fresh(&path);
        assert_eq!(c.board.port, 7500);
        assert_eq!(c.folder_root, start.folder_root);
        assert_eq!(
            c.agent_cfg("claude").dir_override,
            Some(PathBuf::from("/given/claude"))
        );
    }
}
