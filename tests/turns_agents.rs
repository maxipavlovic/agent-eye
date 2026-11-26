//! Turn events of Codex, agy and OpenCode from their files to the database: the normal read, a
//! session that grows (or is rewritten) in the middle of a turn, and the backfill of a session an
//! older eye already read. The Claude side is in `turns.rs`.

use agent_eye::agents::Agents;
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::turns::backfill;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

const CODEX: &str = include_str!("fixtures/codex-turns.jsonl");
const AGY: &str = include_str!("fixtures/agy-turns.jsonl");
const OPENCODE: &str = include_str!("fixtures/opencode-turns.json");

const CODEX_ID: &str = "019a7000-0000-7000-8000-00000000c0de";
const AGY_ID: &str = "a7777777-7777-4777-8777-777777777777";
const OPENCODE_ID: &str = "ses_turns1";

struct Env {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl Env {
    /// An empty folder of `agent`, the only agent that is followed.
    fn new(agent: &str) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(agent);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::parse(&format!(
            "[agents]\nenabled = [\"{agent}\"]\n[agents.{agent}]\ndir = \"{}\"\n",
            dir.display()
        ))
        .unwrap();
        // the test's sessions live in temp folders, which eye leaves out by default
        cfg.exclude.temp = false;
        let db = Db::open(&paths.db_path()).unwrap();
        Env {
            _tmp: tmp,
            dir,
            paths,
            cfg,
            db,
        }
    }

    fn agents(&self) -> Agents {
        agent_eye::agents::enabled(&self.cfg)
    }

    fn scan(&mut self) {
        scan(&mut self.db, &self.cfg, false).unwrap();
    }

    fn forget_turns(&self) {
        self.db.conn.execute("DELETE FROM turn", []).unwrap();
        self.db.conn.execute("DELETE FROM turn_state", []).unwrap();
    }

    fn backfill(&mut self) {
        let agents = self.agents();
        backfill::run_all(&mut self.db, &self.paths, &agents).unwrap();
    }

