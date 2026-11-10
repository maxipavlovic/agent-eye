//! Finding Codex's session files: which rollouts there are, which of them are sessions a person
//! has (not sub-agents, `codex exec` runs or Codex's own helper threads) and the names `/rename`
//! gave them.

use crate::agents::append_only::for_each_line;
use crate::agents::{Stamp, Transcript};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

/// Where Codex writes the rollout of every session: `YYYY/MM/DD/rollout-*.jsonl` below it.
pub const SESSIONS_DIR: &str = "sessions";
/// Where `codex archive` moves them, flat.
pub const ARCHIVED_DIR: &str = "archived_sessions";
const INDEX_FILE: &str = "session_index.jsonl";

/// Most rollouts whose first lines are remembered.
const HEADS_MAX: usize = 100_000;
/// The first lines of a rollout are read up to this many bytes.
const HEAD_BYTES: u64 = 4 << 20;
/// A legacy fork can start with its parent's `session_meta`; the right one is among the first few.
const HEAD_LINES: usize = 4;

static RE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^rollout-(\d{4}-\d{2})-\d{2}t\d{2}-\d{2}-\d{2}-([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})(?:_[0-9a-z-]+)?\.jsonl$")
        .expect("static regex")
});

/// A rollout file on disk.
#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    /// The thread id in the file name: the session id.
    pub thread_id: String,
    /// `YYYY-MM`, from the file name; groups the copies in eye's archive.
    pub month: String,
    pub stamp: Stamp,
}

/// `(month, thread id)` of a rollout file name such as
/// `rollout-2026-11-09T14-03-11-<thread id>.jsonl` (a reverted thread has a `_<n>` after the id).
pub fn parse_file_name(name: &str) -> Option<(String, String)> {
    let lower = name.to_ascii_lowercase();
    let c = RE_NAME.captures(&lower)?;
    Some((c[1].to_string(), c[2].to_string()))
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Found>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let (path, Ok(md)) = (e.path(), e.metadata()) else {
            continue;
        };
        if md.is_dir() {
            if depth > 0 {
                walk(&path, depth - 1, out);
            }
            continue;
        }
        let Some((month, thread_id)) = e.file_name().to_str().and_then(parse_file_name) else {
            continue;
        };
        out.push(Found {
            path,
            thread_id,
            month,
            stamp: Stamp::of_metadata(&md),
        });
    }
}

/// Every rollout under a Codex folder, one per thread: when a thread has several files (a reverted
/// thread, or one both live and archived) the newest wins. Sorted by path.
pub fn find_rollouts(dir: &Path) -> Vec<Found> {
    let mut all = vec![];
    walk(&dir.join(SESSIONS_DIR), 3, &mut all);
    walk(&dir.join(ARCHIVED_DIR), 0, &mut all);
    let mut best: HashMap<String, Found> = HashMap::new();
    for f in all {
        let newer = best
            .get(&f.thread_id)
            .is_none_or(|b| (f.stamp.mtime, &f.path) > (b.stamp.mtime, &b.path));
        if newer {
            best.insert(f.thread_id.clone(), f);
        }
    }
    let mut v: Vec<Found> = best.into_values().collect();
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v
}

/// What a rollout is for a person looking at their sessions.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    /// A session of its own: a card.
    Session,
    /// Work a session started: shown on the card of its parent, not as a card.
    Child {
        parent: String,
        title: String,
        detail: Option<String>,
    },
    /// Left out, like Codex's own picker leaves it out: `codex exec` runs, internal and
    /// guardian-review threads, memory consolidation.
    Hidden,
}

/// What the first lines of a rollout say about it.
#[derive(Debug, Clone)]
pub struct Head {
    pub cwd: Option<String>,
    pub role: Role,
}

static HEADS: LazyLock<Mutex<HashMap<PathBuf, Head>>> = LazyLock::new(Mutex::default);

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(v, |v, k| v.get(*k))?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A sub-agent thread names its parent in `parent_thread_id` or in `source.subagent.thread_spawn`.
fn child_of(meta: &Value) -> Option<Role> {
    let spawn = ["source", "subagent", "thread_spawn"];
    let at = |top: &str, inner: &str| {
        let mut p = spawn.to_vec();
        p.push(inner);
        str_at(meta, &[top]).or_else(|| str_at(meta, &p))
    };
    let parent = at("parent_thread_id", "parent_thread_id")?;
    let role = at("agent_role", "agent_role");
    let title = at("agent_nickname", "agent_nickname")
        .or(role)
        .unwrap_or("sub-agent");
    Some(Role::Child {
        parent: parent.to_ascii_lowercase(),
        title: title.to_string(),
        detail: role.map(String::from),
    })
}

