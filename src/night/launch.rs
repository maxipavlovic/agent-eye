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

/// What a run is never allowed to run. A rule matches the command text, so these are a first line
/// only (`git -c x=y push` is not matched); what stops a push is the network allowlist in
/// `barrier::settings`, which has no git host in it.
pub const DENY: [&str; 11] = [
    "Bash(git push*)",
    "Bash(git -C * push*)",
    "Bash(git remote*)",
    "Bash(gh*)",
    "Bash(ssh*)",
    "Bash(scp*)",
    "Bash(npm publish*)",
    "Bash(cargo publish*)",
    "Bash(twine upload*)",
    "Bash(gem push*)",
    "Bash(docker push*)",
];

/// The agent an OpenCode run is, defined only by the run's inline config: OpenCode applies an
/// agent's own permissions after the global ones, so a run as the user's default agent would let
/// that agent's permissions in the user's config beat the rules below (a real run pushed that way).
pub const OPENCODE_AGENT: &str = "eye-night";

/// OpenCode's config for a run as [`OPENCODE_AGENT`]: everything is allowed except what `DENY`
/// names (the same commands, as OpenCode's patterns), nothing outside the run's folder, no
/// sub-agents (a sub-agent is another agent, whose permissions the user's config sets: a real run's
/// sub-agent pushed), the MCP servers `mcp` names switched off (their tools are not commands these
/// rules can match) and no update. `--auto` then approves whatever is not denied. The rules match
/// the text of a tool call; they are not a sandbox (a shell command can still write anywhere),
/// which is why the run needs `[night] allow_unsandboxed`.
pub fn opencode_permissions(mcp: &[String]) -> String {
    let mut bash = serde_json::Map::new();
    bash.insert("*".into(), "allow".into());
    for d in DENY {
        let pat = d
            .strip_prefix("Bash(")
            .and_then(|r| r.strip_suffix(')'))
            .unwrap_or(d);
        bash.insert(pat.into(), "deny".into());
    }
    let rules = serde_json::json!({
        "bash": bash,
        "edit": "allow",
        "external_directory": "deny",
        "task": "deny"
    });
    crate::agents::opencode::agent_config(OPENCODE_AGENT, rules, mcp).to_string()
}

/// Package registries a sandboxed run may reach by default: `[night] network` replaces it.
pub const NETWORK: [&str; 7] = [
    "registry.npmjs.org",
    "pypi.org",
    "files.pythonhosted.org",
    "crates.io",
    "index.crates.io",
    "static.crates.io",
    "proxy.golang.org",
];

/// Hosts a run never reaches, even when `[night] network` lists them or a wildcard covers them.
pub const NETWORK_DENIED: [&str; 3] = ["github.com", "gitlab.com", "bitbucket.org"];

