//! The OpenCode adapter, built from the source (sst/opencode v2.3.2) and not tried on a real install:
//! synthetic sessions that follow its structs, in the three layouts found in the wild (the
//! `session_v2` tables, the older `session` tables and the JSON files of v1.0 and v1.1). The
//! databases are made by this test, since `*.db` files are not committed.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;

use agent_eye::agents::{self, opencode::OpenCode, Agent, Agents};
use agent_eye::archive::{self, Archived};
use agent_eye::config::Config;
use agent_eye::conversation;
use agent_eye::db::Db;
use agent_eye::model::Session;
use agent_eye::paths::Paths;
use agent_eye::scan::{scan, ScanStats};
use agent_eye::search;
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};

/// A `session_v2` session with a tool call and tokens, also present in the older tables.
const V2: &str = "ses_v2koala";
/// A session of the older tables only.
const V1: &str = "ses_v1old";
/// A sub-agent session of V2: hidden.
const CHILD: &str = "ses_child";
/// A session of the JSON file layout.
const JSON: &str = "ses_json1";

struct World {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

fn sql(conn: &Connection, q: &str, p: impl rusqlite::Params) {
    conn.execute(q, p).unwrap();
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("opencode");
        fixtures::copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/opencode-home"),
            &dir,
        );
        let c = Connection::open(dir.join("opencode.db")).unwrap();
        c.execute_batch(
            "CREATE TABLE session_v2 (id text PRIMARY KEY, project_id text, directory text NOT NULL, parent_id text, title text, time_created integer, time_updated integer);
             CREATE TABLE session_message (id text PRIMARY KEY, session_id text NOT NULL, type text NOT NULL, seq integer NOT NULL, time_created integer, time_updated integer, data text NOT NULL);
             CREATE TABLE session_part (message_id text NOT NULL, session_id text NOT NULL, ordinal integer NOT NULL, type text NOT NULL, data text NOT NULL, PRIMARY KEY (message_id, ordinal));
             CREATE TABLE session (id text PRIMARY KEY, project_id text, directory text NOT NULL, parent_id text, title text, time_created integer, time_updated integer);
             CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer, data text NOT NULL);
             CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer, data text NOT NULL);",
        )
        .unwrap();
        let ins = "INSERT INTO session_v2 VALUES(?1,'p',?2,?3,?4,?5,?6)";
        sql(
            &c,
            ins,
            (
                V2,
                "/work/koala hub",
                None::<String>,
                "Koala census",
                1_793_100_000_000i64,
                1_793_100_010_000i64,
            ),
        );
        sql(
            &c,
            ins,
            (
                CHILD,
                "/work/koala hub",
                Some(V2),
                "Sub task",
                1_793_100_001_000i64,
                1_793_100_002_000i64,
            ),
        );
        sql(
            &c,
            "INSERT INTO session VALUES(?1,'p',?2,NULL,?3,?4,?5)",
            (V2, "/elsewhere", "Older copy of the same", 1i64, 2i64),
        );
        sql(
            &c,
            "INSERT INTO session VALUES(?1,'p',?2,NULL,?3,?4,?5)",
            (
                V1,
                "/work/old",
                "Old session",
                1_792_000_000_000i64,
                1_792_000_005_000i64,
            ),
        );
        let msg = |id: &str, kind: &str, seq: i64, t: i64, data: serde_json::Value| {
            sql(
                &c,
                "INSERT INTO session_message VALUES(?1,?2,?3,?4,?5,?5,?6)",
                (id, V2, kind, seq, t, data.to_string()),
            );
        };
        msg(
            "msg_1",
            "user",
            1,
            1_793_100_000_000,
            json!({"text": "List the koala enclosures", "files": []}),
        );
        msg(
            "msg_2",
            "assistant",
            2,
            1_793_100_005_000,
            json!({"agent": "build", "model": {"id": "koala-2", "providerID": "acme"}, "content": [],
                "tokens": {"input": 200, "output": 50, "reasoning": 5, "cache": {"read": 800, "write": 0}}}),
        );
        sql(&c, "INSERT INTO session_message VALUES('msg_3',?1,'system',3,1,1,'{\"text\":\"injected\"}')", [V2]);
        for (o, kind, data) in [
            (
                0,
                "tool",
                json!({"type": "tool", "id": "t1", "name": "read", "state": {"status": "completed", "input": {"filePath": "/work/koala hub/map.txt"}}}),
            ),
            (
                1,
                "text",
                json!({"type": "text", "text": "The enclosure is by the lake."}),
            ),
        ] {
            sql(
                &c,
                "INSERT INTO session_part VALUES('msg_2',?1,?2,?3,?4)",
                (V2, o, kind, data.to_string()),
            );
        }
        sql(
            &c,
            "INSERT INTO message VALUES('m_a',?1,1792000000000,?2)",
            (
                V1,
                json!({"role": "user", "time": {"created": 1_792_000_000_000i64}}).to_string(),
            ),
        );
        sql(
            &c,
            "INSERT INTO message VALUES('m_b',?1,1792000002000,?2)",
            (V1, json!({"role": "assistant", "modelID": "old-1", "time": {"created": 1_792_000_002_000i64},
                "tokens": {"input": 10, "output": 4, "reasoning": 0, "cache": {"read": 0, "write": 2}}}).to_string()),
        );
        sql(
            &c,
            "INSERT INTO part VALUES('p1','m_a',?1,1,?2)",
            (
                V1,
                json!({"type": "text", "text": "Ping the old server"}).to_string(),
            ),
        );
        sql(
            &c,
            "INSERT INTO part VALUES('p2','m_b',?1,2,?2)",
            (
                V1,
                json!({"type": "text", "text": "Pong, see APP-77"}).to_string(),
            ),
        );
        drop(c);
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::parse(&format!(
            "[agents]\nenabled = [\"opencode\"]\n[agents.opencode]\ndir = \"{}\"\n[outputs]\njira_projects = [\"APP\"]\n",
            dir.display()
        ))
        .unwrap();
        cfg.summary.enabled = true;
        let db = Db::open(&paths.db_path()).unwrap();
        World {
            _tmp: tmp,
            dir,
            paths,
            cfg,
            db,
        }
    }
    fn scan(&mut self) -> ScanStats {
        scan(&mut self.db, &self.cfg, false).unwrap()
    }
    fn session(&self, id: &str) -> Session {
        self.db.session_by_id(id).unwrap().unwrap()
    }
    fn agents(&self) -> Agents {
        agents::enabled(&self.cfg)
    }
}

