//! `eye daemon`: one process that drains hook events, keeps the index and live status fresh,
//! summarizes and archives quiet sessions, and serves the board when it is turned on.

use crate::agents::{self, Agents, Wait, LEGACY_ID};
use crate::archive;
use crate::backup;
use crate::config::Config;
use crate::db::{meta_set, Db};
use crate::hook;
use crate::paths::Paths;
use crate::scan;
use crate::server;
use crate::summarize;
use crate::util::{now_ms, short_id};
use anyhow::{Context, Result};
use rusqlite::{params, TransactionBehavior};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_secs(2);
const WORKER_EVERY: Duration = Duration::from_secs(30);
/// How often the daemon looks whether a daily snapshot is due.
const BACKUP_EVERY: Duration = Duration::from_secs(600);
const LOG_MAX: u64 = 10 << 20;
const QUEUE_ROTATE: u64 = 1 << 20;
const SUMMARIES_PER_ROUND: usize = 3;
const EVENT_KEEP_MS: i64 = 30 * 24 * 3600 * 1000;

/// Holds `daemon.lock` (flock) for the life of the process.
pub struct Lock {
    _file: std::fs::File,
}

impl Lock {
    /// Take the lock, or fail when another daemon holds it.
    pub fn acquire(paths: &Paths) -> Result<Lock> {
        let p = paths.home.join("daemon.lock");
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&p)
            .with_context(|| format!("opening {}", p.display()))?;
        // SAFETY: flock on a file descriptor we own.
        let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if r != 0 {
            let mut other = String::new();
            let _ = f.read_to_string(&mut other);
            anyhow::bail!("the daemon is already running (pid {})", other.trim());
        }
        f.set_len(0)?;
        f.seek(SeekFrom::Start(0))?;
        write!(f, "{}", std::process::id())?;
        Ok(Lock { _file: f })
    }
}

/// Is a daemon holding the lock? Reads the pid and checks it is alive.
pub fn running_pid(paths: &Paths) -> Option<i64> {
    let txt = std::fs::read_to_string(paths.home.join("daemon.lock")).ok()?;
    let pid: i64 = txt.trim().parse().ok()?;
    crate::util::pid_alive(pid).then_some(pid)
}

