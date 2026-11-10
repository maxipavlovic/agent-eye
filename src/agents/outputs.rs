//! What a piece of transcript text says about the work: Jira keys, pull requests, commits. An
//! adapter hands over the text of a message or a tool call and gets the [`Delta`] filled in, so
//! every agent finds outputs by the same rules.

use super::{Delta, OutputHit};
use crate::util::head_chars;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

/// Most bytes of one tool result that are searched for pull requests and Jira links.
pub const RESULT_SCAN_CAP: usize = 32_768;

/// Outputs already reported: `(kind, value)`, so a value counts once however often it is seen.
pub type Seen = HashSet<(&'static str, String)>;

static RE_PR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https://bitbucket\.org/[^/\s"]+/[^/\s"]+/pull-requests/\d+|https://github\.com/[^/\s"]+/[^/\s"]+/pull/\d+"#)
        .expect("static regex")
});
static RE_JIRA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z][A-Z0-9]{1,9}-\d{1,6}\b").expect("static regex"));
static RE_JIRA_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([a-z0-9-]+\.atlassian\.net)/browse/([A-Z][A-Z0-9]{1,9})-\d{1,6}")
        .expect("static regex")
});
static RE_JIRA_BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[^A-Za-z0-9])([A-Z][A-Z0-9]{1,9})-\d{1,6}").expect("static regex")
});
static RE_COMMIT_MSG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)(?:\s-m|--message)[=\s]\s*(?:"((?:[^"\\]|\\.)*)"|'([^']*)')"#)
        .expect("static regex")
});
/// `git commit` actually run: at the start of the command or after `&&`, `;`, `|` or `(`, not just
/// the words inside a script or a quoted string.
static RE_GIT_COMMIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|&&|;|\|\|?|\(|\n)\s*git\s+(?:-C\s+\S+\s+)*commit\b").expect("static regex")
});
/// `git commit -F - <<'EOF'` (or `--file=-`): the message is the heredoc body.
static RE_COMMIT_HEREDOC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)(?:\s-F\s*-|--file[=\s]-)\s.*?<<-?\s*['"]?(\w+)['"]?[^\n]*\n(.*)"#)
        .expect("static regex")
});
static RE_COMMIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[^\]\s]+ ([0-9a-f]{7,40})\]").expect("static regex"));

const JIRA_DENY: &[&str] = &[
    "UTF", "ISO", "SHA", "RFC", "TLS", "SSL", "HTTP", "HTTPS", "PEP", "CVE", "GPT", "AES", "RSA",
    "ECDSA", "X",
];

/// Is `dir` one of the folders in `ignore`, or inside one?
pub fn dir_ignored(dir: &str, ignore: &[String]) -> bool {
    ignore.iter().any(|p| {
        let p = p.trim_end_matches('/');
        !p.is_empty() && (dir == p || dir.strip_prefix(p).is_some_and(|r| r.starts_with('/')))
    })
}

/// The first `max` bytes of `t`, cut on a character boundary.
pub fn cap_str(t: &str, max: usize) -> &str {
    if t.len() <= max {
        return t;
    }
    let mut e = max;
    while !t.is_char_boundary(e) {
        e -= 1;
    }
    &t[..e]
}

/// Report an output once.
pub fn push_output(
    d: &mut Delta,
    seen: &mut Seen,
    kind: &'static str,
    value: String,
    url: Option<String>,
    title: Option<String>,
    ts: i64,
) {
    if seen.insert((kind, value.clone())) {
        d.outputs.push(OutputHit {
            kind,
            value,
            url,
            title,
            ts,
        });
    }
}

/// Collect Jira-shaped keys; returns the prefixes found. Whether a key is shown is decided
/// when it is stored (known projects only).
pub fn scan_jira(d: &mut Delta, seen: &mut Seen, text: &str, ts: i64) -> Vec<String> {
    let text = cap_str(text, 65_536);
    for m in RE_JIRA_URL.captures_iter(text) {
        d.jira_url_prefixes.insert(m[2].to_string());
        d.jira_hosts.insert(m[2].to_string(), m[1].to_string());
    }
    let mut prefixes = vec![];
    for m in RE_JIRA.find_iter(text) {
        let key = m.as_str();
        let (prefix, num) = key.split_once('-').unwrap_or((key, ""));
        if JIRA_DENY.contains(&prefix) || num.starts_with('0') {
            continue;
        }
        // OBJ-467-638 is an object id with a second number, not a Jira key
        let rest = &text.as_bytes()[m.end()..];
        if rest.len() > 1 && rest[0] == b'-' && rest[1].is_ascii_digit() {
            continue;
        }
        prefixes.push(prefix.to_string());
        if seen.insert(("jira", key.to_string())) {
            d.jira.push((key.to_string(), ts));
        }
    }
    prefixes
}

