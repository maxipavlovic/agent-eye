//! Which Codex sessions are open, and whether they are working. Codex keeps no file that says so.
//! What it does is hold an exclusive lock on `thread-writer-locks/<thread id>.lock` for as long as
//! a session is open (also while it is idle), and write `task_started` to the rollout when a
//! turn begins and `task_complete` or `turn_aborted` when it ends.

use super::files::Index;
use crate::agents::LiveSession;
use std::fs::{File, TryLockError};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const LOCKS_DIR: &str = "thread-writer-locks";
/// How much of the end of a rollout is read at a time to find the last turn marker.
const CHUNK: u64 = 256 << 10;
/// How far back the end of a rollout is searched for it.
const SEARCH_MAX: u64 = 32 << 20;
/// The markers, and whether a turn is open after each. Codex writes `task_started` and
/// `task_complete` and reads `turn_started` and `turn_complete` as the same events.
const MARKERS: [(&[u8], bool); 5] = [
    (b"\"type\":\"task_started\"", true),
    (b"\"type\":\"turn_started\"", true),
    (b"\"type\":\"task_complete\"", false),
    (b"\"type\":\"turn_complete\"", false),
    (b"\"type\":\"turn_aborted\"", false),
];
/// The longest marker, in bytes.
const MARKER_MAX: u64 = 24;

/// Does a process hold the writer lock of this thread? A shared, non-blocking lock is tried and let
/// go at once: it succeeds only when nobody holds the exclusive one. Only the lock files that are
/// there are tried, a handful at most. Codex takes the lock without waiting, so a probe that lands
/// in the instant it does (a session being resumed) could make it say the thread has a writer; the
/// window is as long as one system call.
pub fn writer_lock_held(dir: &Path, thread_id: &str) -> bool {
    let Ok(f) = File::open(dir.join(LOCKS_DIR).join(format!("{thread_id}.lock"))) else {
        return false;
    };
    match f.try_lock_shared() {
        Ok(()) => {
            let _ = f.unlock();
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => false,
    }
}

fn last_position(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Is a turn open at the end of the rollout: its last marker is `task_started`? A rollout with no
/// marker (a thread without a turn yet) has none open.
pub fn turn_open(path: &Path) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return false;
    };
    let (mut end, floor) = (len, len.saturating_sub(SEARCH_MAX));
    let mut buf = Vec::new();
    while end > floor {
        let start = end.saturating_sub(CHUNK).max(floor);
        buf.clear();
        buf.resize((end - start) as usize, 0);
        if f.seek(SeekFrom::Start(start)).is_err() || f.read_exact(&mut buf).is_err() {
            return false;
        }
        let last = MARKERS
            .iter()
            .filter_map(|(m, open)| Some((last_position(&buf, m)?, *open)))
            .max_by_key(|(at, _)| *at);
        if let Some((_, open)) = last {
            return open;
        }
        if start == floor {
            break;
        }
        // go back a chunk, keeping the bytes where a marker may have been cut in two
        end = start + MARKER_MAX;
    }
    false
}

