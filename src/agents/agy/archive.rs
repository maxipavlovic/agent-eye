//! The archive copy of a conversation: a folder with a consistent snapshot of the conversation
//! database (`VACUUM INTO`, compressed) and every file of the conversation's `brain/<id>/` folder
//! compressed one by one under `files/`. agy's `scratch/` (clones, build output) and files over
//! 8 MiB are left out. The conversation's row in `conversation_summaries.db` is kept too
//! (`eye-summary.json`). Restoring makes the folder again with the snapshot inside it as
//! `eye-conversation.db`; when the folder goes back to agy's own `brain/`, `reinstall` also puts the
//! snapshot where agy keeps the conversation (`conversations/<id>.db`) and the row back, which is
//! what `agy --conversation <id>` needs to open it after agy deleted the original.

use super::files::{db_of, open_ro, plain_relative, CONVERSATIONS, SNAPSHOT};
use crate::archive::{compress_file, decompress_file};
use crate::util::valid_session_id;
use anyhow::{Context, Result};
use rusqlite::types::{Value as Sql, ValueRef};
use serde_json::{json, Value};
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

const FILE_CAP: u64 = 8 << 20;
const TOTAL_CAP: u64 = 256 << 20;
/// The conversation's row of `conversation_summaries.db`, as JSON, inside an archived copy and a
/// restored folder.
pub const SUMMARY: &str = "eye-summary.json";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() & 1 == 1 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The row of conversation `id` in agy's summaries database, as `{column: value}` (a blob is
/// `{"hex": ...}`). None when there is no such row.
pub fn summary_row(agy_dir: &Path, id: &str) -> Option<Value> {
    let c = open_ro(&agy_dir.join("conversation_summaries.db"))?;
    let mut st = c
        .prepare("SELECT * FROM conversation_summaries WHERE conversation_id=?1")
        .ok()?;
    let names: Vec<String> = st.column_names().map(|n| n.to_string()).collect();
    st.query_row([id], |r| {
        let mut m = serde_json::Map::new();
        for (i, n) in names.iter().enumerate() {
            m.insert(
                n.clone(),
                match r.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(v) => json!(v),
                    ValueRef::Real(v) => json!(v),
                    ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    ValueRef::Blob(b) => json!({ "hex": hex(b) }),
                },
            );
        }
        Ok(Value::Object(m))
    })
    .ok()
}

/// Put a row made by `summary_row` back, unless the conversation has one. Columns the table does
/// not have (another agy version) are left out.
pub fn restore_row(agy_dir: &Path, row: &Value) -> Result<bool> {
    let db = agy_dir.join("conversation_summaries.db");
    anyhow::ensure!(db.is_file(), "agy has no conversation_summaries.db here");
    let conn = rusqlite::Connection::open(&db)?;
    conn.busy_timeout(std::time::Duration::from_millis(2_000))?;
    let have: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('conversation_summaries')")?
        .query_map([], |r| r.get::<_, String>(0))?
        .flatten()
        .collect();
    let obj = row.as_object().context("odd summary row")?;
    let (mut cols, mut vals) = (vec![], vec![]);
    for (k, v) in obj.iter().filter(|(k, _)| have.contains(k)) {
        cols.push(format!("\"{k}\""));
        vals.push(match v {
            Value::Null => Sql::Null,
            Value::Number(n) => n
                .as_i64()
                .map(Sql::Integer)
                .or_else(|| n.as_f64().map(Sql::Real))
                .unwrap_or(Sql::Null),
            Value::String(t) => Sql::Text(t.clone()),
            Value::Object(o) => o
                .get("hex")
                .and_then(Value::as_str)
                .and_then(unhex)
                .map_or(Sql::Null, Sql::Blob),
            _ => Sql::Null,
        });
    }
    let marks = (1..=cols.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let n = conn.execute(
        &format!(
            "INSERT OR IGNORE INTO conversation_summaries ({}) VALUES ({marks})",
            cols.join(",")
        ),
        rusqlite::params_from_iter(vals),
    )?;
    Ok(n > 0)
}

/// After `unarchive` put the folder of conversation `id` (which `folder` may carry another name,
/// while it is being restored) back at `folder` (inside agy's own
/// `brain/`): the database snapshot moves to `conversations/<id>.db` (unless agy still has the
/// conversation) and the summaries row is added. Returns what was done, for people.
pub fn reinstall(agy_dir: &Path, id: &str, folder: &Path) -> Result<Vec<String>> {
    anyhow::ensure!(valid_session_id(id), "odd conversation id {id:?}");
    let mut done = vec![];
    let snap = folder.join(SNAPSHOT);
    let live = agy_dir.join(CONVERSATIONS).join(format!("{id}.db"));
    if snap.is_file() && !live.exists() {
        mkdir(live.parent().context("no parent")?)?;
        std::fs::copy(&snap, &live).context("putting the conversation database back")?;
        done.push("database".to_string());
    }
    if snap.is_file() {
        // agy's own database is the conversation now; the snapshot would shadow it (`db_of`)
        std::fs::remove_file(&snap)?;
    }
    let row = folder.join(SUMMARY);
    if row.is_file() {
        let v: Value = serde_json::from_slice(&std::fs::read(&row)?)?;
        if restore_row(agy_dir, &v).unwrap_or(false) {
            done.push("summaries row".to_string());
        }
        std::fs::remove_file(&row)?;
    }
    Ok(done)
}

fn mkdir(p: &Path) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(p)
        .with_context(|| format!("creating {}", p.display()))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if dir == root && e.file_name() == "scratch" {
                continue;
            }
            walk(root, &p, out);
        } else if ft.is_file() {
            out.push(p);
        }
    }
}

