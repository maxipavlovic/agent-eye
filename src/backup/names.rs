//! What a snapshot's file name says, and which snapshots the keep rules leave.

use crate::config::BackupCfg;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const PREFIX: &str = "eye-";
const SUFFIX: &str = ".db.zst";
/// Snapshot times are UTC, marked with the `Z`, so a clock change never reorders them.
const STAMP: &str = "%Y%m%d-%H%M%SZ";
/// The length of a formatted `STAMP`.
const STAMP_LEN: usize = 16;
/// A temp file of a snapshot or a restore older than this was left by a crash.
const TEMP_STALE: Duration = Duration::from_secs(3600);

/// Why a snapshot was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Daily,
    Manual,
    /// Before the schema was upgraded to this version.
    PreUpgrade(i64),
    PreRestore,
}

impl Reason {
    fn word(&self) -> String {
        match self {
            Reason::Daily => "daily".into(),
            Reason::Manual => "manual".into(),
            Reason::PreUpgrade(v) => format!("pre-v{v}"),
            Reason::PreRestore => "pre-restore".into(),
        }
    }

    fn parse(s: &str) -> Option<Reason> {
        match s {
            "daily" => Some(Reason::Daily),
            "manual" => Some(Reason::Manual),
            "pre-restore" => Some(Reason::PreRestore),
            _ => s
                .strip_prefix("pre-v")
                .and_then(|v| v.parse().ok())
                .map(Reason::PreUpgrade),
        }
    }

    /// The reason as people read it in a listing.
    pub fn label(&self) -> String {
        self.word()
    }
}

/// The file name of a snapshot made at `at` (UTC) of a database at schema `schema`, which is
/// left out of the name when it is not known.
pub fn file_name(at: NaiveDateTime, reason: &Reason, schema: Option<i64>) -> String {
    let schema = schema.map_or(String::new(), |v| format!("-s{v}"));
    format!(
        "{PREFIX}{}-{}{schema}{SUFFIX}",
        at.format(STAMP),
        reason.word()
    )
}

/// A snapshot in a folder, read from its file name.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// When it was made, in UTC.
    pub stamp: NaiveDateTime,
    pub reason: Reason,
    /// The schema version of the database it holds, from the name; None when the name has none.
    pub schema: Option<i64>,
    /// Bytes on disk.
    pub bytes: u64,
}

impl Entry {
    /// None when the file is not named as eye names its snapshots.
    pub fn from_path(path: &Path) -> Option<Entry> {
        let name = path.file_name()?.to_str()?.to_string();
        let body = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
        let (stamp, reason) = body.split_at_checked(STAMP_LEN)?;
        let stamp = NaiveDateTime::parse_from_str(stamp, STAMP).ok()?;
        let reason = reason.strip_prefix('-')?;
        let (reason, schema) = match reason.rsplit_once('-') {
            Some((r, s)) if s.starts_with('s') && s[1..].parse::<i64>().is_ok() => {
                (r, s[1..].parse().ok())
            }
            _ => (reason, None),
        };
        let reason = Reason::parse(reason)?;
        let md = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
        Some(Entry {
            name,
            path: path.to_path_buf(),
            stamp,
            reason,
            schema,
            bytes: md.len(),
        })
    }

    /// When it was made, in Unix ms.
    pub fn stamp_ms(&self) -> i64 {
        Utc.from_utc_datetime(&self.stamp).timestamp_millis()
    }

    /// When it was made, on the wall clock of this machine.
    pub fn local_stamp(&self) -> DateTime<Local> {
        Utc.from_utc_datetime(&self.stamp).with_timezone(&Local)
    }
}

/// The names the keep rules leave, from the snapshots newest first: the newest `keep_daily`
/// dailies; the newest daily of each of the newest `keep_weekly` ISO weeks that have one; the last
/// 3 `pre-v*`, the last 3 `pre-restore` and the last 5 `manual`.
pub fn keep(all: &[Entry], cfg: &BackupCfg) -> HashSet<String> {
    let of = |want: fn(&Reason) -> bool| all.iter().filter(move |e| want(&e.reason));
    let mut keep: HashSet<String> = HashSet::new();
    let dailies: Vec<&Entry> = of(|r| matches!(r, Reason::Daily)).collect();
    keep.extend(dailies.iter().take(cfg.keep_daily).map(|e| e.name.clone()));
    let mut weeks: Vec<(i32, u32)> = vec![];
    for e in &dailies {
        let w = e.stamp.date().iso_week();
        let week = (w.year(), w.week());
        if !weeks.contains(&week) && weeks.len() < cfg.keep_weekly {
            weeks.push(week);
            keep.insert(e.name.clone());
        }
    }
    keep.extend(
        of(|r| matches!(r, Reason::PreUpgrade(_)))
            .take(3)
            .map(|e| e.name.clone()),
    );
    keep.extend(
        of(|r| matches!(r, Reason::PreRestore))
            .take(3)
            .map(|e| e.name.clone()),
    );
    keep.extend(
        of(|r| matches!(r, Reason::Manual))
            .take(5)
            .map(|e| e.name.clone()),
    );
    keep
}

/// Remove the temp files (`.eye-*.tmp`) a crashed snapshot or restore left in `dir`.
pub fn remove_stale_temps(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t + TEMP_STALE < SystemTime::now());
        if name.starts_with(".eye-") && name.ends_with(".tmp") && stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