fn hits(db: &Db, word: &str) -> Vec<String> {
    search::find(db, &[word.to_string()], 10, false, 0)
        .unwrap()
        .into_iter()
        .map(|h| h.session.id)
        .collect()
}

#[test]
fn the_scan_reads_all_three_layouts_and_leaves_sub_agents_out() {
    let mut w = World::new();
    let st = w.scan();
    assert_eq!(
        (st.files_seen, st.files_parsed),
        (3, 3),
        "V2 (once), V1 and the JSON one"
    );
    for id in [V2, V1, JSON] {
        assert_eq!(w.session(id).agent, "opencode", "{id}");
    }
    assert!(w.db.session_by_id(CHILD).unwrap().is_none());
    let a = w.session(V2);
    assert_eq!(
        a.title.as_deref(),
        Some("Koala census"),
        "the v2 row wins over the older copy"
    );
    assert_eq!(a.cwd.as_deref(), Some("/work/koala hub"));
    assert_eq!((a.prompt_count, a.reply_count), (1, 1));
    assert_eq!(a.first_prompt.as_deref(), Some("List the koala enclosures"));
    let j = w.session(JSON);
    assert_eq!(j.cwd.as_deref(), Some("/work/legacy app"));
    assert_eq!(j.title.as_deref(), Some("Legacy wombat survey"));
    assert_eq!(
        j.last_assistant_text.as_deref(),
        Some("There are 11 wombats.")
    );
    assert_eq!(w.session(V1).project_dir.as_deref(), Some("-work-old"));
    assert_eq!(w.scan().files_parsed, 0, "a second scan reads nothing");
    assert!(hits(&w.db, "koala").contains(&V2.to_string()));
    assert!(hits(&w.db, "wombats").contains(&JSON.to_string()));
    assert!(hits(&w.db, "injected").is_empty());
}

#[test]
fn tokens_and_models_come_from_the_messages() {
    let mut w = World::new();
    w.scan();
    let t = w.session(V2).tokens.unwrap();
    assert_eq!(
        (t.input, t.output, t.cache_read, t.cache_write),
        (200, 55, 800, 0),
        "reasoning counts as output"
    );
    let t = w.session(JSON).tokens.unwrap();
    assert_eq!((t.input, t.output, t.cache_read), (120, 40, 400));
    assert_eq!(w.session(V1).tokens.unwrap().cache_write, 2);
    let agents = w.agents();
    let agent = agents.get("opencode").unwrap();
    let path = |id: &str| {
        w.db.session_by_id(id)
            .unwrap()
            .unwrap()
            .transcript_path
            .unwrap()
    };
    let m = agent.usage_msgs(Path::new(&path(V2))).unwrap();
    assert_eq!((m.len(), m[0].model.as_str()), (1, "koala-2"));
    assert_eq!(
        agent.usage_msgs(Path::new(&path(JSON))).unwrap()[0].model,
        "wombat-1"
    );
}