/// Which rollouts are sessions: the sources Codex's own resume picker lists (`cli`, `vscode`, and
/// the `atlas` and `chatgpt` clients; no source means `vscode`), minus helper threads.
fn role_of(meta: &Value) -> Role {
    let thread_source = str_at(meta, &["thread_source"]);
    if matches!(
        thread_source,
        Some("guardian_review" | "memory_consolidation")
    ) {
        return Role::Hidden;
    }
    if thread_source == Some("subagent") {
        return child_of(meta).unwrap_or(Role::Hidden);
    }
    match meta.get("source") {
        None | Some(Value::Null) => Role::Session,
        Some(Value::String(s)) if matches!(s.as_str(), "cli" | "vscode") => Role::Session,
        Some(Value::Object(o)) if o.contains_key("subagent") => {
            child_of(meta).unwrap_or(Role::Hidden)
        }
        Some(Value::Object(_)) => match str_at(meta, &["source", "custom"]) {
            Some("atlas" | "chatgpt") => Role::Session,
            _ => Role::Hidden,
        },
        Some(_) => Role::Hidden,
    }
}

/// The head of the rollout of thread `thread_id`, from its `session_meta` line. None while the
/// first line is not written completely or when there is none.
fn read_head(path: &Path, thread_id: &str) -> Option<Head> {
    let mut r = BufReader::new(std::fs::File::open(path).ok()?.take(HEAD_BYTES));
    let mut first: Option<Value> = None;
    let mut buf = Vec::new();
    for _ in 0..HEAD_LINES {
        buf.clear();
        if r.read_until(b'\n', &mut buf).ok()? == 0 || buf.last() != Some(&b'\n') {
            break;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&buf) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let meta = v.get("payload").cloned().unwrap_or(Value::Null);
        // a legacy fork can carry its parent's meta first: the one with this thread's id counts
        if str_at(&meta, &["id"]).is_some_and(|i| i.eq_ignore_ascii_case(thread_id)) {
            first = Some(meta);
            break;
        }
        first.get_or_insert(meta);
    }
    let meta = first?;
    Some(Head {
        cwd: str_at(&meta, &["cwd"]).map(String::from),
        role: role_of(&meta),
    })
}

/// The head of a rollout. A session's `session_meta` never changes, so it is read once and kept.
pub fn head(path: &Path, thread_id: &str) -> Option<Head> {
    let mut heads = HEADS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = heads.get(path) {
        return Some(h.clone());
    }
    let h = read_head(path, thread_id)?;
    if heads.len() >= HEADS_MAX {
        heads.clear();
    }
    heads.insert(path.to_path_buf(), h.clone());
    Some(h)
}

/// The name `/rename` gave a session and when.
#[derive(Debug, Clone)]
pub struct Name {
    pub text: String,
}

/// Session names by thread id.
pub type Names = HashMap<String, Name>;

/// The names read from a `session_index.jsonl`, with the stamp of the file they came from.
type Cached = (Stamp, Arc<Names>);

static NAMES: LazyLock<Mutex<HashMap<PathBuf, Cached>>> = LazyLock::new(Mutex::default);

/// The names in `session_index.jsonl`: one line per rename, the last one of a session wins and an
/// empty name takes the name away. Read again only when the file changed.
pub fn names(dir: &Path) -> Arc<Names> {
    let path = dir.join(INDEX_FILE);
    let Some(stamp) = Stamp::of_file(&path) else {
        return Arc::default();
    };
    let mut cache = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, names)) = cache.get(&path).filter(|(s, _)| *s == stamp) {
        return names.clone();
    }
    let mut names = Names::new();
    let _ = for_each_line(&path, 0, |line| {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let Some(id) = str_at(&v, &["id"]).map(str::to_ascii_lowercase) else {
            return;
        };
        match str_at(&v, &["thread_name"]) {
            Some(text) => {
                names.insert(
                    id,
                    Name {
                        text: text.to_string(),
                    },
                );
            }
            None => {
                names.remove(&id);
            }
        }
    });
    let names = Arc::new(names);
    cache.insert(path, (stamp, names.clone()));
    names
}

