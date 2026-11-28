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

/// Where the value that starts at byte `from` of `lines[first]` ends: the last line it occupies,
/// and the comment after it on that line. A list or table that is not closed on its first line
/// runs through the line that closes it. Strings are skipped, so a bracket or a `#` inside one
/// counts for nothing, and so do the comments between the elements of a list. A value that never
/// closes runs to the last line.
pub(crate) fn value_end<'a>(
    lines: &[&'a str],
    first: usize,
    from: usize,
) -> (usize, Option<&'a str>) {
    let (mut depth, mut multi, mut at, mut start) = (0i32, None::<&[u8]>, first, from);
    loop {
        let line = lines[at];
        let b = line.as_bytes();
        let mut comment = None;
        let mut p = start;
        while p < b.len() {
            if let Some(close) = multi {
                if b[p..].starts_with(close) {
                    multi = None;
                    p += 3;
                } else {
                    p += if close[0] == b'"' && b[p] == b'\\' {
                        2
                    } else {
                        1
                    };
                }
                continue;
            }
            match b[p] {
                q @ (b'"' | b'\'') => {
                    if b[p..].starts_with(&[q, q, q]) {
                        multi = Some(if q == b'"' { b"\"\"\"" } else { b"\'\'\'" });
                        p += 3;
                        continue;
                    }
                    p += 1;
                    while p < b.len() && b[p] != q {
                        p += if q == b'"' && b[p] == b'\\' { 2 } else { 1 };
                    }
                    p += 1;
                }
                b'[' | b'{' => {
                    depth += 1;
                    p += 1;
                }
                b']' | b'}' => {
                    depth -= 1;
                    p += 1;
                }
                b'#' => {
                    comment = Some(&line[p..]);
                    break;
                }
                _ => p += 1,
            }
        }
        if (multi.is_none() && depth <= 0) || at + 1 == lines.len() {
            return (at, comment);
        }
        at += 1;
        start = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(text: &str, key_line: usize) -> (usize, Option<&str>) {
        let lines: Vec<&str> = text.lines().collect();
        let from = lines[key_line].find('=').unwrap() + 1;
        value_end(&lines, key_line, from)
    }

    #[test]
    fn a_value_ends_where_its_brackets_and_strings_do() {
        assert_eq!(end("a = 1", 0), (0, None));
        assert_eq!(end("a = 1  # one", 0), (0, Some("# one")));
        assert_eq!(end("a = [\"x\", \"y\"]  # two", 0), (0, Some("# two")));
        assert_eq!(end("a = [\n  \"x\",\n]\nb = 1", 0), (2, None));
        assert_eq!(end("a = [\n]\n", 0), (1, None));
        // brackets and # inside strings, and comments between the elements
        assert_eq!(
            end("a = [\"]\", '[#', \"q\\\"]\"]  # c", 0),
            (0, Some("# c"))
        );
        assert_eq!(
            end("a = [\n  \"x\", # ]\n  # [\n  \"#]\",\n]  # end\nb = 2", 0),
            (4, Some("# end"))
        );
        assert_eq!(end("a = {\n  k = 1,\n}\nb = 2", 0), (2, None));
        // a multi-line string closes on its own line
        assert_eq!(end("a = \"\"\"x\n]\n\"\"\"\nb = 1", 0), (2, None));
        // a value that never closes runs to the end
        assert_eq!(end("a = [\n  \"x\"\n", 0), (1, None));
    }

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
    fn a_list_on_several_lines_is_saved_over_like_any_other() {
        // what the board's settings panel and `eye board on`, `eye night on` do to such a file
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.toml");
        std::fs::write(
            &p,
            "[agents]\nenabled = [\n  \"claude\",\n  \"codex\",\n]\n\n[board]\nquiet = [\n]\n",
        )
        .unwrap();
        crate::config::write_keys(
            &p,
            &[
                ("[board]", "quiet", "[\"app\"]".to_string()),
                ("[agents]", "enabled", "[\"claude\"]".to_string()),
            ],
        )
        .unwrap();
        crate::config::write_board_enabled(&p, true).unwrap();
        let c = Config::parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(c.board.quiet, ["app"]);
        assert_eq!(c.enabled_agent_ids(), ["claude"]);
        assert!(c.board.enabled);
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
