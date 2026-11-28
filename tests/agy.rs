//! The Antigravity CLI (agy) adapter, from conversations in the shape agy 1.5 writes
//! (`tests/fixtures/agy-home`: the JSON log of each conversation under `brain/<id>/`; the SQLite
//! databases are made by this test, since `*.db` files are not committed): the scan, tokens by
//! model, search, an incremental read, titles that change in another database, the history, the
//! conversation, resume lines, the archive and detection.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;

use agent_eye::agents::{self, agy::Agy, Agent, Agents};
use agent_eye::archive::{self, Archived};
use agent_eye::config::Config;
use agent_eye::conversation;
use agent_eye::db::Db;
use agent_eye::model::Session;
use agent_eye::paths::Paths;
use agent_eye::scan::{scan, ScanStats};
use agent_eye::search;
use rusqlite::Connection;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A conversation with two prompts, tool calls, a failed tool, a pull request link, a title.
const A: &str = "a1111111-1111-4111-8111-111111111111";
/// A short one with no title; its folder is only in `history.jsonl`.
const B: &str = "b2222222-2222-4222-8222-222222222222";
/// A conversation eye started for a summary (folder `eye-ask-...`): not a session.
const C: &str = "c3333333-3333-4333-8333-333333333333";
/// A conversation of a sub-agent: not a session.
const D: &str = "d4444444-4444-4444-8444-444444444444";
/// A database with no log.
const E: &str = "e5555555-5555-4555-8555-555555555555";
/// Only its prompt is left, in the history.
const GONE: &str = "f6666666-6666-4666-8666-666666666666";

fn vi(mut n: u64) -> Vec<u8> {
    let mut o = vec![];
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            o.push(b);
            return o;
        }
        o.push(b | 0x80);
    }
}

fn ld(num: u64, b: &[u8]) -> Vec<u8> {
    let mut v = vi(num << 3 | 2);
    v.extend(vi(b.len() as u64));
    v.extend_from_slice(b);
    v
}

/// A `gen_metadata` row as agy writes it: the model call's data, with the model's name in it.
fn gen_row(model: &str) -> Vec<u8> {
    let mut inner = ld(19, b"test-flash");
    inner.extend(ld(21, model.as_bytes()));
    let mut row = ld(4, b"msg");
    row.extend(ld(1, &inner));
    row
}

