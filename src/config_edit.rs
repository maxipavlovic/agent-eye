//! Rewriting `config.toml` in place: one lock for every writer, and no write of a file that would
//! not parse.

use crate::config::Config;
use anyhow::{bail, Context, Result};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a writer waits for another one that is rewriting the file.
const WAIT: Duration = Duration::from_secs(5);

/// The text of the config file, or the defaults when it is not there.
fn text_or_default(path: &Path) -> Result<String> {
    if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    } else {
        Ok(crate::config::default_toml())
    }
}

/// Read the file (the defaults when it is missing), let `edit` change the text and write it back,
/// all while holding `config.lock` (flock) next to it, so two writers never lose each other's
/// change. A result that does not parse is refused and the file stays as it was.
pub fn rewrite(path: &Path, edit: impl FnOnce(&str) -> String) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path.with_file_name("config.lock"))
        .context("opening config.lock")?;
    let until = Instant::now() + WAIT;
    // SAFETY: flock on a file descriptor we own; it is released when `lock` is dropped.
    while unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        if Instant::now() >= until {
            bail!("config.toml is being changed by another eye; try again");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let text = edit(&text_or_default(path)?);
    if Config::parse(&text).is_err() {
        bail!("config.toml does not parse, so it was not changed; fix it or move it away");
    }
    crate::files::write_private(path, &text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_that_does_not_parse_is_refused_and_the_file_stays() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.toml");
        std::fs::write(&p, "[remind\nnotify = true\n").unwrap();
        let e = rewrite(&p, |s| s.to_string()).unwrap_err();
        assert!(format!("{e:#}").contains("does not parse"), "{e:#}");
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "[remind\nnotify = true\n"
        );
    }

    #[test]
    fn a_missing_file_starts_from_the_defaults() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("home").join("config.toml");
        rewrite(&p, |s| s.replace("notify = false", "notify = true")).unwrap();
        assert!(
            Config::parse(&std::fs::read_to_string(&p).unwrap())
                .unwrap()
                .remind
                .notify
        );
    }

    #[test]
    fn writers_at_the_same_moment_keep_every_change() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.toml");
        std::fs::write(&p, "[board]\nport = 7500\n").unwrap();
        let hands: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    rewrite(&p, |s| format!("{s}# {i}\n")).unwrap();
                })
            })
            .collect();
        for h in hands {
            h.join().unwrap();
        }
        let text = std::fs::read_to_string(&p).unwrap();
        for i in 0..8 {
            assert!(text.contains(&format!("# {i}\n")), "{text}");
        }
    }

    #[test]
    fn a_held_lock_makes_the_writer_give_up() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.toml");
        let held = std::fs::File::create(t.path().join("config.lock")).unwrap();
        // SAFETY: flock on a file descriptor we own.
        assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
        let e = rewrite(&p, |s| s.to_string()).unwrap_err();
        assert!(format!("{e:#}").contains("another eye"), "{e:#}");
    }
}
