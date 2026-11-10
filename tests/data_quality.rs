//! A5 data-quality fixes: pending tool results across scans, commits without a sha, Jira
//! project learning, ignored folders, live entries without a transcript.

use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

const S: &str = "dddddddd-0000-4000-8000-000000000001";
const T: &str = "dddddddd-0000-4000-8000-000000000002";

struct Env {
    _tmp: tempfile::TempDir,
    claude: PathBuf,
    #[allow(dead_code)]
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        std::fs::create_dir_all(claude.join("projects/-work-p")).unwrap();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        paths.ensure_home().unwrap();
        let db = Db::open(&paths.db_path()).unwrap();
        Env {
            _tmp: tmp,
            cfg: Config::with_claude_dir(&claude),
            claude,
            paths,
            db,
        }
    }
    fn path(&self, id: &str) -> PathBuf {
        self.claude
            .join("projects")
            .join("-work-p")
            .join(format!("{id}.jsonl"))
    }
    fn append(&self, id: &str, lines: &[serde_json::Value]) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(id))
            .unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }
    fn scan(&mut self) {
        scan(&mut self.db, &self.cfg, false).unwrap();
    }
    fn outs(&self, id: &str, kind: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .db
            .outputs_for(id)
            .unwrap()
            .into_iter()
            .filter(|o| o.kind == kind)
            .map(|o| o.value)
            .collect();
        v.sort();
        v
    }
}

fn user(id: &str, ts: &str, text: &str) -> serde_json::Value {
    json!({"type":"user","timestamp":ts,"cwd":"/work/p","sessionId":id,"gitBranch":"main",
           "message":{"content":text}})
}
fn tool_use(
    id: &str,
    ts: &str,
    tid: &str,
    name: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    json!({"type":"assistant","timestamp":ts,"cwd":"/work/p","sessionId":id,
           "message":{"content":[{"type":"tool_use","id":tid,"name":name,"input":input}]}})
}
fn tool_result(id: &str, ts: &str, tid: &str, text: &str, is_error: bool) -> serde_json::Value {
    json!({"type":"user","timestamp":ts,"cwd":"/work/p","sessionId":id,
           "message":{"content":[{"type":"tool_result","tool_use_id":tid,"content":text,"is_error":is_error}]}})
}

#[test]
fn output_survives_a_scan_between_tool_use_and_result() {
    let mut e = Env::new();
    e.append(
        S,
        &[
            user(S, "2026-11-01T10:00:00Z", "publish it"),
            tool_use(
                S,
                "2026-11-01T10:00:01Z",
                "t1",
                "Artifact",
                json!({"title":"Plan","file_path":"/work/p/plan.html"}),
            ),
            tool_use(
                S,
                "2026-11-01T10:00:02Z",
                "t2",
                "Bash",
                json!({"command":"git commit -m \"Fix the thing\""}),
            ),
        ],
    );
    e.scan();
    assert!(e.outs(S, "artifact").is_empty() && e.outs(S, "commit").is_empty());
    e.append(
        S,
        &[
            tool_result(
                S,
                "2026-11-01T10:00:05Z",
                "t1",
                "Published https://claude.ai/artifact/abcdef12-3456",
                false,
            ),
            tool_result(
                S,
                "2026-11-01T10:00:06Z",
                "t2",
                "ok 1 file changed, 2 insertions(+)",
                false,
            ),
        ],
    );
    e.scan();
    assert_eq!(
        e.outs(S, "artifact"),
        vec!["https://claude.ai/artifact/abcdef12-3456"]
    );
    assert_eq!(
        e.outs(S, "commit"),
        vec!["Fix the thing"],
        "RTK output has no sha"
    );
    // a full re-read gives the same result
    scan(&mut e.db, &e.cfg, true).unwrap();
    assert_eq!(e.outs(S, "commit"), vec!["Fix the thing"]);
}

#[test]
fn failed_commit_and_heredoc_message() {
    let mut e = Env::new();
    e.append(
        S,
        &[
            user(S, "2026-11-01T10:00:00Z", "commit"),
            tool_use(S, "2026-11-01T10:00:01Z", "t1", "Bash", json!({"command":"git commit -m 'nope'"})),
            tool_result(S, "2026-11-01T10:00:02Z", "t1", "nothing to commit", true),
            tool_use(S, "2026-11-01T10:00:03Z", "t2", "Bash",
                json!({"command":"git add -A && git commit -m \"$(cat <<'EOF'\nPROJ-5 heredoc subject\n\nbody\nEOF\n)\""})),
            tool_result(S, "2026-11-01T10:00:04Z", "t2", "ok", false),
            tool_use(S, "2026-11-01T10:00:05Z", "t3", "Bash", json!({"command":"git commit --amend --no-edit"})),
            tool_result(S, "2026-11-01T10:00:06Z", "t3", "[main abcdef1] x", false),
        ],
    );
    e.scan();
    assert_eq!(
        e.outs(S, "commit"),
        vec!["PROJ-5 heredoc subject", "abcdef1"]
    );
}

