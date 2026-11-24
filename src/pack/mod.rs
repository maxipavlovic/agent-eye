//! One portable file with everything eye knows: `eye export` writes it, `eye import` reads it on
//! this machine or another one. A pack is a tar, compressed with zstd, that holds
//!
//! - `manifest.json`: format version, eye version, schema, who and where it came from, counts and
//!   the sha256 of every other file;
//! - `eye.db`: a consistent snapshot of the database (`VACUUM INTO`, as the backups make);
//! - `archive/…`: every file of eye's transcript archive;
//! - `live/<agent>/<project dir>/…`: copies (in the archive's form) of transcripts that are still
//!   with the agent but not archived yet, so the pack holds every session eye knows;
//! - `config.toml`: for reference, without anything that looks like a secret. Never applied.
//!
//! The board's token (`board.token`) is never part of a pack.

mod export;
mod import;
pub mod remap;
mod session;

pub use export::{export, Exported};
pub use import::{apply, plan, Plan, Prepared};
pub use session::{export_session, render, Format};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

/// The layout of a pack. A pack with a higher number was made by a newer eye and is refused.
pub const FORMAT: u32 = 1;
/// File extension of a pack.
pub const EXT: &str = "eyepack";

/// What a pack says about itself, first in the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub eye_version: String,
    /// The schema version of the database inside.
    pub schema: i64,
    /// Unix ms.
    pub created_at: i64,
    pub host: String,
    /// The home folder of the user it was made for, and eye's own folder.
    pub home: String,
    pub eye_home: String,
    pub user: String,
    pub counts: Counts,
    /// The sessions whose transcript came from the agent's folder (`live/…`), not from the archive.
    #[serde(default)]
    pub live: Vec<Live>,
    /// Every file of the pack except this one.
    pub files: BTreeMap<String, Sum>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counts {
    pub sessions: u64,
    pub ideas: u64,
    pub archive_files: u64,
    pub live_files: u64,
    /// Bytes of all files before compression.
    pub bytes: u64,
}

/// One live copy and the session it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Live {
    pub id: String,
    pub agent: String,
    /// Path in the pack (`live/…`).
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sum {
    pub sha256: String,
    pub bytes: u64,
    /// Modification time of the file, Unix seconds.
    pub mtime: i64,
}

/// sha256 of everything `r` yields, as hex, and how many bytes that was.
pub fn hash_reader(mut r: impl Read) -> std::io::Result<(String, u64)> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut n = 0u64;
    loop {
        let got = r.read(&mut buf)?;
        if got == 0 {
            break;
        }
        h.update(&buf[..got]);
        n += got as u64;
    }
    Ok((crate::util::hex(&h.finalize()), n))
}

pub fn mtime_secs(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// The machine's name, for the file name and the manifest.
pub fn host_name() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname writes at most that many bytes.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len() - 1) } == 0;
    let name = if ok {
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    } else {
        String::new()
    };
    let name = name.split('.').next().unwrap_or("").to_string();
    if name.is_empty() {
        "host".into()
    } else {
        name
    }
}

/// A host name as one safe word of a file name.
pub fn file_word(s: &str) -> String {
    let w: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let w = w.trim_matches('-').to_string();
    if w.is_empty() {
        "host".into()
    } else {
        w
    }
}

/// `agent-eye-<host>-<YYYYMMDD-HHMM>.eyepack`
pub fn pack_name(host: &str, at: &jiff::ZonedDateTime) -> String {
    format!(
        "agent-eye-{}-{}.{EXT}",
        file_word(host),
        at.strftime("%Y%m%d-%H%M")
    )
}

/// A folder for work in progress under the eye home, removed when this is dropped.
pub struct Scratch {
    pub dir: PathBuf,
}

