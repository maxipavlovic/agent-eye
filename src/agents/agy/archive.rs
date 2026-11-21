//! The archive copy of a conversation: a folder with a consistent snapshot of the conversation
//! database (`VACUUM INTO`, compressed) and every file of the conversation's `brain/<id>/` folder
//! compressed one by one under `files/`. agy's `scratch/` (clones, build output) and files over
//! 8 MiB are left out. Restoring makes the folder again, with the snapshot inside it as
//! `eye-conversation.db`; agy's own database is not put back, so a restored copy can be read but
//! agy cannot resume it.

use super::files::{db_of, open_ro, plain_relative, SNAPSHOT};
use crate::archive::{compress_file, decompress_file};
use anyhow::{Context, Result};
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

const FILE_CAP: u64 = 8 << 20;
const TOTAL_CAP: u64 = 256 << 20;

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
    let mut files = vec![];
    walk(folder, folder, &mut files);
    let mut total = 0u64;
    for f in files {
        let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
        let rel = f.strip_prefix(folder)?;
        if size > FILE_CAP || total + size > TOTAL_CAP || rel == Path::new(SNAPSHOT) {
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