pub fn archive(folder: &Path, copy: &Path) -> Result<()> {
    mkdir(copy)?;
    if let Some(db) = db_of(folder) {
        if let Some(conn) = open_ro(&db) {
            let snap = copy.join(SNAPSHOT);
            conn.execute("VACUUM INTO ?1", [snap.to_string_lossy()])
                .context("snapshotting the conversation database")?;
            compress_file(&snap, &copy.join(format!("{SNAPSHOT}.zst")))?;
            std::fs::remove_file(&snap)?;
        }
    }
    // the row sits next to `brain/` and `conversations/`: folder = <agy>/brain/<id>
    if let (Some(id), Some(agy)) = (
        folder.file_name().and_then(|n| n.to_str()),
        folder.parent().and_then(Path::parent),
    ) {
        if let Some(row) = summary_row(agy, id) {
            let tmp = copy.join(SUMMARY);
            std::fs::write(&tmp, serde_json::to_vec(&row)?)?;
            compress_file(&tmp, &copy.join(format!("{SUMMARY}.zst")))?;
            std::fs::remove_file(&tmp)?;
        }
    }
    let mut files = vec![];
    walk(folder, folder, &mut files);
    let mut total = 0u64;
    for f in files {
        let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
        let rel = f.strip_prefix(folder)?;
        if size > FILE_CAP
            || total + size > TOTAL_CAP
            || rel == Path::new(SNAPSHOT)
            || rel == Path::new(SUMMARY)
        {
            continue;
        }
        total += size;
        let mut dest = copy.join("files").join(rel).into_os_string();
        dest.push(".zst");
        let dest = std::path::PathBuf::from(dest);
        mkdir(dest.parent().context("no parent")?)?;
        compress_file(&f, &dest)?;
    }
    Ok(())
}

pub fn unarchive(copy: &Path, dest: &Path) -> Result<()> {
    mkdir(dest)?;
    let snap = copy.join(format!("{SNAPSHOT}.zst"));
    if snap.is_file() {
        decompress_file(&snap, &dest.join(SNAPSHOT))?;
    }
    let row = copy.join(format!("{SUMMARY}.zst"));
    if row.is_file() {
        decompress_file(&row, &dest.join(SUMMARY))?;
    }
    let root = copy.join("files");
    let mut files = vec![];
    walk(&root, &root, &mut files);
    for f in files {
        let rel = f.strip_prefix(&root)?;
        let Some(name) = rel.to_str().and_then(|s| s.strip_suffix(".zst")) else {
            continue;
        };
        let rel = Path::new(name);
        anyhow::ensure!(plain_relative(rel), "odd path in an archived copy: {name}");
        let out = dest.join(rel);
        mkdir(out.parent().context("no parent")?)?;
        decompress_file(&f, &out)?;
    }
    Ok(())
}
