//! Agents keep their sessions in different shapes: one growing file (Claude Code and the first fake
//! agent), or a folder with a SQLite database in it (the second fake agent). The core knows none of
//! this. Each adapter says how to go on reading its sessions from a marker, how to tell that a
//! session was rewritten, and how to copy one to the archive and put it back.

#[allow(dead_code)]
#[path = "support/fake_agent.rs"]
mod fake_agent;
#[path = "support/folder_agent.rs"]
mod folder_agent;

use agent_eye::agents::{self, Agents};
use agent_eye::archive::{self, Archived};
use agent_eye::config::Config;
use agent_eye::conversation;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::search;
use agent_eye::server::{new_state, router};
use agent_eye::summarize;
use axum::body::Body;
use axum::http::Request;
use axum::Router;
use folder_agent::agent_writes;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
const CLAUDE_ID: &str = "aaaaaaaa-0000-4000-8000-0000000000c1";
const FAKE_ID: &str = "fake-session-0001";
const DB_ID: &str = "db-session-0001";
const DB_PROJECT: &str = "work-db";

const DB_ROWS: [(&str, i64, &str); 3] = [
    ("u", 1793527200000, "Find the quokka enclosure"),
    ("a", 1793527205000, "The quokka enclosure is by the lake."),
    ("u", 1793527260000, "Thanks, close it"),
];

