//! The seam in front of the machine: processes, containers and git. Tests feed their own data.

use crate::platform::procs::{self, Info, Proc};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long one git call may take before it is given up.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(8);
/// How long `docker ps` may take.
pub const DOCKER_TIMEOUT: Duration = Duration::from_secs(2);

/// A running container as `docker ps` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Docker {
    /// The full container id.
    pub id: String,
    pub name: String,
    pub image: String,
    /// When it was created, in Unix seconds.
    pub created: i64,
    /// Compose's working folder label, when there is one.
    pub workdir: Option<String>,
    /// Host folders bind-mounted into it.
    pub mounts: Vec<String>,
}

/// Everything a scan or an action asks of the machine.
pub trait Probe: Sync {
    /// Processes of the current user. `want` keeps a command out before the slow lookups.
    fn processes(&self, want: &dyn Fn(&str) -> bool) -> Vec<Proc>;
    /// Owner and start time of one pid now.
    fn proc_info(&self, pid: u32) -> Option<Info>;
    /// SIGTERM.
    fn terminate(&self, pid: u32) -> std::io::Result<()>;
    /// Running containers (none when docker is not there).
    fn docker(&self) -> Vec<Docker>;
    /// `docker stop <id>`.
    fn docker_stop(&self, id: &str) -> Result<String, String>;
    /// Run git in `dir`; stdout when it succeeded. Never writes.
    fn git(&self, dir: &Path, args: &[&str]) -> Option<String>;
    /// Run a git command that changes things (worktree remove): the text git said, or its refusal.
    fn git_act(&self, dir: &Path, args: &[&str]) -> Result<String, String>;
    /// Scratchpad roots of the agents.
    fn scratch_roots(&self) -> Vec<PathBuf>;
}

/// The real machine.
pub struct System {
    roots: Vec<PathBuf>,
}

impl System {
    pub fn new() -> System {
        System { roots: vec![] }
    }

    /// With the scratchpad roots of these agents.
    pub fn with_roots(roots: Vec<PathBuf>) -> System {
        System { roots }
    }
}

impl Default for System {
    fn default() -> Self {
        System::new()
    }
}

/// Run `cmd` for at most `limit`; stdout, stderr and whether it succeeded. A call that takes
/// longer is killed (it is a child of ours) and gives None.
pub fn run_timeout(mut cmd: Command, limit: Duration) -> Option<(bool, String, String)> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let (mut out, mut err) = (child.stdout.take()?, child.stderr.take()?);
    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        let mut s = vec![];
        let _ = std::io::Read::read_to_end(&mut out, &mut s);
        let _ = tx.send((0, s));
    });
    std::thread::spawn(move || {
        let mut s = vec![];
        let _ = std::io::Read::read_to_end(&mut err, &mut s);
        let _ = tx2.send((1, s));
    });
    let deadline = Instant::now() + limit;
    let ok = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(3));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let (mut o, mut e) = (String::new(), String::new());
    for _ in 0..2 {
        if let Ok((which, bytes)) = rx.recv_timeout(Duration::from_secs(2)) {
            let t = String::from_utf8_lossy(&bytes).into_owned();
            if which == 0 {
                o = t;
            } else {
                e = t;
            }
        }
    }
    Some((ok, o, e))
}

fn git_command(dir: &Path, args: &[&str]) -> Command {
    let git = crate::util::find_in("git", &crate::platform::search_path())
        .unwrap_or_else(|| PathBuf::from("git"));
    let mut c = Command::new(git);
    c.arg("--no-optional-locks")
        // a configured file-system monitor would start a daemon and write to the repo
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    c
}

fn docker_path() -> Option<PathBuf> {
    crate::util::find_in("docker", &crate::platform::search_path())
}

/// Parse `docker ps --no-trunc --format '{{json .}}'`, one JSON object per line.
pub fn parse_docker_ps(text: &str) -> Vec<Docker> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let id = s("ID");
            if id.is_empty() {
                return None;
            }
            let labels = s("Labels");
            let workdir = labels.split(',').find_map(|kv| {
                kv.strip_prefix("com.docker.compose.project.working_dir=")
                    .map(str::to_string)
            });
            // `2026-11-21 10:00:00 +0100 CET`
            let created = s("CreatedAt");
            let created = created
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            let created = jiff::Timestamp::strptime("%Y-%m-%d %H:%M:%S %z", &created)
                .map(|t| t.as_second())
                .unwrap_or(0);
            let mounts = s("Mounts")
                .split(',')
                .filter(|m| m.starts_with('/'))
                .map(str::to_string)
                .collect();
            Some(Docker {
                id,
                name: s("Names"),
                image: s("Image"),
                created,
                workdir,
                mounts,
            })
        })
        .collect()
}

impl Probe for System {
    fn processes(&self, want: &dyn Fn(&str) -> bool) -> Vec<Proc> {
        procs::list_own(want)
    }

    fn proc_info(&self, pid: u32) -> Option<Info> {
        procs::info(pid)
    }

    fn terminate(&self, pid: u32) -> std::io::Result<()> {
        procs::terminate(pid)
    }

    fn docker(&self) -> Vec<Docker> {
        let Some(docker) = docker_path() else {
            return vec![];
        };
        let mut c = Command::new(docker);
        c.args(["ps", "--no-trunc", "--format", "{{json .}}"]);
        match run_timeout(c, DOCKER_TIMEOUT) {
            Some((true, out, _)) => parse_docker_ps(&out),
            _ => vec![],
        }
    }

    fn docker_stop(&self, id: &str) -> Result<String, String> {
        let docker = docker_path().ok_or("docker is not on PATH")?;
        let mut c = Command::new(docker);
        c.args(["stop", id]);
        match run_timeout(c, Duration::from_secs(30)) {
            Some((true, out, _)) => Ok(out.trim().to_string()),
            Some((false, out, err)) => Err(format!("{out}{err}").trim().to_string()),
            None => Err("docker did not answer in time".into()),
        }
    }

    fn git(&self, dir: &Path, args: &[&str]) -> Option<String> {
        match run_timeout(git_command(dir, args), GIT_TIMEOUT) {
            Some((true, out, _)) => Some(out),
            _ => None,
        }
    }

    fn git_act(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        match run_timeout(git_command(dir, args), Duration::from_secs(30)) {
            Some((true, out, err)) => Ok(format!("{out}{err}").trim().to_string()),
            Some((false, out, err)) => Err(format!("{out}{err}").trim().to_string()),
            None => Err("git did not answer in time".into()),
        }
    }

    fn scratch_roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
}
