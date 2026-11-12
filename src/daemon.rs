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
use crate::worked;
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
/// The longest an upgrade restart waits for a snapshot or a summary round to finish.
const RESTART_WAIT: Duration = Duration::from_secs(60);
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

/// Tell about the reminders that came due, once each. While `[remind] notify` is off they are still
/// marked as told, with no notice, so switching it on covers only what falls due afterwards. The
/// file is read again each time, so a switch made while the daemon runs counts at once.
fn notify_reminders(db: &Db, paths: &Paths, cfg: &Config, send: &mut dyn FnMut(&str, &str)) {
    let on = crate::remind::notices::is_on(paths, cfg.remind.notify);
    let mut silent = |_: &str, _: &str| {};
    let send: &mut dyn FnMut(&str, &str) = if on { send } else { &mut silent };
    match crate::remind::notify_due(db, now_ms(), send) {
        Ok(0) => {}
        Ok(n) => log(paths, &format!("tick: {n} reminders due")),
        Err(e) => log(paths, &format!("reminders: {e:#}")),
    }
}

/// One pass of the main loop: events, then the incremental scan, a few sessions of the work timeline
/// backfill, then due reminders.
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
    match worked::backfill::run_batch(db, paths, &agents::enabled(cfg), worked::backfill::BATCH) {
        Ok(p) if p.counted + p.unreadable > 0 => log(
            paths,
            &format!(
                "work timeline: {} sessions counted, {} unreadable, {} to go",
                p.counted, p.unreadable, p.left
            ),
        ),
        Ok(_) => {}
        Err(e) => log(paths, &format!("work timeline: {e:#}")),
    }
    notify_reminders(db, paths, cfg, &mut |title, body| {
        if !crate::platform::notify::send(title, body) {
            log(paths, "reminder notice not shown");
        }
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
/// When its binary is upgraded on disk, it stops the same way and starts the new one in its place.
pub fn run(paths: Paths, cfg: Config, port: u16, board: bool) -> Result<()> {
    paths.ensure_home()?;
    let lock = Lock::acquire(&paths)?;
    log(&paths, &format!("daemon start pid {}", std::process::id()));
    let stop = Arc::new(AtomicBool::new(false));
    let worker_busy = Arc::new(AtomicBool::new(false));
    let mut watch = upgrade::Watch::of_this_process();
    let bin = watch.as_ref().map(|w| w.path().to_path_buf());
    let upgraded = Arc::new(AtomicBool::new(false));
    let handle = {
        let (paths, cfg, stop, busy) = (paths.clone(), cfg.clone(), stop.clone(), worker_busy);
        let upgraded = upgraded.clone();
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
                // A snapshot of a big database or to a slow mirror takes a while: it runs in its own
                // thread, one slow job at a time, so hooks and scans keep going meanwhile.
                let backup_due = last_backup.is_none_or(|t| t.elapsed() >= BACKUP_EVERY);
                if backup_due && !busy.swap(true, Ordering::SeqCst) {
                    last_backup = Some(Instant::now());
                    let (paths, cfg, busy) = (paths.clone(), cfg.clone(), busy.clone());
                    std::thread::spawn(move || {
                        let r = Db::open(&paths.db_path())
                            .and_then(|d| backup::daily_if_due(&d.conn, &paths, &cfg, now_ms()));
                        match r {
                            Ok(Some(s)) => log(&paths, &format!("snapshot {}", s.path.display())),
                            Ok(None) => {}
                            Err(e) => log(&paths, &format!("snapshot failed: {e:#}")),
                        }
                        busy.store(false, Ordering::SeqCst);
                    });
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
                if watch.as_mut().is_some_and(|w| w.upgraded()) {
                    log(&paths, "the binary was upgraded; restarting into it");
                    let since = Instant::now();
                    while busy.load(Ordering::SeqCst) && since.elapsed() < RESTART_WAIT {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    upgraded.store(true, Ordering::SeqCst);
                    server::ask_shutdown();
                    break;
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
    if let Some(bin) = bin.filter(|_| upgraded.load(Ordering::SeqCst)) {
        drop(lock);
        return Err(upgrade::restart(&bin));
    }
    result
}

mod upgrade;

#[cfg(test)]
mod tests;
