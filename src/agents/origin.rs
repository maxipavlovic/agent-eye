//! Where a session was started, as a small set of words, mapped from what the agent wrote.
//!
//! Claude Code writes `"entrypoint"` on its transcript lines (its `CLAUDE_CODE_ENTRYPOINT`). The
//! values below are the ones Claude Code 2.1.311 knows (read from the binary). Codex writes an
//! `originator` and a `source` in `session_meta`; the adapter joins them as `originator/source`
//! (`codex-tui/vscode`). Anything not listed maps to None: a name is never guessed.

use std::path::Path;

/// The words a session's origin can have, in the order the filter lists them.
pub const ORIGINS: [&str; 7] = [
    "terminal", "headless", "app", "desktop", "ide", "web", "night",
];

/// Map what an agent wrote to an origin word. `raw` is the entrypoint (Claude Code) or
/// `originator/source` (Codex). None when unknown.
pub fn classify(agent: &str, raw: &str) -> Option<&'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    match agent {
        "claude" => claude(raw),
        "codex" => codex(raw),
        _ => None,
    }
}

fn claude(raw: &str) -> Option<&'static str> {
    Some(match raw {
        "cli" => "terminal",
        // `claude -p`: the GitHub action and a plain headless run
        "sdk-cli" | "claude-code-github-action" => "headless",
        // the Agent SDK for TypeScript and Python, and Claude Code as an MCP server: apps that
        // drive it (T3 Code, Conductor)
        "sdk-ts" | "sdk-py" | "mcp" => "app",
        "claude-desktop"
        | "claude-desktop-3p"
        | "local-agent"
        | "local_agent"
        | "claude-coworker"
        | "claude-coworker-terminal" => "desktop",
        "claude-vscode" => "ide",
        "remote"
        | "remote_baku"
        | "remote_cowork"
        | "remote_trigger"
        | "remote_cowork_trigger"
        | "remote_desktop"
        | "remote_mobile"
        | "remote_projects"
        | "ssh-remote"
        | "claude_in_slack"
        | "claude-in-slack"
        | "claude-in-teams" => "web",
        _ => return None,
    })
}

fn codex(raw: &str) -> Option<&'static str> {
    let (originator, source) = raw.split_once('/').unwrap_or((raw, ""));
    // the source says how the thread was started; the originator names the program
    match source {
        "vscode" => return Some("ide"),
        "exec" => return Some("headless"),
        "mcp" => return Some("app"),
        "custom:atlas" | "custom:chatgpt" => return Some("desktop"),
        _ => {}
    }
    match originator {
        "codex_cli_rs" | "codex-tui" => Some("terminal"),
        "codex_exec" => Some("headless"),
        "codex_vscode" => Some("ide"),
        "codex_atlas" | "codex_chatgpt_desktop" => Some("desktop"),
        o if o.starts_with("Codex ") => Some("desktop"),
        _ => None,
    }
}

/// The origin to store: a session whose folder is under eye's `night` folder is a night run; any
/// other follows what the agent wrote.
pub fn of_session(
    agent: &str,
    raw: Option<&str>,
    cwd: Option<&str>,
    night_dir: Option<&Path>,
) -> Option<&'static str> {
    if crate::night::is_night_folder(night_dir, cwd) {
        return Some("night");
    }
    raw.and_then(|r| classify(agent, r))
}

/// The sentence for the picker preview: "an app (sdk-ts)".
pub fn phrase(origin: &str, raw: Option<&str>) -> String {
    let what = match origin {
        "terminal" => "a terminal",
        "headless" => "a headless run",
        "app" => "an app",
        "desktop" => "the desktop app",
        "ide" => "an editor",
        "web" => "the web or a remote",
        "night" => "a night run",
        o => o,
    };
    match raw.filter(|r| !r.is_empty()) {
        Some(r) => format!("{what} ({r})"),
        None => what.to_string(),
    }
}

/// The word shown on a chip, None for the terminal (the normal case).
pub fn chip(origin: &str) -> Option<&'static str> {
    Some(match origin {
        "headless" => "Headless",
        "app" => "App",
        "desktop" => "Desktop",
        "ide" => "IDE",
        "web" => "Web",
        "night" => "Night",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_entrypoints_map_and_unknown_is_none() {
        for (raw, want) in [
            ("cli", "terminal"),
            ("sdk-cli", "headless"),
            ("sdk-ts", "app"),
            ("sdk-py", "app"),
            ("claude-vscode", "ide"),
            ("claude-desktop", "desktop"),
            ("local-agent", "desktop"),
            ("remote_mobile", "web"),
        ] {
            assert_eq!(classify("claude", raw), Some(want), "{raw}");
        }
        assert_eq!(classify("claude", "something-new"), None);
        assert_eq!(classify("claude", ""), None);
        assert_eq!(classify("agy", "cli"), None);
    }

    #[test]
    fn codex_source_wins_over_originator() {
        assert_eq!(classify("codex", "codex_cli_rs/cli"), Some("terminal"));
        assert_eq!(classify("codex", "codex_exec/exec"), Some("headless"));
        assert_eq!(classify("codex", "codex-tui/vscode"), Some("ide"));
        assert_eq!(classify("codex", "codex_vscode"), Some("ide"));
        assert_eq!(classify("codex", "Codex Desktop"), Some("desktop"));
        assert_eq!(classify("codex", "mystery/odd"), None);
    }

    #[test]
    fn a_folder_under_the_night_dir_is_a_night_run() {
        let night = Path::new("/home/x/.agent-eye/night");
        let got = of_session(
            "claude",
            Some("sdk-cli"),
            Some("/home/x/.agent-eye/night/3/wt"),
            Some(night),
        );
        assert_eq!(got, Some("night"));
        let got = of_session("claude", Some("sdk-cli"), Some("/work/p"), Some(night));
        assert_eq!(got, Some("headless"));
        assert_eq!(
            of_session("claude", None, Some("/work/p"), Some(night)),
            None
        );
        // the user's own folder that looks like a run folder is not one
        assert_eq!(
            of_session(
                "claude",
                Some("sdk-cli"),
                Some("/work/night/7/wt"),
                Some(night)
            ),
            Some("headless")
        );
    }
}
