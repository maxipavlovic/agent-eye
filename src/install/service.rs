//! The daemon's service as install, uninstall and `board on|off` meet it: what is started,
//! restarted or stopped, and the words that say what became of it. What the service manager prints
//! is captured and never shown (see `platform::service`); a failure is one line in our words.

use super::{Opts, BIN_NAME};
use crate::platform::service::{How, Runner, Started, Step, Stopped};
use crate::report::{Report, Row};
use crate::util::show_path;

/// The area of the daemon's row.
pub const AREA: &str = "daemon";

/// How the service file stands after an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Added,
    Updated,
    Unchanged,
}

/// What a start or a stop says, and whether it worked.
pub struct Said {
    pub text: String,
    pub ok: bool,
}

impl Said {
    fn ok(text: String) -> Said {
        Said { text, ok: true }
    }
    fn failed(text: String) -> Said {
        Said { text, ok: false }
    }
}

fn runner<'a>(o: &Opts, path: &'a std::ffi::OsStr) -> Runner<'a> {
    Runner {
        path,
        timing: o.timing,
    }
}

/// `launch agent dev.agent-eye.daemon`, or the systemd unit.
fn service(o: &Opts) -> String {
    format!("{} {}", o.manager.what(), o.manager.service())
}

/// Commands for a person to run, as text.
fn by_hand(steps: &[Step]) -> String {
    let cmds: Vec<String> = steps.iter().map(|s| format!("`{s}`")).collect();
    cmds.join(", then ")
}

/// A start that did not work, in our words, with the next step.
fn could_not_start(verb: &str, why: &str) -> String {
    format!(
        "could not {verb} ({why}). Run `{} doctor`, or start it with `{BIN_NAME} daemon`.",
        crate::pick::bin_name()
    )
}

/// What a finished start says: what happened, then which service. `restarted` is how a daemon
/// that was running reads (it depends on why it was restarted).
fn started_text(o: &Opts, s: Started, file: FileState, restarted: &str) -> String {
    let mut inside = service(o);
    if s.how == How::Started && file != FileState::Unchanged {
        inside.push_str(if file == FileState::Added {
            ", file added"
        } else {
            ", file updated"
        });
    }
    if let Some(pid) = s.pid {
        inside.push_str(&format!(", pid {pid}"));
    }
    let how = match s.how {
        How::Restarted => restarted.to_string(),
        How::Reloaded => format!("restarted with its new {} file", o.manager.what()),
        How::Started => "started".to_string(),
    };
    format!("{how} ({inside})")
}

/// Start the daemon after an install wrote the service file: restart it in place when it runs from
/// a file that did not change (`reload` says it did), load it again when the file changed, start it
/// when it is not loaded. Where eye may not run the service manager, say what it would do, or what
/// to run.
pub fn start(o: &Opts, reload: bool, file: FileState) -> Said {
    let at = show_path(&o.unit());
    let Some(path) = o.service_path() else {
        if o.dry_run {
            return Said::ok(match file {
                FileState::Added => {
                    format!("would add {at}, then start the daemon ({})", service(o))
                }
                FileState::Updated => {
                    format!(
                        "would update {at}, then restart the daemon ({})",
                        service(o)
                    )
                }
                FileState::Unchanged => format!(
                    "would restart the daemon ({}), or start it when it is not running",
                    service(o)
                ),
            });
        }
        let wrote = match file {
            FileState::Added => format!("added {at}"),
            FileState::Updated => format!("updated {at}"),
            FileState::Unchanged => format!("{at} unchanged"),
        };
        return Said::ok(format!(
            "{wrote}; not started ({}). To start it run {}",
            o.not_run_because(),
            by_hand(&o.manager.start_by_hand(&o.unit()))
        ));
    };
    match o.manager.start(&runner(o, &path), &o.unit(), reload) {
        Ok(s) => Said::ok(started_text(o, s, file, "restarted on the new binary")),
        Err(why) => Said::failed(could_not_start("start", &why)),
    }
}

/// Stop the daemon before an uninstall removes its files: what became of it, or what to run when
/// eye may not.
pub fn stop(o: &Opts) -> Said {
    let Some(path) = o.service_path() else {
        if o.dry_run {
            return Said::ok(format!("would stop ({})", service(o)));
        }
        return Said::ok(format!(
            "not stopped ({}). To stop it run {}",
            o.not_run_because(),
            by_hand(&o.manager.stop_by_hand())
        ));
    };
    match o.manager.stop(&runner(o, &path)) {
        Ok(Stopped::Stopped) => Said::ok(format!("stopped ({})", service(o))),
        Ok(Stopped::NotRunning) => Said::ok("was not running".to_string()),
        Err(why) => Said::failed(format!(
            "could not stop ({why}). Stop it by hand with {}",
            by_hand(&o.manager.stop_by_hand())
        )),
    }
}

/// `board on|off`: start the daemon, or restart it so the setting applies. A service file that
/// runs another eye home is not ours to restart.
pub fn restart(o: &Opts) -> Report {
    let mut rep = Report::default();
    let said = match o.service_path() {
        Some(path) if o.unit_serves_home() => {
            let file = FileState::Unchanged;
            match o.manager.start(&runner(o, &path), &o.unit(), false) {
                Ok(s) => Said::ok(started_text(o, s, file, "restarted")),
                Err(why) => Said::failed(could_not_start("restart", &why)),
            }
        }
        Some(_) => Said::ok(format!(
            "not restarted (no {} file for this eye home); restart the daemon to apply",
            o.manager.what()
        )),
        None => Said::ok(format!(
            "not restarted ({}); restart the daemon to apply",
            o.not_run_because()
        )),
    };
    rep.failed = !said.ok;
    rep.push(Row::new(AREA, said.text));
    rep
}