    /// How many events of each kind the database has.
    fn counts(&self) -> Vec<(String, i64)> {
        self.db
            .conn
            .prepare("SELECT kind, count(*) FROM turn GROUP BY kind ORDER BY kind")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
}

fn kinds(pairs: &[(&str, i64)]) -> Vec<(String, i64)> {
    pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn codex_rollout(e: &Env) -> PathBuf {
    e.dir.join(format!(
        "sessions/2026/11/03/rollout-2026-11-03T09-00-00-{CODEX_ID}.jsonl"
    ))
}

/// The log of the agy conversation, and the files that make agy list it (a database, a folder in
/// `history.jsonl`).
fn agy_log(e: &Env) -> PathBuf {
    write(&e.dir.join(format!("conversations/{AGY_ID}.db")), "");
    write(
        &e.dir.join("history.jsonl"),
        &format!("{{\"display\":\"x\",\"timestamp\":1,\"workspace\":\"/tmp/demo\",\"conversationId\":\"{AGY_ID}\"}}\n"),
    );
    e.dir.join(format!(
        "brain/{AGY_ID}/.system_generated/logs/transcript_full.jsonl"
    ))
}

fn codex_counts() -> Vec<(String, i64)> {
    kinds(&[
        ("compact", 1),
        ("end", 2),
        ("interrupt", 1),
        ("prompt", 2),
        ("refusal", 1),
        ("think", 1),
        ("tool", 3),
        ("tool_error", 1),
    ])
}

fn agy_counts() -> Vec<(String, i64)> {
    kinds(&[
        ("end", 2),
        ("prompt", 2),
        ("refusal", 1),
        ("think", 1),
        ("tool", 3),
        ("tool_error", 2),
    ])
}

fn halves(text: &str, at: usize) -> (String, String) {
    let lines: Vec<&str> = text.lines().collect();
    let (a, b) = lines.split_at(at);
    (a.join("\n") + "\n", b.join("\n") + "\n")
}

#[test]
fn codex_turns_come_from_the_read_a_growing_rollout_and_the_backfill() {
    let mut e = Env::new("codex");
    let path = codex_rollout(&e);
    // the first read stops inside the second turn, which the rest of the file then finishes
    let (head, tail) = halves(CODEX, 12);
    write(&path, &head);
    e.scan();
    append(&path, &tail);
    e.scan();
    e.scan();
    assert_eq!(e.counts(), codex_counts());
    e.forget_turns();
    e.backfill();
    assert_eq!(e.counts(), codex_counts());
}

#[test]
fn a_rewritten_codex_rollout_replaces_its_turns() {
    let mut e = Env::new("codex");
    let path = codex_rollout(&e);
    // a third prompt, which the shorter file that takes its place does not have
    let extra = r#"{"timestamp":"2026-11-03T09:10:00.000Z","ordinal":99,"type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","id":"u3","content":[{"type":"text","text":"one more"}]}}}"#;
    write(&path, &format!("{CODEX}{extra}\n"));
    e.scan();
    assert_eq!(
        e.counts()
            .iter()
            .find(|(k, _)| k == "prompt")
            .map(|(_, n)| *n),
        Some(3)
    );
    write(&path, CODEX);
    e.scan();
    assert_eq!(e.counts(), codex_counts());
}

#[test]
fn agy_turns_come_from_the_read_a_growing_log_and_the_backfill() {
    let mut e = Env::new("agy");
    let log = agy_log(&e);
    let (head, tail) = halves(AGY, 10);
    write(&log, &head);
    e.scan();
    append(&log, &tail);
    e.scan();
    e.scan();
    assert_eq!(e.counts(), agy_counts());
    e.forget_turns();
    e.backfill();
    assert_eq!(e.counts(), agy_counts());
}

#[test]
fn a_rewritten_agy_log_replaces_its_turns() {
    let mut e = Env::new("agy");
    let log = agy_log(&e);
    // a third prompt, which the shorter file that takes its place does not have
    let extra = r#"{"step_index":99,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-04T08:10:00Z","content":"<USER_REQUEST>\none more\n</USER_REQUEST>"}"#;
    write(&log, &format!("{AGY}{extra}\n"));
    e.scan();
    assert_eq!(
        e.counts()
            .iter()
            .find(|(k, _)| k == "prompt")
            .map(|(_, n)| *n),
        Some(3)
    );
    write(&log, AGY);
    e.scan();
    assert_eq!(e.counts(), agy_counts());
}

/// The OpenCode session of the fixture as files of its older layout; only the first `upto`
/// messages are written.
fn opencode_files(e: &Env, upto: usize) {
    let f: Value = serde_json::from_str(OPENCODE).unwrap();
    let root = e.dir.join("storage");
    let session = serde_json::json!({"id": OPENCODE_ID, "directory": "/tmp/demo", "title": "Turns",
        "time": {"created": 1_793_696_400_000_i64, "updated": 1_793_696_460_000_i64 + upto as i64}});
    write(
        &root.join(format!("session/prj1/{OPENCODE_ID}.json")),
        &session.to_string(),
    );
    let mut n = 0;
    for m in &f["v1"]["messages"].as_array().unwrap()[..upto] {
        let id = m[0].as_str().unwrap();
        let mut data = m[2].clone();
        data["id"] = id.into();
        write(
            &root.join(format!("message/{OPENCODE_ID}/{id}.json")),
            &data.to_string(),
        );
        for p in f["v1"]["parts"][id].as_array().into_iter().flatten() {
            n += 1;
            let mut part = p.clone();
            part["id"] = format!("prt_{n:03}").into();
            write(
                &root.join(format!("part/{id}/prt_{n:03}.json")),
                &part.to_string(),
            );
        }
    }
}

#[test]
fn opencode_turns_come_from_the_read_a_session_that_grows_and_the_backfill() {
    let mut e = Env::new("opencode");
    // the first read stops after the aborted message; the session is read whole again when it grows
    opencode_files(&e, 3);
    e.scan();
    let first = e.counts();
    assert_eq!(
        first.iter().find(|(k, _)| k == "end").map(|(_, n)| *n),
        Some(1)
    );
    opencode_files(&e, 6);
    e.scan();
    e.scan();
    assert_eq!(e.counts(), codex_counts());
    e.forget_turns();
    e.backfill();
    assert_eq!(e.counts(), codex_counts());
}