/// Jira site links in a tool result or command: they tell which site a project prefix lives on.
pub fn scan_jira_urls(d: &mut Delta, text: &str) {
    if !text.contains("atlassian.net") {
        return;
    }
    for m in RE_JIRA_URL.captures_iter(cap_str(text, RESULT_SCAN_CAP)) {
        d.jira_url_prefixes.insert(m[2].to_string());
        d.jira_hosts.insert(m[2].to_string(), m[1].to_string());
    }
}

/// Jira project prefixes in a git branch name (`feature/PROJ-12-x`).
pub fn scan_jira_branch(d: &mut Delta, branch: &str) {
    for c in RE_JIRA_BRANCH.captures_iter(branch) {
        d.jira_branch_prefixes.insert(c[1].to_string());
    }
}

/// Pull request links in `text`.
pub fn scan_pr(d: &mut Delta, seen: &mut Seen, text: &str, ts: i64) {
    if !text.contains("pull") {
        return;
    }
    for m in RE_PR.find_iter(cap_str(text, RESULT_SCAN_CAP)) {
        let u = m.as_str().to_string();
        push_output(d, seen, "pr", u.clone(), Some(u), None, ts);
    }
}

/// Does the shell command run `git commit`?
pub fn is_git_commit(cmd: &str) -> bool {
    RE_GIT_COMMIT.is_match(cmd)
}

/// The short hash in what `git commit` printed (`[main 1a2b3c4] message`).
pub fn commit_sha(output: &str) -> Option<String> {
    RE_COMMIT
        .captures(cap_str(output, RESULT_SCAN_CAP))
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// First line of the `-m` message of a `git commit` command.
pub fn commit_message(cmd: &str) -> Option<String> {
    let Some(c) = RE_COMMIT_MSG.captures(cmd) else {
        let h = RE_COMMIT_HEREDOC.captures(cmd)?;
        let end = &h[1];
        let line = h[2]
            .lines()
            .map(str::trim)
            .take_while(|l| *l != end)
            .find(|l| !l.is_empty())?;
        return Some(head_chars(line, 120));
    };
    let raw = c.get(1).or_else(|| c.get(2))?.as_str();
    let raw = match raw.find("<<") {
        // -m "$(cat <<'EOF' ... )": the message starts on the line after the marker
        Some(i) => raw[i..].split_once('\n').map_or("", |(_, rest)| rest),
        None => raw,
    };
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(head_chars(line, 120))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_git_commits_count() {
        for c in [
            "git commit -m x",
            "cd /a && git commit -q -F - <<'EOF'",
            "git -C /a commit -m x",
            "make && git commit",
        ] {
            assert!(is_git_commit(c), "{c}");
        }
        for c in [
            "python3 -c \"if 'git commit' not in line: pass\"",
            "grep -n 'git commit' f",
            "echo git commit",
        ] {
            assert!(!is_git_commit(c), "{c}");
        }
    }

    #[test]
    fn commit_message_from_a_heredoc_file() {
        let cmd = "git add a.py && git commit -q -F - <<'EOF'\nPROJ-123 Keep legacy currencies\n\nLonger body.\nEOF";
        assert_eq!(
            commit_message(cmd).as_deref(),
            Some("PROJ-123 Keep legacy currencies")
        );
        assert_eq!(
            commit_message("git commit --file=- <<EOF\nFix it\nEOF").as_deref(),
            Some("Fix it")
        );
        assert_eq!(
            commit_message("git commit -m \"Plain\"").as_deref(),
            Some("Plain")
        );
        assert_eq!(commit_message("git commit -F - <<'EOF'\nEOF"), None);
    }

    #[test]
    fn the_hash_is_read_from_the_commit_summary() {
        assert_eq!(
            commit_sha("[main 1a2b3c4] Fix it\n 1 file changed").as_deref(),
            Some("1a2b3c4")
        );
        assert_eq!(commit_sha("ok 1 file changed"), None);
    }
}
