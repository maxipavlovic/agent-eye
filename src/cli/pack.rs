//! `eye export` (one portable file, or one session as a page) and `eye import`.

use super::Global;
use crate::install::Opts;
use crate::pack::{self, Format};
use crate::util::show_path;
use anyhow::{bail, Context, Result};
use std::io::{IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// `-` as a file name: standard output for export, standard input for import, so a pack can go
/// through another program (`eye export --to - | age -p > backup.eyepack.age`).
fn is_stdio(p: &Path) -> bool {
    p.as_os_str() == "-"
}

/// Copy a finished pack to standard output, which must not be a terminal.
fn to_stdout(file: &Path) -> Result<()> {
    let mut f = std::fs::File::open(file)?;
    let out = std::io::stdout();
    let mut lock = out.lock();
    std::io::copy(&mut f, &mut lock).context("writing the pack to standard output")?;
    lock.flush()?;
    Ok(())
}

/// The answer to "go on?": from the terminal itself when standard input carries the pack.
fn ask(from_tty: bool) -> Result<bool> {
    let hint = "import without asking";
    super::ask::confirm(
        "go on?",
        if from_tty {
            super::ask::From::DevTty(hint)
        } else {
            super::ask::From::Stdin
        },
    )
}

/// `to` as the file to write: a folder gets `name` inside it; nothing means the current folder.
fn target(to: Option<&Path>, name: &str) -> Result<PathBuf> {
    let p = match to {
        Some(t) if t.is_dir() => t.join(name),
        Some(t) => t.to_path_buf(),
        None => std::env::current_dir()?.join(name),
    };
    anyhow::ensure!(!p.exists(), "{} already exists", p.display());
    Ok(p)
}

pub fn export(
    g: &Global,
    session: Option<&str>,
    to: Option<&Path>,
    html: bool,
    md: bool,
) -> Result<()> {
    let env = g.open()?;
    let Some(id) = session else {
        if html || md {
            bail!("--html and --md write one session: give its id (`eye export <session> --html`)");
        }
        let name = pack::pack_name(&pack::host_name(), &jiff::ZonedDateTime::now());
        if to.is_some_and(is_stdio) {
            if std::io::stdout().is_terminal() {
                bail!("a pack is binary: send it to a program or a file, e.g. `eye export --to - | age -p > {name}.age`");
            }
            let work = pack::Scratch::new(&env.paths.home, "export-stdout")?;
            let out = work.dir.join(&name);
            let done = pack::export(&env.db, &env.paths, &env.agents, &out)?;
            to_stdout(&out)?;
            let c = &done.counts;
            eprintln!(
                "{:.1} MB to standard output: {} sessions, {} ideas, {} archive files",
                done.bytes as f64 / 1e6,
                c.sessions,
                c.ideas,
                c.archive_files
            );
            return Ok(());
        }
        let out = target(to, &name)?;
        let done = pack::export(&env.db, &env.paths, &env.agents, &out)?;
        let c = &done.counts;
        println!("{}", show_path(&done.path));
        println!(
            "{:.1} MB: {} sessions, {} ideas, {} archive files, {} transcripts copied from the agents' folders",
            done.bytes as f64 / 1e6,
            c.sessions,
            c.ideas,
            c.archive_files,
            c.live_files
        );
        println!("it holds your transcripts: keep it private (see docs/security.md)");
        return Ok(());
    };
    let format = if md { Format::Markdown } else { Format::Html };
    let quiet = crate::quiet::Quiet::of(&env.cfg);
    let (name, text) = pack::export_session(&env.db, &env.paths, &env.agents, id, quiet, format)?;
    if to.is_some_and(is_stdio) {
        let mut out = std::io::stdout().lock();
        out.write_all(text.as_bytes())?;
        out.flush()?;
        return Ok(());
    }
    let out = target(to, &name)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&out)
        .with_context(|| format!("creating {}", out.display()))?;
    f.write_all(text.as_bytes())?;
    println!("{} ({:.1} KB)", show_path(&out), text.len() as f64 / 1e3);
    Ok(())
}

pub fn import(g: &Global, file: &Path, yes: bool, dry_run: bool) -> Result<()> {
    let paths = g.paths()?;
    let cfg = g.config_read_only(&paths);
    let piped = is_stdio(file);
    // a pack on standard input is saved first: checking and unpacking read it more than once
    let (file, _work) = if piped {
        if std::io::stdin().is_terminal() {
            bail!(
                "nothing to read: send a pack in, e.g. `age -d backup.eyepack.age | eye import -`"
            );
        }
        paths.ensure_home()?;
        let work = pack::Scratch::new(&paths.home, "import-stdin")?;
        let saved = work.dir.join("stdin.eyepack");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&saved)?;
        std::io::copy(&mut std::io::stdin().lock(), &mut f)
            .context("reading the pack from standard input")?;
        (saved, Some(work))
    } else {
        (file.to_path_buf(), None)
    };
    let plan = pack::plan(&paths, &file)?;
    println!(
        "import {}",
        if piped {
            "standard input".into()
        } else {
            show_path(&file)
        }
    );
    for l in plan.lines(&paths) {
        println!("{l}");
    }
    if dry_run {
        println!("dry run: nothing was changed");
        return Ok(());
    }
    if !yes {
        if !piped && !std::io::stdin().is_terminal() {
            bail!("not a terminal: add --yes to import without asking");
        }
        if !ask(piped)? {
            println!("nothing was changed");
            return Ok(());
        }
    }
    let o = Opts::resolve(&paths, None, None)?;
    let done = pack::apply(&o, &cfg, plan)?;
    match &done.applied.pre_restore {
        Some(p) => println!("kept the old database as {}", show_path(p)),
        None => println!("there was no sound database to keep"),
    }
    if done.applied.daemon_restarted {
        println!("daemon started again");
    }
    if let Some(why) = &done.applied.restart_failed {
        println!("the daemon is not running: {why}");
    }
    println!(
        "imported: {} new and {} updated sessions, {} ideas, {} transcript copies",
        done.sessions_added, done.sessions_updated, done.ideas_added, done.files_copied
    );
    Ok(())
}