#[test]
fn jira_prefixes_are_learned() {
    let mut e = Env::new();
    // PROJ: seen in a browse URL; BR: in a branch; ONCE: typed once; TYPED: typed in 2 sessions
    let mut l = user(
        S,
        "2026-11-01T10:00:00Z",
        "see https://x.atlassian.net/browse/PROJ-123 and PR-0001 and ONCE-2026",
    );
    l["gitBranch"] = json!("bugfix/BR-4172-thing");
    e.append(
        S,
        &[
            l,
            user(
                S,
                "2026-11-01T10:01:00Z",
                "also TYPED-12 and NF-2 and OBJ-467-638",
            ),
        ],
    );
    e.append(
        T,
        &[user(
            T,
            "2026-11-01T11:00:00Z",
            "continue TYPED-12 and OBJ-467-638",
        )],
    );
    e.scan();
    let mut all = e.outs(S, "jira");
    all.extend(e.outs(T, "jira"));
    all.sort();
    assert_eq!(all, vec!["PROJ-123", "TYPED-12", "TYPED-12"], "{all:?}");
    // evidence arriving later brings the earlier candidates in without a rescan of the file
    e.append(
        T,
        &[user(
            T,
            "2026-11-01T12:00:00Z",
            "https://x.atlassian.net/browse/NF-9",
        )],
    );
    e.scan();
    assert_eq!(
        e.outs(S, "jira")
            .iter()
            .filter(|k| k.starts_with("NF"))
            .count(),
        1
    );
    // the config override replaces learning
    e.cfg.outputs.jira_projects = vec!["ONCE".into()];
    e.scan();
    assert_eq!(e.outs(S, "jira"), vec!["ONCE-2026"]);
    assert!(e.outs(T, "jira").is_empty());
}

#[test]
fn claude_dir_and_temp_folders_are_dropped() {
    let mut e = Env::new();
    let home = std::env::var("HOME").unwrap_or_default();
    let w = |p: &str| {
        tool_use(
            S,
            "2026-11-01T10:00:01Z",
            &format!("w{p}"),
            "Write",
            json!({"file_path": p, "content": "x"}),
        )
    };
    e.append(
        S,
        &[
            user(S, "2026-11-01T10:00:00Z", "write"),
            w("/work/p/src/a.rs"),
            w(&format!("{}/memory/m.md", e.claude.display())),
            w(&format!("{home}/.claude/memory/m.md")),
            w("/tmp/scratch/x"),
            w("/private/tmp/y/z"),
            w("/var/folders/ab/cd/T/f"),
            w("/tmpfoo/keep.txt"),
        ],
    );
    e.scan();
    assert_eq!(e.outs(S, "folder"), vec!["/tmpfoo", "/work/p/src"]);
}

#[test]
fn live_entries_without_transcript_never_become_cards() {
    let mut e = Env::new();
    e.append(
        S,
        &[
            user(S, "2026-11-01T10:00:00Z", "hello"),
            user(S, "2026-11-01T10:00:01Z", "again"),
        ],
    );
    let pid = std::process::id();
    // the summarizer: sdk-cli, session id with no transcript
    std::fs::write(
        e.claude.join("sessions").join(format!("{pid}.json")),
        format!(r#"{{"pid":{pid},"sessionId":"{T}","cwd":"/work/p","status":"busy","entrypoint":"sdk-cli","updatedAt":5}}"#),
    )
    .unwrap();
    e.scan();
    assert!(e.db.session_by_id(T).unwrap().is_none());
    assert_eq!(e.db.all_sessions().unwrap().len(), 1);
    // a real live session gets its status
    std::fs::write(
        e.claude.join("sessions").join(format!("{pid}.json")),
        format!(r#"{{"pid":{pid},"sessionId":"{S}","cwd":"/work/p","status":"busy","entrypoint":"cli","updatedAt":7}}"#),
    )
    .unwrap();
    e.scan();
    assert_eq!(
        e.db.session_by_id(S)
            .unwrap()
            .unwrap()
            .live_status
            .as_deref(),
        Some("busy")
    );
    // history-only rows (no transcript) are not given a live status either
    std::fs::write(
        e.claude.join("sessions").join(format!("{pid}.json")),
        format!(r#"{{"pid":{pid},"sessionId":"{T}","cwd":"/work/p","status":"busy","entrypoint":"sdk-cli","updatedAt":8}}"#),
    )
    .unwrap();
    e.db.conn
        .execute(
            "INSERT INTO session(id,kind,transcript_state) VALUES(?1,'history_only','deleted')",
            [T],
        )
        .unwrap();
    e.scan();
    assert!(e
        .db
        .session_by_id(T)
        .unwrap()
        .unwrap()
        .live_status
        .is_none());
}
