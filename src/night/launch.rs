//! The command line of a run for each agent, and starting it detached from eye.
//!
//! Every run is `sh -c '"$@"; echo $? > exit' sh <agent> <args>` in its own session (`setsid`), with
//! stdout and stderr appended to `log`. The daemon finds it again by pid and start time, and reads
//! `exit` when the process is gone, so a restart or an upgrade of the daemon does not lose a run.

use super::{barrier, iso, Run};
use crate::agents::{Agents, LEGACY_ID};
use crate::config::Config;
use crate::db::Db;
use crate::loose::Probe;
use crate::paths::Paths;
use crate::util::new_uuid;
use anyhow::{Context, Result};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a run is never allowed to run: nothing leaves the machine.
pub const DENY: [&str; 6] = [
    "Bash(git push*)",
    "Bash(git -C * push*)",
    "Bash(git remote*)",
    "Bash(gh pr create*)",
    "Bash(gh pr merge*)",
    "Bash(gh release*)",
];

/// Build and test commands a run may use when Claude is not in `auto` mode.
pub const ALLOW: [&str; 14] = [
    "Bash(git status*)",
    "Bash(git diff*)",
    "Bash(git log*)",
    "Bash(git add*)",
    "Bash(git commit*)",
    "Bash(cargo *)",
    "Bash(npm *)",
    "Bash(npx *)",
    "Bash(node *)",
    "Bash(python*)",
    "Bash(pytest*)",
    "Bash(go *)",
    "Bash(make*)",
    "Bash(ls*)",
];

/// What to run and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The session id the run will have, when the agent lets eye choose it.
    pub session_id: Option<String>,
}

/// A prompt that starts with `-` would be read as a flag.
fn safe_prompt(p: &str) -> String {
    if p.starts_with('-') {
        format!("Task: {p}")
    } else {
        p.to_string()
    }
}