/// A sub-agent of a session.
#[derive(Debug, Clone)]
pub struct Child {
    pub id: String,
    pub path: PathBuf,
    pub stamp: Stamp,
    pub title: String,
    pub detail: Option<String>,
}

/// A session found on disk.
#[derive(Debug, Clone)]
pub struct Visible {
    pub path: PathBuf,
    pub cwd: Option<String>,
}

/// What one look at the folder found besides the transcripts.
#[derive(Debug, Default)]
pub struct Index {
    pub sessions: HashMap<String, Visible>,
    /// Sub-agents by the thread id of their parent.
    pub children: HashMap<String, Vec<Child>>,
}

/// The sessions under a Codex folder, with the sub-agents and paths the rest of the adapter needs.
/// A named session takes the change time of `session_index.jsonl` when that is newer than its
/// rollout, so a rename (or an unnamed one) makes eye read it again and take the new title. The
/// file's own time is used, not the time written in the line, so it does not depend on clocks.
pub fn scan(dir: &Path) -> (Vec<Transcript>, Index) {
    let names = names(dir);
    let renamed_at = Stamp::of_file(&dir.join(INDEX_FILE)).map_or(0, |s| s.mtime);
    let mut out = vec![];
    let mut index = Index::default();
    for f in find_rollouts(dir) {
        let Some(head) = head(&f.path, &f.thread_id) else {
            continue;
        };
        match head.role {
            Role::Session => {
                let mut stamp = f.stamp;
                if names.contains_key(&f.thread_id) {
                    stamp.mtime = stamp.mtime.max(renamed_at);
                }
                index.sessions.insert(
                    f.thread_id.clone(),
                    Visible {
                        path: f.path.clone(),
                        cwd: head.cwd,
                    },
                );
                out.push(Transcript {
                    project_dir: f.month,
                    session_id: f.thread_id,
                    path: f.path,
                    stamp,
                });
            }
            Role::Child {
                parent,
                title,
                detail,
            } => index.children.entry(parent).or_default().push(Child {
                id: f.thread_id,
                path: f.path,
                stamp: f.stamp,
                title,
                detail,
            }),
            Role::Hidden => {}
        }
    }
    (out, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rollout_file_names_carry_the_month_and_the_thread() {
        let id = "019a6b2e-7c10-7a41-9d3e-5f0c2b7a1e44";
        assert_eq!(
            parse_file_name(&format!("rollout-2026-11-09T14-03-11-{id}.jsonl")),
            Some(("2026-11".into(), id.into()))
        );
        assert_eq!(
            parse_file_name(&format!("rollout-2026-11-09T14-03-11-{id}_2.jsonl")),
            Some(("2026-11".into(), id.into())),
            "a reverted thread"
        );
        for bad in [
            "rollout-2026-11-09T14-03-11.jsonl",
            &format!("rollout-2026-11-09T14-03-11-{id}.jsonl.zst"),
            &format!("{id}.jsonl"),
            "history.jsonl",
        ] {
            assert_eq!(parse_file_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_what_codexs_own_picker_lists_is_a_session() {
        let role = |meta: Value| role_of(&meta);
        for shown in [
            json!({"source": "cli", "thread_source": "user"}),
            json!({"source": "vscode"}),
            json!({"source": {"custom": "atlas"}}),
            json!({"source": {"custom": "chatgpt"}}),
            json!({"cwd": "/x"}),
        ] {
            assert_eq!(role(shown.clone()), Role::Session, "{shown}");
        }
        for hidden in [
            json!({"source": "exec"}),
            json!({"source": "mcp"}),
            json!({"source": {"custom": "other"}}),
            json!({"source": {"internal": "memory_consolidation"}}),
            json!({"source": "cli", "thread_source": "guardian_review"}),
            json!({"source": "cli", "thread_source": "memory_consolidation"}),
            json!({"source": {"subagent": "review"}}),
            json!({"source": {"subagent": "compact"}, "thread_source": "subagent"}),
            json!({"source": 5}),
        ] {
            assert_eq!(role(hidden.clone()), Role::Hidden, "{hidden}");
        }
    }

    #[test]
    fn a_sub_agent_names_its_parent_either_way() {
        let spawn = json!({"source": {"subagent": {"thread_spawn": {
            "parent_thread_id": "AAA", "depth": 1, "agent_nickname": "Hume", "agent_role": "explorer"}}},
            "thread_source": "subagent"});
        let want = Role::Child {
            parent: "aaa".into(),
            title: "Hume".into(),
            detail: Some("explorer".into()),
        };
        assert_eq!(role_of(&spawn), want);
        let flat = json!({"source": {"subagent": "x"}, "parent_thread_id": "aaa",
            "agent_nickname": "Hume", "agent_role": "explorer"});
        assert_eq!(role_of(&flat), want);
        let bare = json!({"parent_thread_id": "aaa", "thread_source": "subagent"});
        assert_eq!(
            role_of(&bare),
            Role::Child {
                parent: "aaa".into(),
                title: "sub-agent".into(),
                detail: None
            }
        );
    }

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn the_newest_file_of_a_thread_wins_and_other_files_are_ignored() {
        let t = tempfile::tempdir().unwrap();
        let id = "019a6b2e-7c10-7a41-9d3e-5f0c2b7a1e44";
        let live = t.path().join(format!(
            "sessions/2026/11/09/rollout-2026-11-09T14-03-11-{id}.jsonl"
        ));
        let archived = t.path().join(format!(
            "archived_sessions/rollout-2026-11-09T14-03-11-{id}.jsonl"
        ));
        write(&live, "{}\n");
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&archived, "{}\n");
        write(&t.path().join("sessions/2026/11/09/notes.txt"), "x");
        write(&t.path().join("history.jsonl"), "{}\n");
        let found = find_rollouts(t.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, archived);
        assert_eq!(found[0].month, "2026-11");
    }

    #[test]
    fn the_head_is_the_meta_of_this_thread_and_waits_for_a_whole_line() {
        let t = tempfile::tempdir().unwrap();
        let (id, parent) = (
            "aaaaaaaa-0000-7000-8000-000000000001",
            "bbbbbbbb-0000-7000-8000-000000000002",
        );
        let meta = |id: &str, cwd: &str| {
            format!(
                "{}\n",
                json!({"type": "session_meta", "payload": {"id": id, "cwd": cwd, "source": "cli"}})
            )
        };
        let p = t.path().join("a.jsonl");
        write(
            &p,
            &format!("{}{}", meta(parent, "/parent"), meta(id, "/mine")),
        );
        assert_eq!(read_head(&p, id).unwrap().cwd.as_deref(), Some("/mine"));
        write(&p, &meta(parent, "/parent"));
        assert_eq!(
            read_head(&p, id).unwrap().cwd.as_deref(),
            Some("/parent"),
            "the first meta when none has this id"
        );
        write(&p, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"aaaa");
        assert!(read_head(&p, id).is_none(), "half a line is not a head");
        write(&p, "not json\n{\"type\":\"turn_context\"}\n");
        assert!(read_head(&p, id).is_none());
    }

    #[test]
    fn a_rename_is_the_last_line_for_the_session_and_an_empty_name_removes_it() {
        let t = tempfile::tempdir().unwrap();
        write(
            &t.path().join(INDEX_FILE),
            &[
                r#"{"id":"AAA","thread_name":"First","updated_at":"2026-11-08T09:10:00.000Z"}"#,
                r#"{"id":"aaa","thread_name":"Second","updated_at":"2026-11-08T09:20:00.000Z"}"#,
                r#"{"id":"bbb","thread_name":"Gone","updated_at":"2026-11-08T09:10:00.000Z"}"#,
                r#"{"id":"bbb","thread_name":"  ","updated_at":"2026-11-08T09:30:00.000Z"}"#,
                "not json",
            ]
            .join("\n"),
        );
        let n = names(t.path());
        assert_eq!(n["aaa"].text, "Second");
        assert!(!n.contains_key("bbb"));
        assert!(names(&t.path().join("nowhere")).is_empty());
    }
}