struct World {
    _tmp: tempfile::TempDir,
    claude: PathBuf,
    fake: PathBuf,
    dbagent: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl World {
    fn agents(&self) -> Agents {
        agents::enabled(&self.cfg)
    }
    fn scan(&mut self) -> agent_eye::scan::ScanStats {
        scan(&mut self.db, &self.cfg, false).unwrap()
    }
    fn db_folder(&self) -> PathBuf {
        agent_writes::path(&self.dbagent, DB_PROJECT, DB_ID)
    }
    fn claude_file(&self) -> PathBuf {
        self.claude
            .join(format!("projects/-work-claude/{CLAUDE_ID}.jsonl"))
    }
    fn fake_file(&self) -> PathBuf {
        self.fake.join(format!("logs/work-fake/{FAKE_ID}.log"))
    }
    fn session(&self, id: &str) -> agent_eye::model::Session {
        self.db.session_by_id(id).unwrap().unwrap()
    }
    fn app(&self) -> Router {
        router(new_state(
            self.paths.clone(),
            self.cfg.clone(),
            TOKEN.into(),
            7420,
        ))
    }
}

/// A Claude Code session, one in the first fake agent's format and one in the second's, in temp
/// folders. `enabled` says which agents the config turns on.
fn world(enabled: &str) -> World {
    fake_agent::register();
    folder_agent::register();
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let fake = tmp.path().join("fake");
    let dbagent = tmp.path().join("dbagent");
    let proj = claude.join("projects/-work-claude");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(format!("{CLAUDE_ID}.jsonl")),
        format!(
            "{}\n{}\n{}\n",
            json!({"type":"user","timestamp":"2026-11-01T10:00:00Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"content":"Investigate the platypus report"}}),
            json!({"type":"assistant","timestamp":"2026-11-01T10:00:05Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"id":"m1","content":[{"type":"text","text":"The platypus report is fine."}]}}),
            json!({"type":"user","timestamp":"2026-11-01T10:01:00Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"content":"Thanks, close it"}}),
        ),
    )
    .unwrap();
    let logs = fake.join("logs/work-fake");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join(format!("{FAKE_ID}.log")),
        "cwd|/work/fake\nu|1793527200000|Find the zebra crossing\na|1793527205000|The zebra crossing is on Main Street.\nu|1793527260000|Thanks, close it\n",
    )
    .unwrap();
    agent_writes::start(&dbagent, DB_PROJECT, DB_ID, "/work/db", &DB_ROWS);
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let mut cfg = Config::parse(&format!(
        "[agents]\nenabled = {enabled}\n[agents.fake]\ndir = \"{}\"\n[agents.fakedb]\ndir = \"{}\"\n",
        fake.display(),
        dbagent.display()
    ))
    .unwrap();
    cfg.override_agent_dir("claude", claude.clone());
    let db = Db::open(&paths.db_path()).unwrap();
    World {
        _tmp: tmp,
        claude,
        fake,
        dbagent,
        paths,
        cfg,
        db,
    }
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn hits(db: &Db, word: &str) -> usize {
    search::find(db, &[word.to_string()], 10, false, 0)
        .unwrap()
        .len()
}

fn texts(m: &[conversation::Message]) -> Vec<String> {
    m.iter().map(|m| m.text.clone()).collect()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

async fn get(app: &Router, uri: &str) -> Value {
    let req = Request::builder()
        .uri(uri)
        .header("host", "127.0.0.1:7420")
        .header("x-eye-token", TOKEN)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success(), "{uri}: {}", resp.status());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn a_folder_session_is_read_on_from_its_marker_and_again_when_rewritten() {
    let mut w = world(r#"["fakedb"]"#);
    // the first scan reads the three rows
    let st = w.scan();
    assert_eq!((st.files_seen, st.files_parsed, st.bytes_read), (1, 1, 3));
    let s = w.session(DB_ID);
    assert_eq!(s.agent, folder_agent::ID);
    assert_eq!(s.project_dir.as_deref(), Some(DB_PROJECT));
    assert_eq!(s.cwd.as_deref(), Some("/work/db"));
    assert_eq!(s.transcript_state.as_deref(), Some("present"));
    assert_eq!(
        s.transcript_path.as_deref(),
        Some(w.db_folder().to_str().unwrap())
    );
    assert_eq!((s.prompt_count, s.reply_count), (2, 1));
    assert_eq!(s.title.as_deref(), Some("Find the quokka enclosure"));
    let marker = |w: &World| {
        w.db.cursor_get(w.db_folder().to_str().unwrap())
            .unwrap()
            .unwrap()
            .marker
    };
    assert_eq!(marker(&w), "1:3");
    assert_eq!(hits(&w.db, "quokka"), 1);

    // nothing changed: the stamp says so and the database is not opened
    assert_eq!(w.scan().files_parsed, 0);

    // two rows added: only those two are read, and they come on top of what is stored
    agent_writes::add(
        &w.dbagent,
        DB_PROJECT,
        DB_ID,
        &[
            ("u", 1793527300000, "And the wallaby paddock?"),
            ("a", 1793527305000, "The wallaby paddock is next door."),
        ],
    );
    let st = w.scan();
    assert_eq!((st.files_parsed, st.bytes_read), (1, 2));
    let s = w.session(DB_ID);
    assert_eq!((s.prompt_count, s.reply_count), (3, 2));
    assert_eq!(
        s.last_assistant_text.as_deref(),
        Some("The wallaby paddock is next door.")
    );
    assert_eq!(marker(&w), "1:5");
    assert_eq!((hits(&w.db, "quokka"), hits(&w.db, "wallaby")), (1, 1));

    // the agent writes the whole session again: everything is read, and it replaces what was stored
    agent_writes::rewrite(
        &w.dbagent,
        DB_PROJECT,
        DB_ID,
        &[
            ("u", 1793528000000, "Start over: count the emus"),
            ("a", 1793528005000, "Three emus."),
        ],
    );
    let st = w.scan();
    assert_eq!((st.files_parsed, st.bytes_read), (1, 2));
    let s = w.session(DB_ID);
    assert_eq!((s.prompt_count, s.reply_count), (1, 1), "no duplicates");
    assert_eq!(s.title.as_deref(), Some("Start over: count the emus"));
    assert_eq!(marker(&w), "2:2");
    assert_eq!(
        (
            hits(&w.db, "quokka"),
            hits(&w.db, "wallaby"),
            hits(&w.db, "emus")
        ),
        (0, 0, 1),
        "the old text is gone from search"
    );
    let n: i64 =
        w.db.conn
            .query_row("SELECT count(*) FROM session_fts", [], |r| r.get(0))
            .unwrap();
    assert_eq!(n, 1);
    // and from here on it goes on again from the new marker
    agent_writes::add(
        &w.dbagent,
        DB_PROJECT,
        DB_ID,
        &[("u", 1793528100000, "Thanks")],
    );
    let st = w.scan();
    assert_eq!((st.files_parsed, st.bytes_read), (1, 1));
    assert_eq!(w.session(DB_ID).prompt_count, 2);
}

#[test]
fn a_folder_session_is_read_by_search_conversation_and_summary_tail() {
    let mut w = world(r#"["fakedb"]"#);
    w.scan();
    let s = w.session(DB_ID);
    let agents = w.agents();
    let agent = agents.of_session(&s).unwrap();
    let all = conversation::read(&s, &w.paths, agent.as_ref()).unwrap();
    assert_eq!(
        texts(&all),
        [
            "Find the quokka enclosure",
            "The quokka enclosure is by the lake.",
            "Thanks, close it"
        ]
    );
    assert_eq!((all[0].role, all[1].role), ("user", "assistant"));
    let ex = summarize::build_excerpt(&w.db, agent.as_ref(), &s).unwrap();
    assert!(
        ex.text.contains("[user] Find the quokka enclosure"),
        "{}",
        ex.text
    );
    assert!(
        ex.text
            .contains("[assistant] The quokka enclosure is by the lake."),
        "{}",
        ex.text
    );
    // the board's search and conversation routes go through the same adapter
    let rt = tokio::runtime::Runtime::new().unwrap();
    let app = w.app();
    let found = rt.block_on(get(&app, "/api/search?q=quokka"));
    assert_eq!(found["results"][0]["card"]["id"], DB_ID);
    assert_eq!(found["results"][0]["card"]["agent_name"], "Dbbot");
    let talk = rt.block_on(get(&app, &format!("/api/cards/{DB_ID}/conversation")));
    assert_eq!(talk["messages"].as_array().unwrap().len(), 3);
    assert_eq!(talk["messages"][2]["text"], "Thanks, close it");
}

#[test]
fn a_folder_session_is_archived_read_from_the_copy_and_put_back() {
    let mut w = world(r#"["fakedb"]"#);
    w.scan();
    let agents = w.agents();
    let agent = agents.get(folder_agent::ID).unwrap().clone();
    let folder = w.db_folder();

    // the copy is a folder of compressed files, next to this agent's others and not in Claude's place
    let Archived::Copied(bytes) =
        archive::archive_session(&w.db, &w.paths, &agents, DB_ID).unwrap()
    else {
        panic!("the session was not copied");
    };
    assert!(bytes > 0);
    let by_project = w.paths.home.join("archive/@fakedb").join(DB_PROJECT);
    assert_eq!(names(&by_project), [DB_ID], "no half-made copy is left");
    assert_eq!(
        names(&by_project.join(DB_ID)),
        ["meta.json.zst", "session.db.zst"]
    );
    assert_eq!(
        w.session(DB_ID).archive_path.as_deref(),
        Some(by_project.join(DB_ID).to_str().unwrap())
    );
    assert_eq!(
        archive::archive_session(&w.db, &w.paths, &agents, DB_ID).unwrap(),
        Archived::Unchanged
    );
    // a session that changed is copied again, over the old copy
    agent_writes::add(
        &w.dbagent,
        DB_PROJECT,
        DB_ID,
        &[("a", 1793527400000, "Glad to help.")],
    );
    w.scan();
    assert!(matches!(
        archive::archive_session(&w.db, &w.paths, &agents, DB_ID).unwrap(),
        Archived::Copied(_)
    ));
    assert_eq!(names(&by_project), [DB_ID]);
    let before = conversation::read(&w.session(DB_ID), &w.paths, agent.as_ref()).unwrap();
    assert_eq!(before.len(), 4);

    // the agent deletes the session folder; eye still knows the session and reads its copy
    std::fs::remove_dir_all(&folder).unwrap();
    w.scan();
    let s = w.session(DB_ID);
    assert_eq!(s.transcript_state.as_deref(), Some("archived"));
    let from_copy = conversation::read(&s, &w.paths, agent.as_ref()).unwrap();
    assert_eq!(texts(&from_copy), texts(&before));
    assert!(!folder.exists(), "reading does not bring the session back");
    assert!(
        names(&w.paths.home.join("tmp")).is_empty(),
        "the temporary copy is removed"
    );
    // the summary tail reads a path that exists as well
    let copy = archive::readable(&s, &w.paths, agent.as_ref()).unwrap();
    let tail = agent.conversation_tail(&copy.path).unwrap();
    assert_eq!(tail.len(), 4);
    drop(copy);
    assert_eq!(hits(&w.db, "quokka"), 1, "still searchable");

    // resume puts the session folder back first
    archive::restore_if_missing(&w.db, &w.paths, &agents, &s).unwrap();
    assert!(folder.join("meta.json").is_file() && folder.join("session.db").is_file());
    let back = conversation::read(&s, &w.paths, agent.as_ref()).unwrap();
    assert_eq!(texts(&back), texts(&before));
    assert!(
        archive::restore(&w.db, &w.paths, &agents, DB_ID).is_err(),
        "a session that is there is not overwritten"
    );
    // the next scan finds it again and counts nothing twice
    let st = w.scan();
    assert_eq!((st.files_parsed, st.bytes_read), (1, 0));
    let s = w.session(DB_ID);
    assert_eq!(s.transcript_state.as_deref(), Some("present"));
    assert_eq!((s.prompt_count, s.reply_count), (2, 2));
}

#[tokio::test]
async fn claude_and_both_fake_agents_run_together() {
    let mut w = world(r#"["claude", "fake", "fakedb"]"#);
    let st = w.scan();
    assert_eq!((st.files_seen, st.files_parsed), (3, 3));
    for (id, agent) in [
        (CLAUDE_ID, "claude"),
        (FAKE_ID, "fake"),
        (DB_ID, folder_agent::ID),
    ] {
        assert_eq!(w.session(id).agent, agent);
    }
    let app = w.app();
    let board = get(&app, "/api/board").await;
    let all: Vec<Value> = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["cards"].as_array().unwrap().clone())
        .collect();
    assert_eq!(all.len(), 3, "{board}");
    let card = |id: &str| all.iter().find(|c| c["id"] == id).unwrap().clone();
    assert_eq!(
        card(DB_ID)["resume_cmd"],
        format!("cd '/work/db' && dbbot resume {DB_ID}")
    );
    assert_eq!(card(DB_ID)["agent_name"], "Dbbot");
    for (word, id) in [
        ("platypus", CLAUDE_ID),
        ("zebra", FAKE_ID),
        ("quokka", DB_ID),
    ] {
        let hit = get(&app, &format!("/api/search?q={word}")).await;
        assert_eq!(hit["results"][0]["card"]["id"], id, "{word}");
    }

    // each session grows in its own way; one scan reads only what is new in each
    let claude_add = format!(
        "{}\n",
        json!({"type":"user","timestamp":"2026-11-01T10:05:00Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
               "message":{"content":"One more thing"}})
    );
    let fake_add = "u|1793527500000|One more stripe\n";
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    append(&w.claude_file(), &claude_add);
    append(&w.fake_file(), fake_add);
    agent_writes::add(
        &w.dbagent,
        DB_PROJECT,
        DB_ID,
        &[("u", 1793527500000, "One more burrow")],
    );
    let st = w.scan();
    assert_eq!(st.files_parsed, 3);
    assert_eq!(
        st.bytes_read,
        (claude_add.len() + fake_add.len()) as u64 + 1,
        "bytes of the two files and one database row"
    );
    for (id, prompts) in [(CLAUDE_ID, 3), (FAKE_ID, 3), (DB_ID, 3)] {
        assert_eq!(w.session(id).prompt_count, prompts, "{id}");
    }

    // the archive: Claude Code's layout from before, one file for the first fake agent, a folder
    // for the second
    let agents = w.agents();
    let originals = (
        std::fs::read(w.claude_file()).unwrap(),
        std::fs::read(w.fake_file()).unwrap(),
    );
    for id in [CLAUDE_ID, FAKE_ID, DB_ID] {
        assert!(matches!(
            archive::archive_session(&w.db, &w.paths, &agents, id).unwrap(),
            Archived::Copied(_)
        ));
    }
    let arch = w.paths.home.join("archive");
    assert!(arch
        .join(format!("-work-claude/{CLAUDE_ID}.jsonl.zst"))
        .is_file());
    assert!(arch
        .join(format!("@fake/work-fake/{FAKE_ID}.jsonl.zst"))
        .is_file());
    assert!(arch.join(format!("@fakedb/{DB_PROJECT}/{DB_ID}")).is_dir());

    // all three are deleted by their agents: eye keeps them and reads every conversation
    std::fs::remove_file(w.claude_file()).unwrap();
    std::fs::remove_file(w.fake_file()).unwrap();
    std::fs::remove_dir_all(w.db_folder()).unwrap();
    w.scan();
    for id in [CLAUDE_ID, FAKE_ID, DB_ID] {
        assert_eq!(
            w.session(id).transcript_state.as_deref(),
            Some("archived"),
            "{id}"
        );
    }
    let last = |id: &str| {
        let app = app.clone();
        let id = id.to_string();
        async move {
            let talk = get(&app, &format!("/api/cards/{id}/conversation")).await;
            let m = talk["messages"].as_array().unwrap().clone();
            m.last().unwrap()["text"].as_str().unwrap().to_string()
        }
    };
    assert_eq!(last(CLAUDE_ID).await, "One more thing");
    assert_eq!(last(FAKE_ID).await, "One more stripe");
    assert_eq!(last(DB_ID).await, "One more burrow");
    assert!(names(&w.paths.home.join("tmp")).is_empty());

    // resume restores each one before it starts the agent
    for id in [CLAUDE_ID, FAKE_ID, DB_ID] {
        archive::restore_if_missing(&w.db, &w.paths, &agents, &w.session(id)).unwrap();
    }
    assert_eq!(std::fs::read(w.claude_file()).unwrap(), originals.0);
    assert_eq!(std::fs::read(w.fake_file()).unwrap(), originals.1);
    assert!(w.db_folder().join("session.db").is_file());
    let st = w.scan();
    assert_eq!(st.files_parsed, 3);
    for (id, prompts) in [(CLAUDE_ID, 3), (FAKE_ID, 3), (DB_ID, 3)] {
        let s = w.session(id);
        assert_eq!(s.transcript_state.as_deref(), Some("present"), "{id}");
        assert_eq!(s.prompt_count, prompts, "{id} counts nothing twice");
    }
}