/// The sessions whose writer lock is held, busy when a turn is open and idle otherwise. A session
/// with an active goal is in a loop. Codex has no per-session remote control or waiting reason.
pub fn sessions(dir: &Path, index: &Index) -> Vec<LiveSession> {
    let Ok(rd) = std::fs::read_dir(dir.join(LOCKS_DIR)) else {
        return vec![];
    };
    let mut ids: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let id = name.strip_suffix(".lock")?;
            (!id.starts_with('.')).then(|| id.to_ascii_lowercase())
        })
        .collect();
    ids.sort();
    let mut goals = super::goals::active(dir);
    let mut out = vec![];
    for id in ids {
        let Some(s) = index.sessions.get(&id) else {
            continue;
        };
        if !writer_lock_held(dir, &id) {
            continue;
        }
        out.push(LiveSession {
            pid: 0,
            cwd: s.cwd.clone(),
            status: Some(if turn_open(&s.path) { "busy" } else { "idle" }.into()),
            updated_at: std::fs::metadata(&s.path)
                .ok()
                .map(|m| crate::util::mtime_ms(&m)),
            entrypoint: Some("cli".into()),
            alive: true,
            waiting_for: None,
            remote_url: None,
            loops: goals.remove(&id).into_iter().collect(),
            session_id: id,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::codex::files::Visible;
    use std::io::Write;

    fn marker(kind: &str) -> String {
        format!(
            "{{\"timestamp\":\"t\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"{kind}\",\"turn_id\":\"t1\"}}}}\n"
        )
    }

    fn rollout(dir: &Path, text: &str) -> std::path::PathBuf {
        let p = dir.join("rollout.jsonl");
        std::fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn a_turn_is_open_after_task_started_and_closed_after_it_ends() {
        let t = tempfile::tempdir().unwrap();
        let filler = "{\"type\":\"response_item\",\"payload\":{}}\n".repeat(3);
        let cases = [
            ("", false),
            (filler.as_str(), false),
            (&format!("{}{filler}", marker("task_started")), true),
            (
                &format!(
                    "{}{filler}{}{filler}",
                    marker("task_started"),
                    marker("task_complete")
                ),
                false,
            ),
            (
                &format!(
                    "{}{}{}",
                    marker("task_complete"),
                    marker("task_started"),
                    filler
                ),
                true,
            ),
            (
                &format!("{}{}", marker("task_started"), marker("turn_aborted")),
                false,
            ),
            (
                &format!("{}{}", marker("turn_started"), marker("turn_complete")),
                false,
            ),
            (&marker("turn_started"), true),
        ];
        for (text, open) in cases {
            assert_eq!(turn_open(&rollout(t.path(), text)), open, "{text:?}");
        }
        assert!(!turn_open(&t.path().join("missing.jsonl")));
    }

    #[test]
    fn a_marker_far_from_the_end_and_across_a_chunk_boundary_is_found() {
        let t = tempfile::tempdir().unwrap();
        // a turn that began, then more output than one chunk
        let big = format!(
            "{}{}",
            marker("task_started"),
            "{\"type\":\"response_item\",\"payload\":\"x\"}\n".repeat(20_000)
        );
        assert!(big.len() as u64 > 2 * CHUNK);
        assert!(turn_open(&rollout(t.path(), &big)));
        // the marker cut in two by the boundary of the last chunk
        let m = marker("task_started");
        let pattern_at = m.find("\"type\":\"task_started\"").unwrap();
        let before = 100;
        let after = CHUNK as usize + pattern_at + 10 - m.len();
        let text = format!("{}\n{m}{}", "x".repeat(before - 1), "y".repeat(after));
        let boundary = text.len() - CHUNK as usize;
        assert!(boundary > before + pattern_at && boundary < before + pattern_at + 20);
        assert!(turn_open(&rollout(t.path(), &text)));
        // a marker quoted inside a tool result is escaped and does not count
        let quoted = format!(
            "{}{{\"payload\":\"{{\\\"type\\\":\\\"task_started\\\"}}\"}}\n",
            marker("task_complete")
        );
        assert!(!turn_open(&rollout(t.path(), &quoted)));
    }

    #[test]
    fn a_session_is_live_while_a_process_holds_its_writer_lock() {
        let t = tempfile::tempdir().unwrap();
        let id = "019a6b2e-7c10-7a41-9d3e-5f0c2b7a1e44";
        let busy = rollout(t.path(), &marker("task_started"));
        let mut index = Index::default();
        index.sessions.insert(
            id.into(),
            Visible {
                path: busy.clone(),
                cwd: Some("/w".into()),
            },
        );
        let locks = t.path().join(LOCKS_DIR);
        std::fs::create_dir_all(&locks).unwrap();
        let lock = locks.join(format!("{id}.lock"));
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)
            .unwrap();
        f.write_all(b"").unwrap();
        std::fs::write(locks.join(".coordination.lock"), "").unwrap();
        // a lock file nobody holds: the session is not open
        assert!(!writer_lock_held(t.path(), id));
        assert!(sessions(t.path(), &index).is_empty());
        // the owner takes it
        f.try_lock().unwrap();
        assert!(writer_lock_held(t.path(), id));
        let live = sessions(t.path(), &index);
        assert_eq!(live.len(), 1);
        assert_eq!(
            (
                live[0].session_id.as_str(),
                live[0].status.as_deref(),
                live[0].alive
            ),
            (id, Some("busy"), true)
        );
        assert_eq!(live[0].cwd.as_deref(), Some("/w"));
        // the turn ends: idle, still open
        std::fs::write(
            &busy,
            format!("{}{}", marker("task_started"), marker("task_complete")),
        )
        .unwrap();
        assert_eq!(
            sessions(t.path(), &index)[0].status.as_deref(),
            Some("idle")
        );
        // a lock of a thread eye shows no card for is left alone
        let other = locks.join("019a6b2e-0000-7000-8000-000000000009.lock");
        std::fs::write(&other, "").unwrap();
        assert_eq!(sessions(t.path(), &index).len(), 1);
        // the owner lets go
        f.unlock().unwrap();
        assert!(sessions(t.path(), &index).is_empty());
        // no lock folder at all
        assert!(sessions(&t.path().join("nowhere"), &index).is_empty());
    }
}