struct World {
    _tmp: tempfile::TempDir,
    agy: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let agy = tmp.path().join("agy");
        fixtures::copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agy-home"),
            &agy,
        );
        let spec: Value =
            serde_json::from_str(&std::fs::read_to_string(agy.join("databases.json")).unwrap())
                .unwrap();
        std::fs::remove_file(agy.join("databases.json")).unwrap();
        std::fs::create_dir_all(agy.join("conversations")).unwrap();
        for c in spec["conversations"].as_array().unwrap() {
            let id = c["id"].as_str().unwrap();
            let conn = Connection::open(agy.join(format!("conversations/{id}.db"))).unwrap();
            conn.execute_batch(
                "CREATE TABLE gen_metadata (idx integer PRIMARY KEY, data blob, size integer NOT NULL DEFAULT 0);",
            )
            .unwrap();
            for (i, m) in c["models"].as_array().unwrap().iter().enumerate() {
                conn.execute(
                    "INSERT INTO gen_metadata(idx, data) VALUES(?1, ?2)",
                    (i as i64, gen_row(m.as_str().unwrap())),
                )
                .unwrap();
            }
        }
        let conn = Connection::open(agy.join("conversation_summaries.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE conversation_summaries (conversation_id text PRIMARY KEY, title text NOT NULL DEFAULT '',
             last_modified_time datetime NOT NULL, workspace_uris text NOT NULL, parent_conversation_id text NOT NULL DEFAULT '');",
        )
        .unwrap();
        for s in spec["summaries"].as_array().unwrap() {
            conn.execute(
                "INSERT INTO conversation_summaries VALUES(?1, ?2, ?3, ?4, ?5)",
                (
                    s["id"].as_str(),
                    s["title"].as_str(),
                    s["modified"].as_str(),
                    s["uris"].as_str(),
                    s["parent"].as_str(),
                ),
            )
            .unwrap();
        }
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::parse(&format!(
            "[agents]\nenabled = [\"agy\"]\n[agents.agy]\ndir = \"{}\"\n[outputs]\njira_projects = [\"APP\"]\n",
            agy.display()
        ))
        .unwrap();
        cfg.summary.enabled = true;
        let db = Db::open(&paths.db_path()).unwrap();
        World {
            _tmp: tmp,
            agy,
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
    fn folder(&self, id: &str) -> PathBuf {
        self.agy.join("brain").join(id)
    }
    fn log(&self, id: &str) -> PathBuf {
        self.folder(id)
            .join(".system_generated/logs/transcript_full.jsonl")
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
fn the_scan_finds_the_conversations_a_person_has_and_leaves_the_rest() {
    let mut w = World::new();
    let st = w.scan();
    assert_eq!((st.files_seen, st.files_parsed), (2, 2), "A and B");
    for id in [A, B] {
        let s = w.session(id);
        assert_eq!(s.agent, "agy", "{id}");
        assert_eq!(s.transcript_state.as_deref(), Some("present"));
    }
    for hidden in [C, D, E] {
        assert!(w.db.session_by_id(hidden).unwrap().is_none(), "{hidden}");
    }
    let a = w.session(A);
    assert_eq!(a.title.as_deref(), Some("Quokka census"));
    assert_eq!(
        a.cwd.as_deref(),
        Some("/work/zoo app"),
        "the uri is decoded"
    );
    assert_eq!(a.project_dir.as_deref(), Some("-work-zoo-app"));
    assert_eq!((a.prompt_count, a.reply_count), (2, 2));
    assert_eq!(
        a.first_prompt.as_deref(),
        Some("Count the quokkas in the census CSV and mention APP-12"),
        "the prompt is the user request, not its metadata"
    );
    assert_eq!(
        a.last_assistant_text.as_deref(),
        Some("The enclosure is by the lake.")
    );
    let b = w.session(B);
    assert_eq!(
        b.cwd.as_deref(),
        Some("/work/hello"),
        "the folder comes from the history"
    );
    assert_ne!(b.title.as_deref(), Some(""));
    assert_eq!(w.scan().files_parsed, 0, "a second scan reads nothing");
}

#[test]
fn tokens_add_up_and_the_model_comes_from_the_database() {
    let mut w = World::new();
    w.scan();
    let t = w.session(A).tokens.unwrap();
    assert_eq!(
        (t.input, t.output, t.cache_read, t.cache_write),
        (5_300, 370, 3_000, 0)
    );
    let agents = w.agents();
    let agent = agents.get("agy").unwrap();
    let msgs = agent.usage_msgs(&w.folder(A)).unwrap();
    assert_eq!(msgs.len(), 4);
    assert!(msgs.iter().all(|m| m.model == "Test Flash (High)"));
    assert_eq!(msgs.iter().map(|m| m.input).sum::<i64>(), t.input);
    assert_eq!(msgs.iter().map(|m| m.cache_read).sum::<i64>(), t.cache_read);
    assert_eq!(
        agent.usage_msgs(&w.folder(B)).unwrap()[0].model,
        "Test Flash (Low)"
    );
    let ev = agent.work_events(&w.folder(A)).unwrap();
    assert_eq!(ev.iter().filter(|e| e.prompt).count(), 2);
}

#[test]
fn search_finds_what_was_said_and_not_the_metadata() {
    let mut w = World::new();
    w.scan();
    assert!(hits(&w.db, "quokkas").contains(&A.to_string()));
    assert!(hits(&w.db, "lake").contains(&A.to_string()));
    assert!(hits(&w.db, "hello").contains(&B.to_string()));
    assert!(hits(&w.db, "ADDITIONAL_METADATA").is_empty());
    assert!(
        hits(&w.db, "Summarise").is_empty(),
        "an ask run is not indexed"
    );
}

#[test]
fn a_log_that_grows_is_read_on_and_a_title_change_is_noticed() {
    let mut w = World::new();
    let log = w.log(A);
    let full = std::fs::read_to_string(&log).unwrap();
    let cut: usize = full.lines().take(4).map(|l| l.len() + 1).sum();
    std::fs::write(&log, format!("{}{}", &full[..cut], &full[cut..cut + 30])).unwrap();
    w.scan();
    let s = w.session(A);
    assert_eq!((s.prompt_count, s.reply_count), (1, 1));
    let t = s.tokens.unwrap();
    assert_eq!((t.input, t.output, t.cache_read), (2_200, 250, 900));
    let cursor =
        w.db.cursor_get(&w.folder(A).to_string_lossy())
            .unwrap()
            .unwrap();
    assert_eq!(
        cursor.marker,
        cut.to_string(),
        "the half line is left for later"
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&log, &full).unwrap();
    assert_eq!(w.scan().files_parsed, 1);
    let s = w.session(A);
    assert_eq!(
        (s.prompt_count, s.reply_count),
        (2, 2),
        "nothing counted twice"
    );
    let t = s.tokens.unwrap();
    assert_eq!((t.input, t.output, t.cache_read), (5_300, 370, 3_000));
    // a new title is in the summaries database only
    std::thread::sleep(std::time::Duration::from_millis(20));
    let conn = Connection::open(w.agy.join("conversation_summaries.db")).unwrap();
    conn.execute(
        "UPDATE conversation_summaries SET title='Marsupial headcount', last_modified_time='2099-01-01 00:00:00.000000+00:00' WHERE conversation_id=?1",
        [A],
    )
    .unwrap();
    assert_eq!(w.scan().files_parsed, 1);
    assert_eq!(w.session(A).title.as_deref(), Some("Marsupial headcount"));
    assert_eq!(w.session(A).prompt_count, 2);
}

#[test]
fn the_history_keeps_prompts_of_conversations_that_are_gone() {
    let mut w = World::new();
    w.scan();
    let g = w.session(GONE);
    assert_eq!(g.kind.as_deref(), Some("history_only"));
    assert_eq!(g.cwd.as_deref(), Some("/work/gone"));
    assert!(hits(&w.db, "files").contains(&GONE.to_string()));
    let all = w.db.all_sessions().unwrap();
    assert_eq!(all.len(), 3, "A, B and the one only the history knows");
}

#[test]
fn the_conversation_reads_turns_with_their_tool_calls() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    let s = w.session(A);
    let msgs = conversation::read(&s, &w.paths, agents.get("agy").unwrap().as_ref()).unwrap();
    let shape: Vec<(&str, u32)> = msgs.iter().map(|m| (m.role, m.tools)).collect();
    assert_eq!(
        shape,
        [("user", 0), ("assistant", 1), ("user", 0), ("assistant", 1)]
    );
    assert!(msgs[1].text.contains("41 quokkas"));
    assert_eq!(msgs[2].text, "Thanks, now list the enclosures");
    let tail = agents
        .get("agy")
        .unwrap()
        .conversation_tail(&w.folder(A))
        .unwrap();
    let tools: Vec<String> = tail
        .iter()
        .filter_map(|t| match t {
            agents::TailItem::Tool(l) => Some(l.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        tools,
        [
            "$ wc -l census.csv",
            "search_web quokka enclosure",
            "error: permission check failed for unsandboxed \"ls\""
        ]
    );
}

#[test]
fn resume_and_start_lines() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    assert_eq!(
        agents.resume_line(&w.session(A)).unwrap(),
        format!("cd '/work/zoo app' && agy --conversation {A}")
    );
    assert!(
        agents.fork_line(&w.session(A)).is_none(),
        "agy has no fork command"
    );
}

#[test]
fn the_archive_keeps_a_snapshot_and_the_folder_without_scratch() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    assert!(matches!(
        archive::archive_session(&w.db, &w.paths, &agents, A).unwrap(),
        Archived::Copied(_)
    ));
    assert!(matches!(
        archive::archive_session(&w.db, &w.paths, &agents, A).unwrap(),
        Archived::Unchanged
    ));
    // the log grows: the copy is made again
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(w.log(A))
        .unwrap();
    f.write_all(b"\n").unwrap();
    assert!(matches!(
        archive::archive_session(&w.db, &w.paths, &agents, A).unwrap(),
        Archived::Copied(_)
    ));
    // agy deletes the conversation: the board still reads it from the copy
    std::fs::remove_dir_all(w.folder(A)).unwrap();
    std::fs::remove_file(w.agy.join("conversations").join(format!("{A}.db"))).unwrap();
    let s = w.session(A);
    let msgs = conversation::read(&s, &w.paths, agents.get("agy").unwrap().as_ref()).unwrap();
    assert_eq!(msgs.len(), 4);
    // a restore puts the folder back without scratch, and the database where agy keeps it
    let dest = archive::restore(&w.db, &w.paths, &agents, A).unwrap();
    assert!(dest.join(".system_generated/steps/1/output.txt").is_file());
    assert!(!dest.join("scratch").exists());
    assert!(
        !dest.join("eye-conversation.db").exists(),
        "the snapshot moved to agy's own place and would shadow it"
    );
    let snap = Connection::open(w.agy.join("conversations").join(format!("{A}.db"))).unwrap();
    let n: i64 = snap
        .query_row("SELECT count(*) FROM gen_metadata", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 4);
    assert_eq!(
        agents
            .get("agy")
            .unwrap()
            .read_conversation(&dest)
            .unwrap()
            .len(),
        4,
        "the restored folder reads the same"
    );
    assert!(agents.get("agy").unwrap().owns_transcript(&dest));
}

#[test]
fn detection_looks_for_the_folder_and_the_program() {
    let tmp = tempfile::tempdir().unwrap();
    let gone = Agy::new(tmp.path().join("nope")).with_executable("/nonexistent/agy");
    assert!(!gone.detect().installed);
    let here = Agy::new(tmp.path().to_path_buf()).with_executable("/nonexistent/agy");
    assert!(here.detect().installed);
    assert!(
        here.transcripts().files.is_empty(),
        "no conversations folder, no sessions"
    );
    let line = here
        .queue_line("Stop", &serde_json::json!({"conversationId": "c1"}), 5)
        .unwrap();
    assert_eq!(
        (line["event"].as_str(), line["session_id"].as_str()),
        (Some("Stop"), Some("c1"))
    );
}

/// agy's `-p` run (`--mode plan --sandbox`) has not been checked on a real agy with a hostile user
/// config, so it still writes summaries, as it did before 0.5, but is not sent prompt text to judge.
#[test]
fn agy_writes_summaries_but_is_not_asked_to_judge_corrections() {
    let mut cfg = Config::parse("[agents]\nenabled = [\"agy\"]\n").unwrap();
    cfg.summary.enabled = true;
    let agents = agents::enabled(&cfg);
    assert!(!agents.get("agy").unwrap().judging_locked_down());
    assert!(agents.summary_agent(&cfg).is_some());
    let (state, why) = agent_eye::clock::judging::judge(&cfg, &agents).unwrap_err();
    assert_eq!(state, agent_eye::clock::judging::Judging::Unavailable);
    assert!(why.contains("Antigravity CLI"), "{why}");
}
