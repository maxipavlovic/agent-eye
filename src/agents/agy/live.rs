//! Which agy conversations are open, and what they are doing.
//!
//! * Open: agy holds an exclusive `flock` on `presence/<id>.lock` for as long as a conversation is
//!   open in a process, idle or not. The files stay behind when agy exits, so a lock that can be
//!   taken is a conversation that is not open. (Checked on agy 1.5.14: held from the start of a
//!   session, released when it quits.)
//! * Busy or idle: the conversation's row in `conversation_summaries.db` says
//!   `CASCADE_RUN_STATUS_RUNNING` while a turn runs and `..._IDLE` after it.
//! * Waiting on you: while agy asks to approve a tool call, that step is the last row of the
//!   conversation's `steps` table and its status is 9 (`CORTEX_STEP_STATUS_WAITING`); it goes to 3
//!   (done) when you answer. The JSON log writes a step only when it is done, so only the
//!   database shows it. The summaries row says `RUNNING` throughout.

use super::files::{self, Summary};
use crate::agents::LiveSession;
use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::path::Path;

const PRESENCE: &str = "presence";
/// `CORTEX_STEP_STATUS_WAITING`: seen on a tool call that waits for approval.
pub const STEP_WAITING: i64 = 9;
/// The step type of a tool's result (`GENERIC`), the one that waits for approval.
const STEP_TOOL: i64 = 132;

/// Does a process hold the presence lock of this conversation? A shared, non-blocking lock is tried
/// and let go at once: it succeeds only when nobody holds the exclusive one.
pub fn lock_held(dir: &Path, id: &str) -> bool {
    let Ok(f) = File::open(dir.join(PRESENCE).join(format!("{id}.lock"))) else {
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

/// What the last step of the conversation's database says it waits for: `None` when it waits for
/// nothing.
pub fn waiting_for(folder: &Path) -> Option<&'static str> {
    let c = files::open_ro(&files::db_of(folder)?)?;
    let (step_type, status): (i64, i64) = c
        .query_row(
            "SELECT step_type, status FROM steps ORDER BY idx DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    (status == STEP_WAITING).then_some(if step_type == STEP_TOOL {
        "permission prompt"
    } else {
        "a question"
    })
}

/// Does a process hold the presence lock of any conversation? Then agy is running, and the files
/// it shares between conversations (history, caches) are not ours to edit.
pub fn any_lock_held(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir.join(PRESENCE)) else {
        return false;
    };
    rd.flatten().any(|e| {
        e.file_name()
            .to_str()
            .and_then(|n| n.strip_suffix(".lock"))
            .is_some_and(|id| lock_held(dir, id))
    })
}

/// The open conversations of agy: those whose presence lock is held, that are not sub-agents.
pub fn sessions(dir: &Path, sums: &HashMap<String, Summary>) -> Vec<LiveSession> {
    let Ok(rd) = std::fs::read_dir(dir.join(PRESENCE)) else {
        return vec![];
    };
    let mut ids: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let id = name.strip_suffix(".lock")?;
            crate::util::valid_session_id(id).then(|| id.to_ascii_lowercase())
        })
        .collect();
    ids.sort();
    let mut out = vec![];
    for id in ids {
        let Some(s) = sums.get(&id) else { continue };
        if s.nested || !lock_held(dir, &id) {
            continue;
        }
        let folder = dir.join(files::BRAIN).join(&id);
        let waiting = waiting_for(&folder);
        let status = match (waiting, s.running) {
            (Some(_), _) => "waiting",
            (None, true) => "busy",
            (None, false) => "idle",
        };
        out.push(LiveSession {
            pid: 0,
            cwd: s.folder.clone(),
            status: Some(status.into()),
            updated_at: std::fs::metadata(files::log_of(&folder))
                .ok()
                .map(|m| crate::util::mtime_ms(&m)),
            // `sdk-cli` is the entrypoint eye calls a headless run; agy's log says which `-p` ran
            entrypoint: Some(
                if super::cli_log::headless(dir, &id) {
                    "sdk-cli"
                } else {
                    "cli"
                }
                .into(),
            ),
            alive: true,
            waiting_for: waiting.map(String::from),
            remote_url: super::cli_log::remote_url(dir, &id),
            loops: vec![],
            session_id: id,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_nobody_holds_is_not_a_live_conversation() {
        let t = tempfile::tempdir().unwrap();
        let id = "aaaaaaaa-1111-2222-3333-444444444444";
        std::fs::create_dir_all(t.path().join(PRESENCE)).unwrap();
        let p = t.path().join(PRESENCE).join(format!("{id}.lock"));
        std::fs::write(&p, "").unwrap();
        assert!(!lock_held(t.path(), id));
        let held = File::create(&p).unwrap();
        held.lock().unwrap();
        assert!(lock_held(t.path(), id));
        let mut sums = HashMap::new();
        sums.insert(id.to_string(), Summary::default());
        let live = sessions(t.path(), &sums);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].status.as_deref(), Some("idle"));
        sums.get_mut(id).unwrap().running = true;
        assert_eq!(sessions(t.path(), &sums)[0].status.as_deref(), Some("busy"));
        sums.get_mut(id).unwrap().nested = true;
        assert!(sessions(t.path(), &sums).is_empty());
        drop(held);
        assert!(sessions(t.path(), &sums).is_empty());
    }
}
