//! `eye export`: collect everything into one pack.

use super::{host_name, mtime_secs, Counts, Item, Live, Manifest, Scratch, Sum, FORMAT};
use crate::agents::Agents;
use crate::archive;
use crate::db::{Db, SCHEMA_VERSION};
use crate::paths::Paths;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What an export wrote.
#[derive(Debug)]
pub struct Exported {
    pub path: PathBuf,
    /// Bytes of the pack on disk.
    pub bytes: u64,
    pub counts: Counts,
}

/// The user's home folder and name, as the manifest records them.
fn who() -> (String, String) {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| {
            Path::new(&home)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    (home, user)
}

/// Every file under `dir` (not links, not half-written copies) as (name in the pack, path).
fn walk(dir: &Path, prefix: &str, out: &mut Vec<Item>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(ft) = e.file_type() else { continue };
        let pack_name = format!("{prefix}/{name}");
        if ft.is_dir() {
            walk(&e.path(), &pack_name, out);
        } else if ft.is_file() && !name.ends_with(".tmp") && !name.ends_with(".restoring") {
            out.push(Item {
                name: pack_name,
                src: e.path(),
            });
        }
    }
}

/// `config.toml` without any line that sets something secret-looking. There is none today (the
/// board's token lives in its own file); this keeps it so when a key is added later.
fn config_for_pack(text: &str) -> String {
    const SECRET: [&str; 5] = ["token", "secret", "password", "passphrase", "api_key"];
    text.lines()
        .filter(|l| {
            let key = l.split('=').next().unwrap_or("").to_ascii_lowercase();
            !(l.contains('=') && SECRET.iter().any(|s| key.contains(s)))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// Transcripts still with their agent that the archive does not hold in their current state: a
/// copy of each, in the archive's form, goes to `live/<agent>/<project dir>/<archive name>`.
fn live_copies(
    db: &Db,
    paths: &Paths,
    agents: &Agents,
    work: &Path,
) -> Result<(Vec<Item>, Vec<Live>)> {
    let (mut items, mut live) = (vec![], vec![]);
    for (n, s) in db.all_sessions()?.into_iter().enumerate() {
        if s.transcript_state.as_deref() != Some("present") {
            continue;
        }
        let (Some(agent), Some(src), Some(pd)) = (
            agents.of_session(&s),
            s.transcript_path.as_deref().map(Path::new),
            s.project_dir.as_deref().filter(|p| !p.is_empty()),
        ) else {
            continue;
        };
        let Some(stamp) = agent.transcript_stamp(src) else {
            continue;
        };
        let dest = archive::archive_path(paths, agent.as_ref(), pd, &s.id);
        let sig = format!("{}:{}", stamp.size, stamp.mtime);
        let current = dest.exists()
            && db.meta_get(&format!("archived_sig:{}", s.id))?.as_deref() == Some(sig.as_str());
        if current {
            continue;
        }
        let tmp = work.join(format!("c{n}"));
        if let Err(e) = agent.archive(src, &tmp) {
            eprintln!("warn: {}: not copied into the pack: {e:#}", s.id);
            continue;
        }
        let base = format!("live/{}/{pd}/{}", agent.id(), agent.archive_name(&s.id));
        let before = items.len();
        if tmp.is_dir() {
            walk(&tmp, &base, &mut items);
        } else {
            items.push(Item {
                name: base.clone(),
                src: tmp,
            });
        }
        if items.len() > before {
            live.push(Live {
                id: s.id.clone(),
                agent: agent.id().to_string(),
                file: base,
            });
        }
    }
    Ok((items, live))
}

/// Make the pack at `out` (a file that must not exist yet) from this eye home.
pub fn export(db: &Db, paths: &Paths, agents: &Agents, out: &Path) -> Result<Exported> {
    let work = Scratch::new(&paths.home, "export")?;
    let mut items: Vec<Item> = vec![];

    let snap = work.dir.join("eye.db");
    db.conn
        .execute("VACUUM INTO ?1", [snap.to_string_lossy()])
        .context("copying the database")?;
    items.push(Item {
        name: "eye.db".into(),
        src: snap,
    });

    let mut archived = vec![];
    walk(&paths.home.join("archive"), "archive", &mut archived);
    let archive_files = archived.len() as u64;
    items.extend(archived);

    let build = work.dir.join("live");
    std::fs::create_dir_all(&build)?;
    let (live_items, live) = live_copies(db, paths, agents, &build)?;
    let live_files = live_items.len() as u64;
    items.extend(live_items);

    if let Ok(text) = std::fs::read_to_string(paths.config_path()) {
        let p = work.dir.join("config.toml");
        std::fs::write(&p, config_for_pack(&text))?;
        items.push(Item {
            name: "config.toml".into(),
            src: p,
        });
    }

    let mut files = BTreeMap::new();
    let mut bytes = 0u64;
    for it in &items {
        let f = std::fs::File::open(&it.src)
            .with_context(|| format!("reading {}", it.src.display()))?;
        let md = f.metadata()?;
        let (sha256, n) = super::hash_reader(f)?;
        bytes += n;
        files.insert(
            it.name.clone(),
            Sum {
                sha256,
                bytes: n,
                mtime: mtime_secs(&md),
            },
        );
    }
    let count = |sql: &str| -> Result<u64> {
        Ok(db.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
    };
    let (home, user) = who();
    let counts = Counts {
        sessions: count("SELECT COUNT(*) FROM session")?,
        ideas: count("SELECT COUNT(*) FROM idea")?,
        archive_files,
        live_files,
        bytes,
    };
    let manifest = Manifest {
        format: FORMAT,
        eye_version: env!("CARGO_PKG_VERSION").into(),
        schema: SCHEMA_VERSION,
        created_at: crate::util::now_ms(),
        host: host_name(),
        home,
        eye_home: paths.home.to_string_lossy().into_owned(),
        user,
        counts: counts.clone(),
        live,
        files,
    };
    super::write(out, &manifest, &items)?;
    let bytes = std::fs::metadata(out)?.len();
    Ok(Exported {
        path: out.to_path_buf(),
        bytes,
        counts,
    })
}
