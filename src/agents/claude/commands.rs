//! The commands that resume, fork and start Claude Code sessions.

use crate::agents::{Invocation, StartPlan, StartRequest};
use crate::util::new_uuid;
use anyhow::{Context, Result};
use regex::Regex;
use std::sync::LazyLock;

/// The program name shown in commands.
pub const PROGRAM: &str = "claude";

static RE_BG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"backgrounded\s*\u{b7}\s*([0-9a-f]{6,12})\b").expect("static regex")
});

/// Resume a session in place.
pub fn resume(session_id: &str) -> Invocation {
    Invocation::new(PROGRAM, &["--resume", session_id])
}

/// A new session that branches from `session_id`; the original stays untouched.
pub fn fork(session_id: &str) -> Invocation {
    Invocation::new(PROGRAM, &["--resume", session_id, "--fork-session"])
}

/// A first message that starts with `-` would be read as a flag (e.g.
/// `--dangerously-skip-permissions`), so it gets a `Task:` prefix.
fn safe_prompt(raw: &str) -> String {
    if raw.starts_with('-') {
        format!("Task: {raw}")
    } else {
        raw.to_string()
    }
}

/// Foreground picks the session id up front (or reuses the one given) and, without a prompt, opens
/// Claude with nothing sent. `--bg` needs a prompt and ignores `--session-id`, so that session is
/// found later through its job.
pub fn start(req: &StartRequest) -> Result<StartPlan> {
    let prompt = req.prompt.map(safe_prompt);
    if req.background {
        let prompt = prompt.context(
            "starting in the background needs a prompt: add what Claude should do first",
        )?;
        return Ok(StartPlan {
            invocation: Invocation {
                program: PROGRAM.into(),
                args: vec!["--bg".into(), "-n".into(), req.title.into(), prompt],
            },
            session_id: None,
        });
    }
    let id = match req.session_id {
        Some(s) => s.to_string(),
        None => new_uuid().context("choosing a session id")?,
    };
    let mut args = vec![
        "--session-id".to_string(),
        id.clone(),
        "-n".into(),
        req.title.into(),
    ];
    args.extend(prompt);
    Ok(StartPlan {
        invocation: Invocation {
            program: PROGRAM.into(),
            args,
        },
        session_id: Some(id),
    })
}

/// The short job id in what `claude --bg` printed ("backgrounded · 1a2b3c4d").
pub fn background_job_id(output: &str) -> Option<String> {
    RE_BG.captures(output).map(|c| c[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "30194328-aaaa-4bbb-8ccc-0123456789ab";

    #[test]
    fn resume_and_fork_lines() {
        assert_eq!(
            resume(ID).shell_line("/tmp/a b"),
            format!("cd '/tmp/a b' && claude --resume {ID}")
        );
        assert_eq!(
            fork(ID).shell_line("/x"),
            format!("cd '/x' && claude --resume {ID} --fork-session")
        );
        // a hostile id is quoted, not run
        assert!(fork("a; id")
            .shell_line("/x")
            .ends_with("--resume 'a; id' --fork-session"));
        assert_eq!(
            resume("a'; touch /tmp/pwn #").shell_line("/x"),
            "cd '/x' && claude --resume 'a'\\''; touch /tmp/pwn #'"
        );
    }

    #[test]
    fn a_folder_that_looks_like_an_option_is_not_one() {
        assert_eq!(
            resume(ID).shell_line("-P"),
            format!("cd -- '-P' && claude --resume {ID}")
        );
        assert_eq!(
            resume(ID).shell_line("relative dir"),
            format!("cd -- 'relative dir' && claude --resume {ID}")
        );
    }

    #[test]
    fn start_lines() {
        let req = |prompt, background, session_id| StartRequest {
            dir: "/tmp/x y",
            title: "Fix it",
            prompt,
            background,
            session_id,
        };
        let p = start(&req(Some("do the thing"), false, Some(ID))).unwrap();
        assert_eq!(p.session_id.as_deref(), Some(ID));
        assert_eq!(
            p.invocation.shell_line("/tmp/x y"),
            format!("cd '/tmp/x y' && claude --session-id {ID} -n 'Fix it' 'do the thing'")
        );
        // no prompt: Claude opens and waits
        let p = start(&req(None, false, Some(ID))).unwrap();
        assert!(p.invocation.shell_line("/x").ends_with("-n 'Fix it'"));
        // a new id is picked when none is given
        let p = start(&req(None, false, None)).unwrap();
        assert!(crate::util::valid_session_id(&p.session_id.unwrap()));
        // a prompt that looks like a flag is not one
        let p = start(&req(
            Some("--dangerously-skip-permissions"),
            false,
            Some(ID),
        ))
        .unwrap();
        assert!(p
            .invocation
            .shell_line("/x")
            .ends_with("'Task: --dangerously-skip-permissions'"));
        // background needs a prompt and has no session id yet
        assert!(start(&req(None, true, None)).is_err());
        let p = start(&req(Some("go"), true, None)).unwrap();
        assert!(p.session_id.is_none());
        assert_eq!(
            p.invocation.shell_line("/x"),
            "cd '/x' && claude --bg -n 'Fix it' go"
        );
    }

    #[test]
    fn finds_the_job_id() {
        assert_eq!(
            background_job_id("started\nbackgrounded · 1a2b3c4d\n").as_deref(),
            Some("1a2b3c4d")
        );
        assert_eq!(background_job_id("Workspace not trusted"), None);
    }
}
