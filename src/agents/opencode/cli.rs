//! Things eye asks the `opencode` program to do: export, import and delete a session. OpenCode
//! owns its database, so eye never writes it.

use super::{store, OpenCode};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(60);

impl OpenCode {
    /// The variables that point `opencode` at the folders this adapter reads, so a data or config
    /// folder that is not the usual one is the one the program uses too.
    pub(super) fn env(&self) -> Vec<(String, String)> {
        let mut v = vec![];
        for (dir, var) in [
            (&self.dir, "XDG_DATA_HOME"),
            (&self.config_dir, "XDG_CONFIG_HOME"),
        ] {
            if dir.file_name().is_some_and(|n| n == "opencode") {
                if let Some(p) = dir.parent() {
                    v.push((var.to_string(), p.to_string_lossy().into_owned()));
                }
            }
        }
        v
    }

    /// Run `opencode <args>` in `cwd`; its stdout. A run that takes more than a minute is stopped.
    pub(super) fn output(&self, args: &[&str], cwd: Option<&Path>) -> Result<String> {
        let mut cmd = Command::new(self.executable_path());
        cmd.args(args);
        if args.first() == Some(&"session") {
            // a private server for the call: eye never starts OpenCode's background service
            cmd.arg("--standalone");
        }
        cmd.envs(self.env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(c) = cwd {
            cmd.current_dir(c);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting `{}` (is it on PATH?)", self.executable_path()))?;
        let out = child.stdout.take().map(super::headless::drain);
        let err = child.stderr.take().map(super::headless::drain);
        let start = Instant::now();
        let status = loop {
            if let Some(st) = child.try_wait()? {
                break st;
            }
            if start.elapsed() > TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(
                    "`opencode {}` timed out",
                    args.first().copied().unwrap_or("")
                );
            }
            std::thread::sleep(Duration::from_millis(30));
        };
        let out = out
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let err = err
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        anyhow::ensure!(
            status.success(),
            "`opencode {}` failed ({status}): {}",
            args.join(" "),
            crate::util::head_chars(String::from_utf8_lossy(&err).trim(), 300)
        );
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Delete a session through OpenCode; nothing is said when it cannot.
    pub(super) fn delete_session(&self, id: &str) {
        let _ = self.output(&["session", "delete", id], None);
    }

    /// Delete the sessions a summary run made in `dir` (when its own events did not name them):
    /// those titled `eye-ask`, from the project's session list.
    pub(super) fn delete_ask_sessions(&self, dir: &Path) {
        let Ok(list) = self.output(&["session", "list"], Some(dir)) else {
            return;
        };
        for line in list.lines() {
            let mut f = line.split('\t');
            if let (Some(id), Some("eye-ask")) = (f.next(), f.next()) {
                self.delete_session(id.trim());
            }
        }
    }

    /// What `opencode session export` says of a session of the database `path` names.
    pub(super) fn export_native(&self, id: &str) -> Option<Value> {
        let out = self.output(&["session", "export", id], None).ok()?;
        let v: Value = serde_json::from_str(out.trim()).ok()?;
        (v.get("info").is_some() && v.get("messages").is_some()).then_some(v)
    }

    /// Put an exported session back with `opencode session import`, in the folder it had when that
    /// is still there. The session keeps its id.
    pub(super) fn import_native(&self, native: &Value, tmp: &Path) -> Result<()> {
        let dir = native
            .pointer("/info/location/directory")
            .and_then(Value::as_str)
            .filter(|d| Path::new(d).is_dir())
            .map(String::from);
        crate::files::write_file(tmp, &serde_json::to_vec(native)?, 0o600)?;
        let tmp_s = tmp.to_string_lossy().into_owned();
        let mut args = vec!["session", "import", tmp_s.as_str()];
        if let Some(d) = dir.as_deref() {
            args.extend(["--directory", d]);
        }
        let r = self.output(&args, None);
        let _ = std::fs::remove_file(tmp);
        r.map(drop)
    }

    /// Is the session of the database path `dest` known to OpenCode (and eye's reading of it)?
    pub(super) fn has_session(&self, dest: &Path) -> bool {
        store::exists(dest)
    }
}
