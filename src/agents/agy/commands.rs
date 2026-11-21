use crate::agents::{Invocation, StartPlan, StartRequest};
use anyhow::Result;

/// The program name shown in commands.
pub const PROGRAM: &str = "agy";

/// Resume a conversation by id (`agy --conversation <id>`). agy keeps the conversation's own
/// folder, and eye runs the line in it.
pub fn resume(session_id: &str) -> Invocation {
    Invocation::new(PROGRAM, &["--conversation", session_id])
}

/// A first message that starts with `-` would be read as a flag.
fn safe_prompt(raw: &str) -> String {
    if raw.starts_with('-') {
        format!("Task: {raw}")
    } else {
        raw.to_string()
    }
}

/// agy in the idea's folder: with a prompt it runs it interactively (`-i`) and goes on with you.
/// agy cannot be told the id of the conversation it creates, so the plan has none and the scanner
/// links the idea to the conversation that starts there next.
pub fn start(req: &StartRequest) -> Result<StartPlan> {
    anyhow::ensure!(
        !req.background,
        "agy cannot run a session in the background: start the idea in the foreground"
    );
    let mut args = vec![];
    if let Some(p) = req.prompt {
        args.push("-i".to_string());
        args.push(safe_prompt(p));
    }
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

    fn req<'a>(prompt: Option<&'a str>, background: bool) -> StartRequest<'a> {
        StartRequest {
            dir: "/tmp/x y",
            title: "t",
            prompt,
            background,
            session_id: Some("ignored"),
        }
    }

    #[test]
    fn resume_and_start_lines() {
        assert_eq!(
            resume("abcd-1234").shell_line("/tmp/a b"),
            "cd '/tmp/a b' && agy --conversation abcd-1234"
        );
        let p = start(&req(Some("do the thing"), false)).unwrap();
        assert_eq!(p.session_id, None);
        assert_eq!(
            p.invocation.shell_line("/x"),
            "cd '/x' && agy -i 'do the thing'"
        );
        let p = start(&req(Some("--dangerously-skip-permissions"), false)).unwrap();
        assert!(p
            .invocation
            .shell_line("/x")
            .ends_with("'Task: --dangerously-skip-permissions'"));
        assert_eq!(
            start(&req(None, false))
                .unwrap()
                .invocation
                .shell_line("/x"),
            "cd '/x' && agy"
        );
        assert!(start(&req(Some("go"), true))
            .unwrap_err()
            .to_string()
            .contains("background"));
    }
}