/// Append a line to `logs/daemon.log` (rotated at 10 MB).
pub fn log(paths: &Paths, msg: &str) {
    let dir = paths.home.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join("daemon.log");
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = std::fs::rename(&p, dir.join("daemon.log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&p)
    {
        let _ = writeln!(
            f,
            "{} {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            msg
        );
    }
}

/// The wait after an event, given the wait the earlier events left (None: none known). A
/// notification follows the PermissionRequest of the same prompt and says less: it keeps the note
/// already there. An event that only ends a prompt leaves any other wait alone.
fn fold_wait(cur: Option<Wait>, kind: &str, w: Wait) -> Option<Wait> {
    if w.only_if_prompt {
        return cur.map(|c| if c.prompt && c.since.is_some() { w } else { c });
    }
    Some(match cur {
        Some(c) if kind == "Notification" && c.since.is_some() => Wait {
            note: c.note.or(w.note),
            ..w
        },
        _ => w,
    })
}

/// Recompute the wait of every session marked waiting from its stored hook events (since the last
/// prompt, start or end): fixes waits recorded under older rules and fills in their notes.
pub fn refresh_waits(db: &Db, agents: &Agents) -> Result<usize> {
    let ids: Vec<(String, String)> = {
        let mut st = db
            .conn
            .prepare("SELECT id, agent FROM session WHERE waiting_since IS NOT NULL")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut st = db.conn.prepare(
        "SELECT kind, ts, payload FROM event WHERE session_id=?1
           AND ts >= COALESCE((SELECT MAX(ts) FROM event WHERE session_id=?1
                              AND kind IN ('SessionStart','UserPromptSubmit','SessionEnd')), 0)
         ORDER BY ts, id",
    )?;
    let mut n = 0;
    for (id, agent_id) in ids {
        let Some(agent) = agents.get(&agent_id) else {
            continue;
        };
        let mut cur: Option<Wait> = None;
        let rows = st.query_map([&id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (kind, ts, payload) = row?;
            let Ok(ev) = serde_json::from_str::<Value>(&payload) else {
                continue;
            };
            if let Some(w) = agent.wait_effect(&kind, &ev, ts) {
                cur = fold_wait(cur, &kind, w);
            }
        }
        let Some(w) = cur else { continue };
        n += db.conn.execute(
            "UPDATE session SET waiting_since=?2, waiting_note=?3, waiting_prompt=?4 WHERE id=?1",
            params![id, w.since, w.note, w.prompt],
        )?;
    }
    Ok(n)
}

/// Apply the effect of one queue line on the session row.
fn apply_event(db: &Db, agents: &Agents, ev: &Value) -> Result<()> {
    let kind = ev.get("event").and_then(Value::as_str).unwrap_or("");
    let ts = ev.get("ts").and_then(Value::as_i64).unwrap_or_else(now_ms);
    let sid = ev.get("session_id").and_then(Value::as_str);
    db.conn.execute(
        "INSERT INTO event(session_id, ts, kind, payload) VALUES(?1,?2,?3,?4)",
        params![sid, ts, kind, ev.to_string()],
    )?;
    let Some(sid) = sid else { return Ok(()) };
    // An event of an agent that is not enabled is kept in the log and changes nothing.
    let agent_id = ev.get("agent").and_then(Value::as_str).unwrap_or(LEGACY_ID);
    let Some(agent) = agents.get(agent_id) else {
        return Ok(());
    };
    if let Some(w) = agent.wait_effect(kind, ev, ts) {
        // `?5` limits the change to a wait that is a prompt
        let sql = if kind == "Notification" {
            "UPDATE session SET waiting_since=?2, waiting_note=CASE WHEN waiting_since IS NULL
               THEN ?3 ELSE COALESCE(waiting_note, ?3) END, waiting_prompt=?4
             WHERE id=?1 AND (NOT ?5 OR waiting_prompt=1)"
        } else {
            "UPDATE session SET waiting_since=?2, waiting_note=?3, waiting_prompt=?4
             WHERE id=?1 AND (NOT ?5 OR waiting_prompt=1)"
        };
        db.conn.execute(
            sql,
            params![sid, w.since, w.note, w.prompt, w.only_if_prompt],
        )?;
    }
    if kind == "SessionEnd" {
        db.conn.execute(
            "UPDATE session SET ended_at=?2 WHERE id=?1",
            params![sid, ts],
        )?;
    }
    Ok(())
}

/// Read what the hooks queued since the last pass. Returns the number of events.
pub fn drain_queue(db: &Db, paths: &Paths, agents: &Agents) -> Result<usize> {
    let p = hook::queue_path(paths);
    let Ok(mut f) = std::fs::File::open(&p) else {
        return Ok(0);
    };
    let size = f.metadata()?.len();
    let mut offset: u64 = db
        .meta_get("queue_offset")?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if offset > size {
        offset = 0;
    }
    if offset == size {
        if size > QUEUE_ROTATE {
            let _ = std::fs::rename(&p, p.with_extension("jsonl.old"));
            meta_set(&db.conn, "queue_offset", "0")?;
        }
        return Ok(0);
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = Vec::new();
    f.take(size - offset).read_to_end(&mut buf)?;
    let mut n = 0;
    let mut used = 0usize;
    let mut newest: Option<i64> = None;
    let tx = rusqlite::Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
    while let Some(i) = buf[used..].iter().position(|&b| b == b'\n') {
        let line = &buf[used..used + i];
        used += i + 1;
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        apply_event(db, agents, &v)?;
        newest = newest.max(v.get("ts").and_then(Value::as_i64));
        n += 1;
    }
    meta_set(
        &db.conn,
        "queue_offset",
        &(offset + used as u64).to_string(),
    )?;
    if let Some(t) = newest {
        meta_set(&db.conn, "last_hook_ts", &t.to_string())?;
    }
    tx.commit()?;
    Ok(n)
}

/// Sessions with no activity for `quiet_minutes`, or whose SessionEnd arrived after their last
/// activity, and that are not busy.
pub fn quiet_sessions(db: &Db, cfg: &Config, now: i64) -> Result<Vec<(String, i64, bool)>> {
    let quiet_ms = cfg.summary.quiet_minutes as i64 * 60_000;
    let mut st = db.conn.prepare(
        "SELECT s.id, s.last_activity_at,
                (s.summary_at IS NULL OR s.last_activity_at > s.summary_at) AS needs_summary
         FROM session s
         WHERE s.transcript_state='present' AND s.kind IS NOT 'history_only'
           AND s.live_status IS NOT 'busy' AND s.last_activity_at IS NOT NULL
           AND (?1 - s.last_activity_at >= ?2
                OR EXISTS(SELECT 1 FROM event e WHERE e.session_id=s.id AND e.kind='SessionEnd'
                          AND e.ts >= s.last_activity_at))
         ORDER BY s.last_activity_at DESC",
    )?;
    let rows = st.query_map(params![now, quiet_ms], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// One worker round: summarize recent quiet sessions (when enabled), then archive every quiet
/// session whose transcript changed.
pub fn quiet_round(db: &mut Db, paths: &Paths, cfg: &Config) -> Result<(usize, usize)> {
    let now = now_ms();
    let quiet = quiet_sessions(db, cfg, now)?;
    let mut summarized = 0;
    if cfg.summary.enabled {
        let horizon = now - cfg.summary.auto_days as i64 * 24 * 3600 * 1000;
        let mut ids = vec![];
        for (id, last, needs) in &quiet {
            if !needs || *last < horizon || ids.len() >= SUMMARIES_PER_ROUND {
                continue;
            }
            let key = format!("quiet_tried:{id}");
            if db.meta_get(&key)?.as_deref() == Some(last.to_string().as_str()) {
                continue;
            }
            meta_set(&db.conn, &key, &last.to_string())?;
            ids.push(id.clone());
        }
        if !ids.is_empty() {
            let out = summarize::summarize(
                db,
                paths,
                cfg,
                &summarize::Options {
                    ids: ids.clone(),
                    all: false,
                    dry_run: false,
                    max: None,
                    force: false,
                    print_excerpt: false,
                },
            )?;
            for o in &out {
                log(paths, &format!("summary {} {}", short_id(&o.id), o.result));
            }
            summarized = out.iter().filter(|o| o.summary.is_some()).count();
        }
    }
    let mut archived = 0;
    let agents = agents::enabled(cfg);
    for (id, _, _) in &quiet {
        match archive::archive_session(db, paths, &agents, id) {
            Ok(archive::Archived::Copied(n)) => {
                archived += 1;
                log(paths, &format!("archived {} ({n} bytes)", short_id(id)));
            }
            Ok(_) => {}
            Err(e) => log(paths, &format!("archive {id}: {e:#}")),
        }
    }
    Ok((summarized, archived))
}

/// Tell about the reminders that came due, once each, unless `[remind] notify` is off.
fn notify_reminders(db: &Db, paths: &Paths, cfg: &Config, send: &mut dyn FnMut(&str, &str)) {
    if !cfg.remind.notify {
        return;
    }
    match crate::remind::notify_due(db, now_ms(), send) {
        Ok(0) => {}
        Ok(n) => log(paths, &format!("tick: {n} reminders due")),
        Err(e) => log(paths, &format!("reminders: {e:#}")),
    }
}

/// One pass of the main loop: events, then the incremental scan, then due reminders.
pub fn tick(db: &mut Db, paths: &Paths, cfg: &Config) -> Result<()> {
    let n = drain_queue(db, paths, &agents::enabled(cfg))?;
    let st = scan::scan(db, cfg, false)?;
    if n > 0 || st.files_parsed > 0 {
        log(
            paths,
            &format!(
                "tick: {n} events, {} transcripts changed, {} ms",
                st.files_parsed, st.elapsed_ms
            ),
        );
    }
    notify_reminders(db, paths, cfg, &mut |title, body| {
        crate::platform::notify::send(title, body);
    });
    let cutoff = now_ms() - EVENT_KEEP_MS;
    if n == 0
        && db.meta_get("event_pruned_day")?.as_deref() != Some(&(cutoff / 86_400_000).to_string())
    {
        db.conn.execute(
            "DELETE FROM event WHERE kind!='summary' AND ts < ?1",
            [cutoff],
        )?;
        meta_set(
            &db.conn,
            "event_pruned_day",
            &(cutoff / 86_400_000).to_string(),
        )?;
    }
    Ok(())
}

/// Run until SIGINT/SIGTERM. With `board` and `board.enabled`, the board is served from this process.
pub fn run(paths: Paths, cfg: Config, port: u16, board: bool) -> Result<()> {
    paths.ensure_home()?;
    let _lock = Lock::acquire(&paths)?;
    log(&paths, &format!("daemon start pid {}", std::process::id()));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_busy = Arc::new(AtomicBool::new(false));
    let handle = {
        let (paths, cfg, stop, busy) = (paths.clone(), cfg.clone(), stop.clone(), worker_busy);
        std::thread::spawn(move || {
            let mut db = match Db::open(&paths.db_path()) {
                Ok(d) => d,
                Err(e) => {
                    log(&paths, &format!("cannot open db: {e:#}"));
                    return;
                }
            };
            if let Err(e) = refresh_waits(&db, &agents::enabled(&cfg)) {
                log(&paths, &format!("waiting state: {e:#}"));
            }
            let mut last_worker: Option<Instant> = None;
            let mut last_backup: Option<Instant> = None;
            while !stop.load(Ordering::Relaxed) {
                if let Err(e) = tick(&mut db, &paths, &cfg) {
                    log(&paths, &format!("tick failed: {e:#}"));
                }
                if last_backup.is_none_or(|t| t.elapsed() >= BACKUP_EVERY) {
                    last_backup = Some(Instant::now());
                    match backup::daily_if_due(&db.conn, &paths, &cfg, now_ms()) {
                        Ok(Some(s)) => log(&paths, &format!("snapshot {}", s.path.display())),
                        Ok(None) => {}
                        Err(e) => log(&paths, &format!("snapshot failed: {e:#}")),
                    }
                }
                let due = last_worker.is_none_or(|t| t.elapsed() >= WORKER_EVERY);
                if due && !busy.swap(true, Ordering::SeqCst) {
                    last_worker = Some(Instant::now());
                    let (paths, cfg, busy) = (paths.clone(), cfg.clone(), busy.clone());
                    std::thread::spawn(move || {
                        let r = Db::open(&paths.db_path())
                            .and_then(|mut d| quiet_round(&mut d, &paths, &cfg));
                        if let Err(e) = r {
                            log(&paths, &format!("quiet round failed: {e:#}"));
                        }
                        busy.store(false, Ordering::SeqCst);
                    });
                }
                std::thread::sleep(TICK);
            }
        })
    };
    let wait_for_signal = || -> Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(server::shutdown_signal());
        Ok(())
    };
    let result = if board && cfg.board.enabled {
        let p = paths.clone();
        let served = server::serve(paths.clone(), cfg, port, move |url| {
            // The link carries the token; the log (which people attach to bug reports) must not.
            log(
                &p,
                &format!("board at {}", url.split('?').next().unwrap_or(url)),
            );
            Ok(())
        });
        match served {
            Ok(()) => Ok(()),
            Err(e) => {
                // A busy port must not stop the scanner and the hooks: keep indexing without the board.
                log(&paths, &format!("board not served: {e:#}"));
                wait_for_signal()
            }
        }
    } else {
        wait_for_signal()
    };
    stop.store(true, Ordering::Relaxed);
    let _ = handle.join();
    log(&paths, "daemon stop");
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::claude::Claude;
    use crate::agents::codex::Codex;
    use std::sync::Arc;

    /// Claude Code is the only agent the daemon tests need.
    fn agents() -> Agents {
        Agents::from_vec(vec![Arc::new(Claude::new("/nonexistent/c".into()))])
    }

    fn run_hook(p: &Paths, event: &str, payload: &[u8]) {
        hook::run(p, agents().first().unwrap().as_ref(), event, payload);
    }

    fn env() -> (tempfile::TempDir, Paths, Db) {
        let t = tempfile::tempdir().unwrap();
        let p = Paths {
            home: t.path().join("h"),
        };
        p.ensure_home().unwrap();
        let db = Db::open(&p.db_path()).unwrap();
        (t, p, db)
    }

    fn add_session(db: &Db, id: &str, last: i64) {
        db.conn
            .execute(
                "INSERT INTO session(id,kind,transcript_state,last_activity_at,prompt_count) VALUES(?1,'interactive','present',?2,3)",
                params![id, last],
            )
            .unwrap();
    }

    #[test]
    fn a_due_reminder_is_announced_once_and_not_when_notices_are_off() {
        let (_t, p, db) = env();
        add_session(&db, "s1", 100);
        db.conn
            .execute("UPDATE session SET remind_at=200, title='Ana'", [])
            .unwrap();
        let seen = std::cell::RefCell::new(vec![]);
        let mut send = |t: &str, _: &str| seen.borrow_mut().push(t.to_string());
        let mut off = Config::default();
        off.remind.notify = false;
        notify_reminders(&db, &p, &off, &mut send);
        assert!(seen.borrow().is_empty());
        let on = Config::default();
        notify_reminders(&db, &p, &on, &mut send);
        notify_reminders(&db, &p, &on, &mut send);
        assert_eq!(*seen.borrow(), ["Check on: Ana"]);
    }

    #[test]
    fn events_set_waiting_and_ended() {
        let (_t, p, db) = env();
        add_session(&db, "s1", 100);
        run_hook(
            &p,
            "Stop",
            br#"{"session_id":"s1","last_assistant_message":"Want me to continue?"}"#.as_slice(),
        );
        assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 1);
        let s = db.session_by_id("s1").unwrap().unwrap();
        assert!(s.waiting_since.is_some());
        assert_eq!(
            s.waiting_note.as_deref(),
            Some("answer: Want me to continue?")
        );
        run_hook(
            &p,
            "UserPromptSubmit",
            br#"{"session_id":"s1","prompt":"yes"}"#.as_slice(),
        );
        run_hook(
            &p,
            "Stop",
            br#"{"session_id":"s1","last_assistant_message":"All done."}"#.as_slice(),
        );
        run_hook(
            &p,
            "SessionEnd",
            br#"{"session_id":"s1","reason":"other"}"#.as_slice(),
        );
        assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 3);
        assert_eq!(
            drain_queue(&db, &p, &agents()).unwrap(),
            0,
            "offset is kept"
        );
        let s = db.session_by_id("s1").unwrap().unwrap();
        assert!(s.waiting_since.is_none() && s.ended_at.is_some());
        assert_eq!(s.waiting_note, None);
        let n: i64 = db
            .conn
            .query_row("SELECT count(*) FROM event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 4);
        assert!(db.meta_get("last_hook_ts").unwrap().is_some());
    }

    #[test]
    fn permission_request_and_garbage_lines() {
        let (_t, p, db) = env();
        add_session(&db, "s2", 100);
        run_hook(
            &p,
            "PermissionRequest",
            br#"{"session_id":"s2","tool_name":"Bash"}"#.as_slice(),
        );
        std::fs::OpenOptions::new()
            .append(true)
            .open(hook::queue_path(&p))
            .unwrap()
            .write_all(b"garbage\n")
            .unwrap();
        assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 1);
        assert!(db
            .session_by_id("s2")
            .unwrap()
            .unwrap()
            .waiting_since
            .is_some());
    }

    #[test]
    fn the_waiting_note_says_what_is_asked() {
        let (_t, p, db) = env();
        add_session(&db, "s3", 100);
        db.conn
            .execute("UPDATE session SET live_status='waiting' WHERE id='s3'", [])
            .unwrap();
        let note = |p: &Paths, db: &Db| {
            drain_queue(db, p, &agents()).unwrap();
            let s = db.session_by_id("s3").unwrap().unwrap();
            (s.waiting_for().map(str::to_string), s.next_line())
        };
        run_hook(&p, "PermissionRequest", br#"{"session_id":"s3","tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which repo first?"}]}}"#.as_slice());
        // the notification that follows keeps the precise note
        run_hook(&p, "Notification", br#"{"session_id":"s3","notification_type":"permission_prompt","message":"Claude needs your permission"}"#.as_slice());
        let (w, next) = note(&p, &db);
        assert_eq!(w.as_deref(), Some("answer: Which repo first?"));
        assert_eq!(next.as_deref(), Some("You: answer: Which repo first?"));
        assert!(db.session_by_id("s3").unwrap().unwrap().blocked());
        run_hook(&p, "UserPromptSubmit", br#"{"session_id":"s3"}"#.as_slice());
        run_hook(&p, "Stop", br#"{"session_id":"s3","last_assistant_message":"Tests pass. Merged it. Shall I also tag the release?"}"#.as_slice());
        assert_eq!(
            note(&p, &db).0.as_deref(),
            Some("answer: Shall I also tag the release?")
        );
        // a finished reply that asks something waits on you but does not block Claude
        assert!(!db.session_by_id("s3").unwrap().unwrap().blocked());
        run_hook(
            &p,
            "PermissionRequest",
            br#"{"session_id":"s3","tool_name":"Bash","tool_input":{"command":"git push"}}"#
                .as_slice(),
        );
        assert_eq!(note(&p, &db).0.as_deref(), Some("allow Bash: git push"));
        // working again (or closed): no note, the written next step shows
        db.conn
            .execute(
                "UPDATE session SET live_status='busy', next_step='Sam: review' WHERE id='s3'",
                [],
            )
            .unwrap();
        assert_eq!(note(&p, &db), (None, Some("Sam: review".into())));
        db.conn
            .execute("UPDATE session SET live_status=NULL WHERE id='s3'", [])
            .unwrap();
        assert_eq!(note(&p, &db).0, None);
        run_hook(&p, "Notification", br#"{"session_id":"s3","notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#.as_slice());
        db.conn
            .execute("UPDATE session SET live_status='waiting' WHERE id='s3'", [])
            .unwrap();
        // "waiting for your input" after a finished reply changes nothing
        assert_eq!(note(&p, &db).0.as_deref(), Some("allow Bash: git push"));
        run_hook(&p, "UserPromptSubmit", br#"{"session_id":"s3"}"#.as_slice());
        run_hook(
            &p,
            "Stop",
            br#"{"session_id":"s3","last_assistant_message":"Done."}"#.as_slice(),
        );
        run_hook(&p, "Notification", br#"{"session_id":"s3","notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#.as_slice());
        assert_eq!(note(&p, &db), (None, Some("Sam: review".into())));
        let s3 = db.session_by_id("s3").unwrap().unwrap();
        assert!(s3.waiting_since.is_none() && !s3.blocked());
    }

    #[test]
    fn stored_waits_are_recomputed_under_the_current_rules() {
        let (_t, _p, db) = env();
        let ev = |sid: &str, ts: i64, kind: &str, payload: &str| {
            db.conn
                .execute(
                    "INSERT INTO event(session_id, ts, kind, payload) VALUES(?1,?2,?3,?4)",
                    params![sid, ts, kind, payload],
                )
                .unwrap();
        };
        // a prompt mid-task, recorded before notes existed
        add_session(&db, "s4", 100);
        ev("s4", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
        ev("s4", 2, "UserPromptSubmit", "{}");
        ev(
            "s4",
            3,
            "PermissionRequest",
            r#"{"tool_name":"AskUserQuestion"}"#,
        );
        ev(
            "s4",
            4,
            "Notification",
            r#"{"notification_type":"permission_prompt"}"#,
        );
        // a finished reply, marked waiting by the idle notification under the old rule
        add_session(&db, "s5", 100);
        ev("s5", 5, "Stop", r#"{"last_message_tail":"All done."}"#);
        ev(
            "s5",
            6,
            "Notification",
            r#"{"notification_type":"idle_prompt"}"#,
        );
        db.conn
            .execute(
                "UPDATE session SET waiting_since=4, live_status='waiting' WHERE id='s4'",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE session SET waiting_since=6, waiting_note='reply to Claude', live_status='waiting' WHERE id='s5'",
                [],
            )
            .unwrap();
        assert_eq!(refresh_waits(&db, &agents()).unwrap(), 2);
        let s4 = db.session_by_id("s4").unwrap().unwrap();
        assert_eq!(
            (s4.waiting_for(), s4.blocked()),
            (Some("answer Claude's question in the session"), true)
        );
        let s5 = db.session_by_id("s5").unwrap().unwrap();
        assert_eq!(
            (s5.waiting_since, s5.waiting_for(), s5.blocked()),
            (None, None, false)
        );
        assert_eq!(
            refresh_waits(&db, &agents()).unwrap(),
            1,
            "only s4 still waits"
        );
    }

    fn codex_agents() -> Agents {
        Agents::from_vec(vec![Arc::new(Codex::new("/nonexistent/x".into()))])
    }

    fn codex_hook(p: &Paths, event: &str, payload: &[u8]) {
        hook::run(p, codex_agents().first().unwrap().as_ref(), event, payload);
    }

    #[test]
    fn a_tool_that_ran_ends_a_prompt_wait_and_not_a_question() {
        let (_t, p, db) = env();
        add_session(&db, "c1", 100);
        db.conn
            .execute("UPDATE session SET agent='codex' WHERE id='c1'", [])
            .unwrap();
        let wait = |db: &Db| {
            drain_queue(db, &p, &codex_agents()).unwrap();
            let s = db.session_by_id("c1").unwrap().unwrap();
            (s.waiting_note, s.waiting_prompt)
        };
        codex_hook(
            &p,
            "PermissionRequest",
            br#"{"session_id":"c1","tool_name":"Bash","tool_input":{"command":"git push"}}"#
                .as_slice(),
        );
        assert_eq!(wait(&db), (Some("allow Bash: git push".into()), true));
        codex_hook(
            &p,
            "PostToolUse",
            br#"{"session_id":"c1","tool_name":"Bash"}"#.as_slice(),
        );
        assert_eq!(
            wait(&db),
            (None, false),
            "the tool ran: the prompt was answered"
        );
        // a reply that asks is a wait of its own, and a tool that ran does not answer it
        codex_hook(
            &p,
            "Stop",
            br#"{"session_id":"c1","last_assistant_message":"Done. Shall I tag it?"}"#.as_slice(),
        );
        codex_hook(
            &p,
            "PostToolUse",
            br#"{"session_id":"c1","tool_name":"Bash"}"#.as_slice(),
        );
        assert_eq!(wait(&db), (Some("answer: Shall I tag it?".into()), false));
    }

    #[test]
    fn a_replay_applies_a_tool_that_ran_the_same_way() {
        let (_t, _p, db) = env();
        let ev = |sid: &str, ts: i64, kind: &str, payload: &str| {
            db.conn
                .execute(
                    "INSERT INTO event(session_id, ts, kind, payload) VALUES(?1,?2,?3,?4)",
                    params![sid, ts, kind, payload],
                )
                .unwrap();
        };
        // answered: nothing is waiting any more
        add_session(&db, "c2", 100);
        ev("c2", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
        ev("c2", 2, "PostToolUse", "{}");
        // a prompt, answered, then a reply that asks, then more tools: the question stays
        add_session(&db, "c3", 100);
        ev("c3", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
        ev("c3", 2, "PostToolUse", "{}");
        ev(
            "c3",
            3,
            "Stop",
            r#"{"last_message_tail":"Shall I tag it?"}"#,
        );
        ev("c3", 4, "PostToolUse", "{}");
        // still on the prompt
        add_session(&db, "c4", 100);
        ev("c4", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
        for id in ["c2", "c3", "c4"] {
            db.conn
                .execute(
                    "UPDATE session SET agent='codex', waiting_since=1, waiting_prompt=1, live_status='idle' WHERE id=?1",
                    [id],
                )
                .unwrap();
        }
        assert_eq!(refresh_waits(&db, &codex_agents()).unwrap(), 3);
        let got = |id: &str| {
            let s = db.session_by_id(id).unwrap().unwrap();
            (
                s.waiting_since,
                s.waiting_for().map(str::to_string),
                s.blocked(),
            )
        };
        assert_eq!(got("c2"), (None, None, false));
        assert_eq!(
            got("c3"),
            (Some(3), Some("answer: Shall I tag it?".into()), false)
        );
        assert_eq!(got("c4"), (Some(1), Some("allow Bash".into()), true));
    }

    #[test]
    fn quiet_by_time_or_session_end() {
        let (_t, p, db) = env();
        let now = 10_000_000;
        add_session(&db, "idle-old", now - 11 * 60_000);
        add_session(&db, "fresh", now - 60_000);
        add_session(&db, "ended", now - 120_000);
        db.conn
            .execute("INSERT INTO event(session_id,ts,kind,payload) VALUES('ended',?1,'SessionEnd','{}')", [now - 100_000])
            .unwrap();
        db.conn
            .execute("UPDATE session SET live_status='busy' WHERE id='fresh'", [])
            .unwrap();
        let q: Vec<String> = quiet_sessions(&db, &Config::default(), now)
            .unwrap()
            .into_iter()
            .map(|x| x.0)
            .collect();
        assert_eq!(q, vec!["ended", "idle-old"]);
        let _ = p;
    }

    #[test]
    fn lock_is_exclusive() {
        let (_t, p, _db) = env();
        let l = Lock::acquire(&p).unwrap();
        assert!(Lock::acquire(&p).is_err());
        assert_eq!(running_pid(&p), Some(std::process::id() as i64));
        drop(l);
        // a program another test starts at this moment holds a copy of the descriptor until it
        // execs, and the lock with it: wait that out
        let again = (0..200).find_map(|_| {
            Lock::acquire(&p).ok().or_else(|| {
                std::thread::sleep(std::time::Duration::from_millis(10));
                None
            })
        });
        assert!(again.is_some());
    }

    #[test]
    fn events_of_a_disabled_agent_are_logged_and_change_nothing() {
        let (_t, p, db) = env();
        add_session(&db, "s6", 100);
        let line = r#"{"ts":5,"event":"Stop","session_id":"s6","agent":"other","last_message_tail":"Go on?"}"#;
        std::fs::create_dir_all(hook::queue_path(&p).parent().unwrap()).unwrap();
        std::fs::write(hook::queue_path(&p), format!("{line}\n")).unwrap();
        assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 1);
        assert!(db
            .session_by_id("s6")
            .unwrap()
            .unwrap()
            .waiting_since
            .is_none());
        let n: i64 = db
            .conn
            .query_row("SELECT count(*) FROM event", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
