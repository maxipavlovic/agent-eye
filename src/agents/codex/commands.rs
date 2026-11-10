use crate::agents::{Invocation, StartPlan, StartRequest};
use anyhow::Result;

/// The program name shown in commands.
pub const PROGRAM: &str = "codex";

/// Resume a session in place. Codex takes the session by id from any folder, but asks which folder
/// to use when it is not the one it was started in, so eye runs it in the last folder.
pub fn resume(session_id: &str) -> Invocation {
    Invocation::new(PROGRAM, &["resume", session_id])
}

/// A new session that branches from `session_id`; the original stays untouched.
pub fn fork(session_id: &str) -> Invocation {
    Invocation::new(PROGRAM, &["fork", session_id])
}

/// A first message that starts with `-` would be read as a flag, and a single word could be the name
/// of a subcommand (`review`, `login`, `exec`): both get a `Task:` prefix.
fn safe_prompt(raw: &str) -> String {
    if raw.starts_with('-') || !raw.contains(char::is_whitespace) {
        format!("Task: {raw}")
    } else {
        raw.to_string()
    }
}

/// Codex in the idea's folder (the line `cd`s there), with the idea's prompt as its first message
/// or waiting for you without one. Codex cannot be told the id of the session it creates, so the
/// plan has none: the scanner links the idea to the session that starts there next. Codex has no
/// background sessions.
pub fn start(req: &StartRequest) -> Result<StartPlan> {
    anyhow::ensure!(
        !req.background,
        "Codex CLI cannot run a session in the background (it has no --bg): start the idea in the foreground"
    );
    let args: Vec<String> = req.prompt.map(safe_prompt).into_iter().collect();
    Ok(StartPlan {
        invocation: Invocation {
            program: PROGRAM.into(),
            args,
        },
        session_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "019a6b2e-7c10-7a41-9d3e-5f0c2b7a1e44";

    #[test]
    fn resume_and_fork_lines() {
        assert_eq!(
            resume(ID).shell_line("/tmp/a b"),
            format!("cd '/tmp/a b' && codex resume {ID}")
        );
        assert_eq!(
            fork(ID).shell_line("/x"),
            format!("cd '/x' && codex fork {ID}")
        );
        assert_eq!(
            resume("a'; touch /tmp/pwn #").shell_line("/x"),
            "cd '/x' && codex resume 'a'\\''; touch /tmp/pwn #'"
        );
        assert_eq!(
            resume(ID).shell_line("-P"),
            format!("cd -- '-P' && codex resume {ID}")
        );
    }

    #[test]
    fn start_lines() {
        let req = |prompt, background| StartRequest {
            dir: "/tmp/x y",
            title: "Fix it",
            prompt,
            background,
            session_id: None,
        };
        let p = start(&req(Some("do the thing"), false)).unwrap();
        assert_eq!(p.session_id, None, "Codex picks the id itself");
        assert_eq!(
            p.invocation.shell_line("/tmp/x y"),
            "cd '/tmp/x y' && codex 'do the thing'"
        );
        let p = start(&req(None, false)).unwrap();
        assert_eq!(p.invocation.shell_line("/x"), "cd '/x' && codex");
        let p = start(&req(
            Some("--dangerously-bypass-approvals-and-sandbox"),
            false,
        ))
        .unwrap();
        assert!(p
            .invocation
            .shell_line("/x")
            .ends_with("'Task: --dangerously-bypass-approvals-and-sandbox'"));
        // one word could be a subcommand of codex
        let p = start(&req(Some("review"), false)).unwrap();
        assert_eq!(
            p.invocation.shell_line("/x"),
            "cd '/x' && codex 'Task: review'"
        );
        // a session id eye holds from an earlier start is of no use to Codex
        let mut again = req(Some("go"), false);
        again.session_id = Some(ID);
        assert_eq!(start(&again).unwrap().session_id, None);
        let e = start(&req(Some("go"), true)).unwrap_err().to_string();
        assert!(e.contains("no --bg") && e.contains("foreground"), "{e}");
    }
}