/// Build and test commands a run may use when Claude is not in `auto` mode.
pub const ALLOW: [&str; 21] = [
    "Bash(git status*)",
    "Bash(git diff*)",
    "Bash(git log*)",
    "Bash(git add*)",
    "Bash(git commit*)",
    "Bash(npm test*)",
    "Bash(npm run *)",
    "Bash(npm ci*)",
    "Bash(npm install*)",
    "Bash(cargo build*)",
    "Bash(cargo test*)",
    "Bash(cargo check*)",
    "Bash(cargo fmt*)",
    "Bash(cargo clippy*)",
    "Bash(python -m pytest*)",
    "Bash(pytest*)",
    "Bash(go test*)",
    "Bash(go build*)",
    "Bash(make test*)",
    "Bash(node *)",
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

/// What an idea run is told about where it works. Its prompt may name files that a worktree or a
/// copy does not hold (notes nobody committed), and a run that cannot find them should stop and
/// say so, not rebuild them from somewhere else. It is worded the same for every agent except for
/// what that agent's own barrier refuses: the Claude barrier refuses a shell command that names
/// the original folder (see `barrier::hook_command`) and OpenCode's rules refuse its file tools
/// outside the run's folder, so a run is told what to do instead. `run.base_ref` is set when the
/// folder is a repo.
fn idea_preamble(run: &Run, cwd: &Path) -> String {
    let orig = &run.folder;
    // `wt.join("")` of a run at the top of its folder ends in a slash
    let cwd: PathBuf = cwd.components().collect();
    let made = if run.base_ref.is_some() {
        format!("a checkout of the last commit of {orig}. Files that are not committed there (untracked notes, uncommitted edits) are not in it")
    } else {
        format!(
            "a copy of {orig} made when the run started, without {}",
            iso::SKIP.join(", ")
        )
    };
    // what is refused is what the agent's own barrier refuses: see `barrier` and `opencode_permissions`
    let refused = match run.agent.as_str() {
        LEGACY_ID => "; if a shell command that names it is refused, read the file with your file-reading tool".to_string(),
        "opencode" => format!(
            "; your file tools refuse a path outside {}, so read a file there with a shell command",
            cwd.display()
        ),
        _ => String::new(),
    };
    format!(
        "Night run: you work in {cwd}, {made}. You may read files in {orig} but change nothing \
         there{refused}. If something this task needs is missing, stop and say so. Do not push anything.",
        cwd = cwd.display()
    )
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

/// The `--settings` JSON of an agy run: the build and test commands of `ALLOW` as `command(...)`
/// rules, and `DENY` as deny rules (`Bash(git push*)` is `command(git push)`).
pub fn agy_settings() -> String {
    let rule = |r: &&str| {
        let inner = r.strip_prefix("Bash(").and_then(|x| x.strip_suffix(')'))?;
        Some(format!(
            "command({})",
            inner.trim_end_matches('*').trim_end()
        ))
    };
    serde_json::json!({"permissions": {
        "allow": ALLOW.iter().filter_map(rule).collect::<Vec<_>>(),
        "deny": DENY.iter().filter_map(rule).collect::<Vec<_>>(),
    }})
    .to_string()
}

/// The inline config of an OpenCode run in `cwd`: its rules, with the MCP servers OpenCode would
/// start there named and switched off. When OpenCode cannot say which, the run does not start: its
/// rules have no `*` deny and `--auto` would approve those servers' tools unattended.
pub fn opencode_guard(exe: &str, cwd: &Path) -> Result<String> {
    let mcp = crate::agents::opencode::mcp_servers_for_run(exe, cwd).context(
        "OpenCode's config could not be read (`opencode debug config`), so its MCP servers cannot be switched off and the run did not start",
    )?;
    Ok(opencode_permissions(&mcp))
}

/// The command of a run. `new_id` names the run's own session (Claude only); `guard` is what
/// confines the run beyond its arguments: for Claude the `--settings` JSON of its barrier (see
/// `barrier`), for OpenCode its inline config (`opencode_permissions`, with the MCP servers of the
/// run's folder named; without a guard none are).
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
    // An idea run is told where it works as well.
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
        format!("{}\n\n{}", idea_preamble(run, cwd), run.prompt)
    };
    let mut args: Vec<String> = vec![];
    let mut session_id = None;
    let mut program = exe.to_string();
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
    } else if run.agent == "agy" {
        // `-p` answers a permission request with a denial (it cannot ask), so the commands a run may
        // use are allowed up front; agy has no fork, so a run is always a new conversation
        args.extend(["-p".into(), prompt]);
        args.extend(["--mode".into(), "accept-edits".into(), "--sandbox".into()]);
        args.extend(["--settings".into(), agy_settings()]);
        if !cfg.night.agy_model.trim().is_empty() {
            args.extend(["--model".into(), cfg.night.agy_model.trim().into()]);
        }
    } else if run.agent == "opencode" {
        // `env` hands OpenCode a config for this run only; `--standalone` makes the server that
        // applies it a private one, not OpenCode's background service
        program = "env".into();
        let config = guard.map_or_else(|| opencode_permissions(&[]), String::from);
        args.extend([
            format!("OPENCODE_CONFIG_CONTENT={config}"),
            exe.into(),
            "run".into(),
            "--standalone".into(),
            "--auto".into(),
            "--agent".into(),
            OPENCODE_AGENT.into(),
        ]);
        if !cfg.night.opencode_model.trim().is_empty() {
            args.extend(["-m".into(), cfg.night.opencode_model.trim().into()]);
        }
        if cont {
            args.extend([
                "--fork".into(),
                "--session".into(),
                run.session_id.clone().unwrap_or_default(),
            ]);
        }
        args.push(prompt);
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
        program,
        args,
        cwd: cwd.into(),
        session_id,
    }
}