/// The name Claude Code gives the folder of a project's transcripts: every character that is not a
/// letter or digit becomes `-`.
pub fn claude_project_dir(cwd: &Path) -> String {
    let real = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    real.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The command of a run. `new_id` names the run's own session (Claude only); `guard` is the
/// `--settings` JSON of its barrier (see `barrier`).
pub fn build(
    cfg: &Config,
    run: &Run,
    exe: &str,
    cwd: &Path,
    new_id: Option<&str>,
    guard: Option<&str>,
) -> Spec {
    let cont = run.kind == "session";
    // A continued session remembers the paths of the original folder: say where the work goes.
    let prompt = if cont {
        format!(
            "Night run: you are continuing this session in an isolated copy of the project at {cwd}. \
             The earlier conversation mentions paths in the original folder ({orig}); use the same files \
             under {cwd} instead and change nothing in {orig}. Do not push anything.\n\n{p}",
            cwd = cwd.display(),
            orig = run.folder,
            p = run.prompt
        )
    } else {
        safe_prompt(&run.prompt)
    };
    let mut args: Vec<String> = vec![];
    let mut session_id = None;
    if run.agent == LEGACY_ID {
        args.extend(["-p".into(), prompt]);
        if cont {
            args.extend([
                "--resume".into(),
                run.session_id.clone().unwrap_or_default(),
                "--fork-session".into(),
            ]);
        }
        if let Some(id) = new_id {
            args.extend(["--session-id".into(), id.into()]);
            session_id = Some(id.to_string());
        }
        let mode = cfg.night.claude_permission.trim();
        args.extend(["--permission-mode".into(), mode.into()]);
        if mode != "auto" && mode != "bypassPermissions" {
            args.extend(["--allowedTools".into(), ALLOW.join(",")]);
        }
        args.extend(["--disallowedTools".into(), DENY.join(",")]);
        if let Some(g) = guard {
            args.extend(["--settings".into(), g.into()]);
        }
        if !cfg.night.claude_model.trim().is_empty() {
            args.extend(["--model".into(), cfg.night.claude_model.trim().into()]);
        }
    } else {
        args.push("exec".into());
        if cont {
            // `exec fork` has no --sandbox or --cd: the folder is the working directory
            args.extend([
                "fork".into(),
                "-c".into(),
                "sandbox_mode=\"workspace-write\"".into(),
            ]);
        } else {
            args.extend([
                "--sandbox".into(),
                "workspace-write".into(),
                "--cd".into(),
                cwd.to_string_lossy().into(),
            ]);
        }
        args.push("--skip-git-repo-check".into());
        if !cfg.night.codex_model.trim().is_empty() {
            args.extend(["-m".into(), cfg.night.codex_model.trim().into()]);
        }
        if cont {
            args.push(run.session_id.clone().unwrap_or_default());
        }
        args.push(prompt);
    }
    Spec {
        program: exe.into(),
        args,
        cwd: cwd.into(),
        session_id,
    }
}

/// Start a spec detached: own session and process group, output to `log`, exit code to `exit`.
/// Returns the pid of the wrapper, which leads the group.
pub fn spawn(spec: &Spec, log: &Path, exit: &Path) -> Result<i64> {
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .with_context(|| format!("opening {}", log.display()))?;
    crate::files::make_private(log);
    let err = out.try_clone()?;
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(r#""$@"; echo $? > "$EYE_NIGHT_EXIT""#)
        .arg("sh")
        .arg(&spec.program)
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env("EYE_NIGHT_EXIT", exit)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    // SAFETY: setsid only changes this child's session, between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd
        .spawn()
        .with_context(|| format!("starting {}", spec.program))?;
    let pid = i64::from(child.id());
    // the child is reaped by the OS when eye exits; a daemon that keeps running reaps it here
    std::thread::spawn(move || {
        let mut c = child;
        let _ = c.wait();
    });
    Ok(pid)
}

/// What starting a run left: where it works and who it is.
#[derive(Debug, Clone)]
pub struct Started {
    pub pid: i64,
    pub pid_start: Option<i64>,
    pub iso: iso::Isolation,
    pub session_id: Option<String>,
}

/// Put the transcript of the session a run continues where Claude looks for it from the worktree.
pub(super) fn stage_transcript(
    db: &Db,
    paths: &Paths,
    agents: &Agents,
    run: &Run,
    cwd: &Path,
) -> Result<()> {
    let sid = run
        .session_id
        .as_deref()
        .context("no session to continue")?;
    let s = db.session_by_id(sid)?.context("the session is gone")?;
    let agent = agents
        .of_session(&s)
        .context("the agent of the session is not enabled")?;
    if agent.id() != LEGACY_ID {
        return Ok(());
    }
    crate::archive::restore_if_missing(db, paths, agents, &s)?;
    let src = PathBuf::from(s.transcript_path.as_deref().context("no transcript path")?);
    let projects = src
        .parent()
        .and_then(Path::parent)
        .context("odd transcript path")?;
    let dir = projects.join(claude_project_dir(cwd));
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("{sid}.jsonl"));
    if !dest.exists() {
        std::fs::copy(&src, &dest).with_context(|| format!("copying {}", src.display()))?;
    }
    Ok(())
}

/// The folder the continued session was working in when it began.
fn original_cwd(db: &Db, run: &Run) -> Option<PathBuf> {
    let s = db.session_by_id(run.session_id.as_deref()?).ok()??;
    s.cwd.or(s.project_dir).map(PathBuf::from)
}

/// Make the run's folder, build its command and start it. On failure nothing keeps running.
pub fn start(
    probe: &dyn Probe,
    db: &Db,
    paths: &Paths,
    cfg: &Config,
    agents: &Agents,
    run: &Run,
) -> Result<Started> {
    let agent = agents
        .get(&run.agent)
        .with_context(|| format!("the agent '{}' is not enabled", run.agent))?;
    // a Claude run needs its sandbox, or the user's say-so to go without, before anything is made
    let sandboxed = if run.agent == LEGACY_ID {
        barrier::gate(cfg)?
    } else {
        true
    };
    let dir = super::run_dir(paths, run.id);
    let slug = super::slug(&run.prompt);
    let iso = iso::prepare(
        probe,
        &dir,
        Path::new(&run.folder),
        &run.night,
        &slug,
        run.id,
    )?;
    let new_id = if run.agent == LEGACY_ID {
        Some(new_uuid().context("choosing a session id")?)
    } else {
        None
    };
    let go = || -> Result<Started> {
        if run.kind == "session" {
            stage_transcript(db, paths, agents, run, &iso.cwd)?;
        }
        let guard = (run.agent == LEGACY_ID).then(|| {
            // what the run must not touch: the planned folder and where the session began
            let mut protect = vec![PathBuf::from(&run.folder)];
            if let Some(c) = original_cwd(db, run) {
                protect.push(c);
            }
            let b = barrier::plan(&iso.root, &protect);
            barrier::settings(&b, &DENY, sandboxed)
        });
        let spec = build(
            cfg,
            run,
            &agent.executable(),
            &iso.cwd,
            new_id.as_deref(),
            guard.as_deref(),
        );
        let pid = spawn(&spec, &dir.join("log"), &dir.join("exit"))?;
        Ok(Started {
            pid,
            pid_start: crate::platform::procs::info(u32::try_from(pid).unwrap_or(0))
                .map(|i| i.start),
            session_id: spec.session_id,
            iso: iso.clone(),
        })
    };
    go().inspect_err(|_| {
        // nothing ran: take the folder back
        let _ = iso::discard(
            probe,
            &paths.home.join("night"),
            &Run {
                worktree: Some(iso.root.to_string_lossy().into()),
                branch: iso.branch.clone(),
                folder: run.folder.clone(),
                status: "failed".into(),
                ..run.clone()
            },
            true,
        );
    })
}