#[test]
fn a_database_session_is_read_though_it_is_no_file() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    let agent = agents.get("opencode").unwrap();
    let s = w.session(V2);
    assert!(s
        .transcript_path
        .as_deref()
        .unwrap()
        .ends_with("opencode.db#ses_v2koala"));
    let msgs = conversation::read(&s, &w.paths, agent.as_ref()).unwrap();
    let shape: Vec<(&str, u32)> = msgs.iter().map(|m| (m.role, m.tools)).collect();
    assert_eq!(shape, [("user", 0), ("assistant", 1)]);
    let tail = agent
        .conversation_tail(Path::new(s.transcript_path.as_deref().unwrap()))
        .unwrap();
    assert!(tail
        .iter()
        .any(|t| matches!(t, agents::TailItem::Tool(l) if l == "read /work/koala hub/map.txt")));
    let j = w.session(JSON);
    let tail = agent
        .conversation_tail(Path::new(j.transcript_path.as_deref().unwrap()))
        .unwrap();
    assert!(tail
        .iter()
        .any(|t| matches!(t, agents::TailItem::Tool(l) if l == "$ wc -l wombats.csv")));
    // restoring is never offered, and a missing database row is not "there"
    assert!(!agent.owns_transcript(Path::new(s.transcript_path.as_deref().unwrap())));
    assert!(!agent.transcript_exists(&w.dir.join("opencode.db#ses_nope")));
}

#[test]
fn a_new_message_changes_the_stamp_and_is_read() {
    let mut w = World::new();
    w.scan();
    let c = Connection::open(w.dir.join("opencode.db")).unwrap();
    sql(
        &c,
        "INSERT INTO session_message VALUES('msg_4',?1,'user',4,1793100020000,1793100020000,?2)",
        (V2, json!({"text": "And the wallabies?"}).to_string()),
    );
    sql(
        &c,
        "UPDATE session_v2 SET time_updated=1793100020000 WHERE id=?1",
        [V2],
    );
    drop(c);
    assert_eq!(w.scan().files_parsed, 1);
    let a = w.session(V2);
    assert_eq!(a.prompt_count, 2, "read whole again, nothing counted twice");
    assert_eq!(a.tokens.unwrap().input, 200);
    assert!(hits(&w.db, "wallabies").contains(&V2.to_string()));
}

#[test]
fn the_archive_is_an_export_that_reads_after_the_database_is_gone() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    for id in [V2, JSON] {
        assert!(
            matches!(
                archive::archive_session(&w.db, &w.paths, &agents, id).unwrap(),
                Archived::Copied(_)
            ),
            "{id}"
        );
        assert!(
            matches!(
                archive::archive_session(&w.db, &w.paths, &agents, id).unwrap(),
                Archived::Unchanged
            ),
            "{id}"
        );
    }
    std::fs::remove_file(w.dir.join("opencode.db")).unwrap();
    std::fs::remove_dir_all(w.dir.join("storage")).unwrap();
    let agent = agents.get("opencode").unwrap();
    let a = conversation::read(&w.session(V2), &w.paths, agent.as_ref()).unwrap();
    assert_eq!(a.len(), 2);
    let j = conversation::read(&w.session(JSON), &w.paths, agent.as_ref()).unwrap();
    assert_eq!((j.len(), j[1].tools), (2, 1));
    assert!(
        archive::restore(&w.db, &w.paths, &agents, V2).is_err(),
        "nothing is written back"
    );
}

#[test]
fn resume_and_start_lines() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    assert_eq!(
        agents.resume_line(&w.session(V2)).unwrap(),
        "cd '/work/koala hub' && opencode --session ses_v2koala"
    );
    assert!(agents.fork_line(&w.session(V2)).is_none());
    let oc = OpenCode::new(w.dir.clone());
    let req = |prompt, background| agents::StartRequest {
        dir: "/x",
        title: "t",
        prompt,
        background,
        session_id: None,
    };
    let p = oc.start(&req(Some("do it"), false)).unwrap();
    assert_eq!(
        p.invocation.shell_line("/x"),
        "cd '/x' && opencode --prompt 'do it'"
    );
    assert!(p.session_id.is_none());
    assert_eq!(
        oc.start(&req(None, false))
            .unwrap()
            .invocation
            .shell_line("/x"),
        "cd '/x' && opencode"
    );
    assert!(oc.start(&req(Some("go"), true)).is_err());
}

#[test]
fn detection_and_an_empty_folder() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        !OpenCode::new(tmp.path().join("nope"))
            .with_executable("/nonexistent/opencode")
            .detect()
            .installed
    );
    let here = OpenCode::new(tmp.path().to_path_buf()).with_executable("/nonexistent/opencode");
    assert!(here.detect().installed);
    assert!(here.transcripts().is_empty());
}