/// Start a spec detached: own session and process group, output to `log`, exit code to `exit`.
/// Its `PATH` starts with `bin`, the folder of eye's own links (see `bin`). Returns the pid of the
/// wrapper, which leads the group.
pub fn spawn(spec: &Spec, bin: &Path, log: &Path, exit: &Path) -> Result<i64> {
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
        .env("PATH", super::bin::run_path(bin))
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

/// The start time of a process, read a few times: the wrapper has only just been forked.
fn process_start(pid: i64) -> Option<i64> {
    let p = u32::try_from(pid).ok()?;
    for _ in 0..5 {
        if let Some(i) = crate::platform::procs::info(p) {
            return Some(i.start);
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    None
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
    // each agent's own gate (a Claude run needs its sandbox, or the user's say-so to go without)
    // before anything is made
    let sandboxed = barrier::gate_for(cfg, &run.agent)?;
    // the eye a run finds first is this one, before anything is made
    let bin = super::bin::dir(paths);
    let exe = super::bin::refresh(&bin)?;
    if super::bin::replaced() {
        crate::daemon::log(
            paths,
            &format!(
                "night: run {} found {} replaced on disk since eye started, so `eye` inside it is a newer one than the daemon; run `eye restart`",
                run.id,
                exe.display()
            ),
        );
    }
    let dir = super::run_dir(paths, run.id);
    let slug = super::slug(&run.prompt);
    let iso = iso::prepare(
        probe,
        &dir,
        Path::new(&run.folder),
        &run.night,
        &slug,
        run.id,
        run.mode(),
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
        let guard = if run.agent == "opencode" {
            Some(opencode_guard(&agent.executable(), &iso.cwd)?)
        } else {
            (run.agent == LEGACY_ID).then(|| {
                // what the run must not touch: the planned folder and where the session began
                let mut protect = vec![PathBuf::from(&run.folder)];
                if let Some(c) = original_cwd(db, run) {
                    protect.push(c);
                }
                let copy = run.mode() == super::Mode::Copy;
                if copy {
                    // a copy locks the whole repo, also when the run began in a sub-folder of it
                    if let Some(top) = iso::repo_root(probe, Path::new(&run.folder)) {
                        protect.push(top);
                    }
                }
                let mut b = barrier::plan(&iso.root, &protect, copy);
                // the sandbox must be able to run the `eye` the run finds on its PATH
                b.allow_reading(&bin);
                b.allow_reading(&exe);
                barrier::settings(&b, &DENY, &cfg.night.network_list(), sandboxed)
            })
        };
        // the repo's base commit is known only now that the run's folder is made
        let made = Run {
            base_ref: iso.base_ref.clone(),
            ..run.clone()
        };
        let spec = build(
            cfg,
            &made,
            &agent.executable(),
            &iso.cwd,
            new_id.as_deref(),
            guard.as_deref(),
        );
        let pid = spawn(&spec, &bin, &dir.join("log"), &dir.join("exit"))?;
        Ok(Started {
            pid,
            pid_start: process_start(pid),
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