impl Scratch {
    pub fn new(eye_home: &Path, what: &str) -> Result<Scratch> {
        let dir = eye_home
            .join("tmp")
            .join(format!("{what}-{}", crate::util::new_uuid()?));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        Ok(Scratch { dir })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One file to put in a pack.
pub struct Item {
    pub name: String,
    pub src: PathBuf,
}

/// Write the pack: the manifest first, then every item, tar inside zstd. The file is 0600 and is
/// removed again when anything fails.
pub fn write(out: &Path, manifest: &Manifest, items: &[Item]) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(out)
        .with_context(|| format!("creating {}", out.display()))?;
    let done = (|| -> Result<()> {
        let enc = zstd::stream::write::Encoder::new(file, 3)?;
        let mut tar = tar::Builder::new(enc);
        let json = serde_json::to_vec_pretty(manifest)?;
        let mut h = tar::Header::new_gnu();
        h.set_size(json.len() as u64);
        h.set_mode(0o600);
        h.set_mtime((manifest.created_at / 1000).max(0) as u64);
        tar.append_data(&mut h, "manifest.json", &json[..])?;
        for it in items {
            let sum = manifest
                .files
                .get(&it.name)
                .with_context(|| format!("{} is not in the manifest", it.name))?;
            let f = std::fs::File::open(&it.src)
                .with_context(|| format!("reading {}", it.src.display()))?;
            let mut h = tar::Header::new_gnu();
            h.set_size(sum.bytes);
            h.set_mode(0o600);
            h.set_mtime(sum.mtime.max(0) as u64);
            // a file that grew since it was hashed must not slip in unchecked
            tar.append_data(&mut h, &it.name, f.take(sum.bytes))?;
        }
        let mut enc = tar.into_inner()?;
        enc.flush()?;
        let file = enc.finish()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(e) = done {
        let _ = std::fs::remove_file(out);
        return Err(e);
    }
    Ok(())
}

/// A pack read into a folder: the manifest, and the sum each file really has.
pub struct Unpacked {
    pub manifest: Manifest,
    pub dir: PathBuf,
}

/// A name inside a pack that stays inside the folder it is unpacked to.
fn safe_name(p: &Path) -> Option<String> {
    let mut parts = vec![];
    for c in p.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.to_string()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// `n` plus 1%.
fn over_one_percent(n: u64) -> u64 {
    n.saturating_add(n / 100)
}

/// Read the pack into `dir` (which must be empty), checking it as it goes: the format, every
/// file's checksum and size against the manifest, and that nothing is missing or extra. Nothing
/// outside `dir` is touched.
pub fn read(pack: &Path, dir: &Path) -> Result<Unpacked> {
    let mut f = std::fs::File::open(pack).with_context(|| format!("opening {}", pack.display()))?;
    let mut magic = [0u8; 4];
    let n = f.read(&mut magic)?;
    if n < 4 || magic != [0x28, 0xb5, 0x2f, 0xfd] {
        bail!(
            "{} is not an eye pack (not zstd data){}",
            pack.display(),
            if std::fs::read(pack)
                .map(|b| b.starts_with(b"age-encryption.org"))
                .unwrap_or(false)
            {
                "; it looks encrypted, and this build cannot decrypt packs"
            } else {
                ""
            }
        );
    }
    drop(f);
    let f = std::fs::File::open(pack)?;
    let mut archive = tar::Archive::new(zstd::stream::read::Decoder::new(f)?);
    let mut manifest: Option<Manifest> = None;
    let mut seen: BTreeMap<String, Sum> = BTreeMap::new();
    let mut unpacked = 0u64;
    for entry in archive.entries().context("reading the pack")? {
        let mut entry = entry.context("reading the pack")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let raw = entry.path()?.into_owned();
        let name = safe_name(&raw).with_context(|| {
            format!(
                "the pack has a path that leaves its folder: {}",
                raw.display()
            )
        })?;
        let mtime = entry.header().mtime().unwrap_or(0) as i64;
        if name == "manifest.json" {
            let mut b = vec![];
            entry.by_ref().take(64 << 20).read_to_end(&mut b)?;
            let m: Manifest = serde_json::from_slice(&b).context("the manifest is damaged")?;
            if m.format > FORMAT {
                bail!(
                    "the pack is format {}, this eye reads up to {FORMAT}; update eye first",
                    m.format
                );
            }
            if m.schema > crate::db::SCHEMA_VERSION {
                bail!(
                    "the pack was made by a newer eye (schema {}, this one knows {}); update eye first",
                    m.schema,
                    crate::db::SCHEMA_VERSION
                );
            }
            // a decompression-bomb guard: the files the manifest lists must add up to the bytes it
            // claims, give or take 1%, before anything is unpacked
            let listed: u64 = m.files.values().map(|s| s.bytes).sum();
            if listed > over_one_percent(m.counts.bytes) {
                bail!(
                    "the manifest lists {listed} bytes of files but says the pack holds {}; refusing it",
                    m.counts.bytes
                );
            }
            manifest = Some(m);
            continue;
        }
        let m = manifest
            .as_ref()
            .context("the pack does not start with its manifest")?;
        let want = m
            .files
            .get(&name)
            .with_context(|| format!("{name} is in the pack but not in its manifest"))?;
        if entry.size() != want.bytes {
            bail!(
                "{name} is {} bytes in the pack but {} in its manifest; refusing it",
                entry.size(),
                want.bytes
            );
        }
        let limit = over_one_percent(m.counts.bytes);
        let dest = dir.join(&name);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dest.parent().context("no parent")?)?;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&dest)
            .with_context(|| format!("{name} is in the pack twice"))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let got = entry.read(&mut buf)?;
            if got == 0 {
                break;
            }
            h.update(&buf[..got]);
            out.write_all(&buf[..got])?;
            total += got as u64;
            unpacked += got as u64;
            if total > want.bytes || unpacked > limit {
                bail!(
                    "{name} unpacks to more than the manifest says ({} bytes in all); refusing it",
                    m.counts.bytes
                );
            }
        }
        let sha = crate::util::hex(&h.finalize());
        if sha != want.sha256 || total != want.bytes {
            bail!("{name} is damaged (checksum does not match); nothing was changed");
        }
        let _ = out.set_modified(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime.max(0) as u64),
        );
        seen.insert(
            name,
            Sum {
                sha256: sha,
                bytes: total,
                mtime,
            },
        );
    }
    let manifest = manifest.context("the pack has no manifest")?;
    if let Some(missing) = manifest.files.keys().find(|k| !seen.contains_key(*k)) {
        bail!("{missing} is listed in the manifest but missing from the pack (cut short?)");
    }
    if !seen.contains_key("eye.db") {
        bail!("the pack has no eye.db");
    }
    Ok(Unpacked {
        manifest,
        dir: dir.to_path_buf(),
    })
}
