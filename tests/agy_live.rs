//! What makes agy as alive as the other agents, from files in the shape agy 1.5.14 writes (ids and
//! text invented): the live status (presence lock, the summaries row, the waiting step), the remote
//! control link and `-p` runs in agy's process logs, sub-agent conversations, forgetting, a restore
//! agy can resume, and `install` / `uninstall` / `doctor`.

use agent_eye::agents::agy::Agy;
use agent_eye::agents::{Agent, InstallCtx, ParseOpts};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};

const A: &str = "a1111111-1111-4111-8111-111111111111";
const B: &str = "b2222222-2222-4222-8222-222222222222";
const KID: &str = "d4444444-4444-4444-8444-444444444444";
const RC: &str = "99999999-8888-4777-8666-555555555555-v2";

const FULL_SCHEMA: &str = "CREATE TABLE conversation_summaries (conversation_id text PRIMARY KEY,
    title text NOT NULL DEFAULT '', preview text NOT NULL DEFAULT '', step_count integer NOT NULL DEFAULT 0,
    last_modified_time datetime NOT NULL, workspace_uris text NOT NULL, status text NOT NULL DEFAULT '',
    source text NOT NULL DEFAULT '', project_id text NOT NULL DEFAULT '', agent_name text NOT NULL DEFAULT '',
    parent_conversation_id text NOT NULL DEFAULT '', nesting_depth integer NOT NULL DEFAULT 0,
    killed numeric NOT NULL DEFAULT false, raw_summary blob);";

struct Home {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    held: Vec<File>,
}

fn step_line(i: u32, kind: &str, extra: Value) -> String {
    let mut v = json!({"step_index": i, "type": kind, "status": "DONE", "created_at": "2026-11-21T21:55:13Z"});
    v.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    format!("{v}\n")
}

impl Home {
    fn new() -> Home {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("antigravity-cli");
        for d in [
            "conversations",
            "brain",
            "presence",
            "log",
            "cache",
            "annotations",
        ] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let c = Connection::open(dir.join("conversation_summaries.db")).unwrap();
        c.execute_batch(FULL_SCHEMA).unwrap();
        Home {
            _tmp: tmp,
            dir,
            held: vec![],
        }
    }

    /// A conversation: its database (a `steps` table with the last step's status), its log, its
    /// summaries row. `running` is the row's status; `last_status` the last step's.
    fn conversation(&self, id: &str, folder: &str, running: bool, last_status: i64, parent: &str) {
        let db = Connection::open(self.dir.join(format!("conversations/{id}.db"))).unwrap();
        db.execute_batch(
            "CREATE TABLE steps (idx integer PRIMARY KEY, step_type integer, status integer);
             CREATE TABLE gen_metadata (idx integer PRIMARY KEY, data blob);
             INSERT INTO steps VALUES (0, 14, 3), (1, 15, 3);",
        )
        .unwrap();
        db.execute("INSERT INTO steps VALUES (2, 132, ?1)", [last_status])
            .unwrap();
        let log = self.dir.join(format!(
            "brain/{id}/.system_generated/logs/transcript_full.jsonl"
        ));
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        let text = step_line(
            0,
            "USER_INPUT",
            json!({"source": "USER_EXPLICIT",
                "content": "<USER_REQUEST>\nCount the quokkas\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nx\n</ADDITIONAL_METADATA>"}),
        ) + &step_line(
            1,
            "PLANNER_RESPONSE",
            json!({"source": "MODEL", "model": "gemini-3.8-flash-low",
                "input_tokens": 1000, "cache_read_tokens": 500, "output_tokens": 20, "content": "Which enclosure?"}),
        );
        std::fs::write(log, text).unwrap();
        let uris = json!([format!("file://{folder}")]).to_string();
        let status = if running {
            "CASCADE_RUN_STATUS_RUNNING"
        } else {
            "CASCADE_RUN_STATUS_IDLE"
        };
        let c = Connection::open(self.dir.join("conversation_summaries.db")).unwrap();
        c.execute(
            "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time, workspace_uris,
             status, parent_conversation_id, agent_name, nesting_depth, raw_summary)
             VALUES (?1, 'Quokka census', '2026-11-21 21:55:13+00:00', ?2, ?3, ?4, ?5, ?6, x'0a0b0c')",
            (
                id,
                uris,
                status,
                parent,
                if parent.is_empty() { "" } else { "self" },
                i64::from(!parent.is_empty()),
            ),
        )
        .unwrap();
    }

    /// agy holds the presence lock while a conversation is open.
    fn open(&mut self, id: &str) {
        let f = File::create(self.dir.join(format!("presence/{id}.lock"))).unwrap();
        f.lock().unwrap();
        self.held.push(f);
    }

    fn agy(&self) -> Agy {
        Agy::new(self.dir.clone()).with_executable("/nonexistent/agy")
    }
}

fn live(a: &Agy, id: &str) -> Option<agent_eye::agents::LiveSession> {
    a.live_sessions().into_iter().find(|s| s.session_id == id)
}

#[test]
fn live_status_is_busy_idle_or_waiting_for_the_conversations_that_are_open() {
    let mut h = Home::new();
    h.conversation(A, "/work/zoo app", true, 3, "");
    h.conversation(B, "/work/zoo", false, 3, "");
    h.open(A);
    h.open(B);
    let a = h.agy();
    let s = live(&a, A).unwrap();
    assert_eq!(s.status.as_deref(), Some("busy"));
    assert_eq!(s.cwd.as_deref(), Some("/work/zoo app"));
    assert!(s.alive && s.waiting_for.is_none() && s.remote_url.is_none());
    assert_eq!(live(&a, B).unwrap().status.as_deref(), Some("idle"));
    assert!(a.live_status_shows_prompts());
    // agy asks to approve a tool call: the last step is waiting (status 9), the row still RUNNING
    let db = Connection::open(h.dir.join(format!("conversations/{A}.db"))).unwrap();
    db.execute("UPDATE steps SET status = 9 WHERE idx = 2", [])
        .unwrap();
    let s = live(&a, A).unwrap();
    assert_eq!(s.status.as_deref(), Some("waiting"));
    assert_eq!(s.waiting_for.as_deref(), Some("permission prompt"));
    // answered: the step is done again
    db.execute("UPDATE steps SET status = 3 WHERE idx = 2", [])
        .unwrap();
    assert_eq!(live(&a, A).unwrap().status.as_deref(), Some("busy"));
}

#[test]
fn a_lock_that_is_not_held_and_a_sub_agent_are_not_live() {
    let mut h = Home::new();
    h.conversation(A, "/work/zoo", true, 3, "");
    h.conversation(KID, "/work/zoo", true, 3, A);
    // files left behind by an agy that exited
    std::fs::write(h.dir.join(format!("presence/{A}.lock")), "").unwrap();
    let a = h.agy();
    assert!(a.live_sessions().is_empty());
    h.open(A);
    h.open(KID);
    let ids: Vec<String> = a
        .live_sessions()
        .into_iter()
        .map(|s| s.session_id)
        .collect();
    assert_eq!(
        ids,
        [A.to_string()],
        "the sub-agent shows as a child, not a session"
    );
}

#[test]
fn the_remote_control_link_and_a_print_run_come_from_agys_process_log() {
    let mut h = Home::new();
    h.conversation(A, "/work/zoo", false, 3, "");
    h.conversation(B, "/work/zoo", false, 3, "");
    h.open(A);
    h.open(B);
    let log = format!(
        "I1121 22:53:01.022774 447 remote_control_v2.go:2108] [remote-control-{RC}] Starting V2 remote control connection to: https://x:443\n\
         I1121 22:53:01.027804 297 server.go:1269] Created conversation {A} (backend=BACKEND_TYPE_UNSPECIFIED)\n\
         I1121 22:53:01.409304 447 remote_control_v2.go:2235] [remote-control-{RC}] Connection status: Connected\n"
    );
    std::fs::write(h.dir.join("log/cli-20261121_225300.log"), &log).unwrap();
    let print = format!(
        "I1121 22:58:19.653443 1 printmode.go:209] Print mode: starting (promptLength=31, model=\"m\", conversationID=\"\")\n\
         I1121 22:58:21.439838 1 session.go:201] Print mode: conversation={B}, sending message\n"
    );
    std::fs::write(h.dir.join("log/cli-20261121_225819.log"), print).unwrap();
    let a = h.agy();
    let s = live(&a, A).unwrap();
    assert_eq!(
        s.remote_url.as_deref(),
        Some(format!("https://antigravity.google.com/r/{RC}").as_str())
    );
    assert_eq!(s.entrypoint.as_deref(), Some("cli"));
    let b = live(&a, B).unwrap();
    assert_eq!(b.remote_url, None);
    assert_eq!(
        b.entrypoint.as_deref(),
        Some("sdk-cli"),
        "a -p run is headless"
    );
    assert_eq!(
        a.origin_raw(&h.dir.join("brain").join(B)).as_deref(),
        Some("print-mode")
    );
    assert_eq!(a.origin_raw(&h.dir.join("brain").join(A)), None);
    // the connection ended: no link
    std::fs::write(
        h.dir.join("log/cli-20261121_225300.log"),
        format!("{log}I1121 22:57:16.648102 447 x.go:1] [remote-control-{RC}] Connection ended. Retrying immediately...\n"),
    )
    .unwrap();
    assert_eq!(live(&a, A).unwrap().remote_url, None);
}

#[test]
fn the_source_column_is_the_raw_origin_when_agy_writes_one() {
    let h = Home::new();
    h.conversation(A, "/work/zoo", false, 3, "");
    let c = Connection::open(h.dir.join("conversation_summaries.db")).unwrap();
    c.execute(
        "UPDATE conversation_summaries SET source = 'CORTEX_TRAJECTORY_SOURCE_CLI'",
        [],
    )
    .unwrap();
    let a = h.agy();
    let t = a.transcripts().pop().unwrap();
    assert_eq!(
        a.origin_raw(&t.path).as_deref(),
        Some("CORTEX_TRAJECTORY_SOURCE_CLI")
    );
    let tokens = HashSet::new();
    let opts = ParseOpts {
        job_tokens: &tokens,
        ignore_dirs: &[],
        since: None,
        carry: None,
        last_msg_id: None,
    };
    let d = a.parse_transcript(&t, &opts).unwrap();
    assert_eq!(
        d.origin_raw.as_deref(),
        Some("CORTEX_TRAJECTORY_SOURCE_CLI")
    );
    assert_eq!(
        d.usage_msgs[0].model, "gemini-3.8-flash-low",
        "the log's own model key"
    );
}

#[test]
fn a_sub_agent_is_a_child_stream_of_its_parent_and_its_tokens_are_read() {
    let h = Home::new();
    h.conversation(A, "/work/zoo", false, 3, "");
    h.conversation(KID, "/work/zoo", true, 3, A);
    let manifest = h.dir.join(format!("brain/{A}/.system_generated/subagents"));
    std::fs::create_dir_all(&manifest).unwrap();
    std::fs::write(
        manifest.join(format!("{KID}.json")),
        json!({"conversationId": KID, "subagentDescriptor": {"typeName": "self", "role": "Counter"},
               "state": "SUBAGENT_STATE_ALIVE"})
        .to_string(),
    )
    .unwrap();
    let a = h.agy();
    let ts = a.transcripts();
    assert_eq!(ts.len(), 1, "only the parent is a session");
    let sub = a.subagents(&ts[0]);
    assert_eq!(sub.children.len(), 1);
    let c = &sub.children[0];
    assert_eq!((c.id.as_str(), c.title.as_str()), (KID, "Counter"));
    assert_eq!(c.detail.as_deref(), Some("self"));
    assert!(!c.finished, "its turn is still running");
    let u = a.read_usage(&sub.transcripts[0], None, None).unwrap();
    assert_eq!(
        (u.tokens.input, u.tokens.output, u.tokens.cache_read),
        (1000, 20, 500)
    );
    assert!(u.replaces);
    // finished once its turn is not running
    let db = Connection::open(h.dir.join("conversation_summaries.db")).unwrap();
    db.execute(
        "UPDATE conversation_summaries SET status = 'CASCADE_RUN_STATUS_IDLE'",
        [],
    )
    .unwrap();
    assert!(a.subagents(&ts[0]).children[0].finished);
}

#[test]
fn forgetting_a_conversation_leaves_nothing_of_it_and_nothing_of_another() {
    let mut h = Home::new();
    h.conversation(A, "/work/zoo", false, 3, "");
    h.conversation(B, "/work/zoo", false, 3, "");
    h.open(A);
    drop(h.held.pop()); // not held: nobody is running it
    std::fs::write(h.dir.join(format!("annotations/{A}.pbtxt")), "x").unwrap();
    let line = |text: &str, id: Option<&str>| {
        let mut v = json!({"display": text, "timestamp": 1, "workspace": "/work/zoo"});
        if let Some(id) = id {
            v["conversationId"] = json!(id);
        }
        format!("{v}\n")
    };
    let history = line("first of A", Some(A))
        + &line("typed in B", Some(B))
        + &line("/exit", Some(A))
        + &line("typed before", None);
    std::fs::write(h.dir.join("history.jsonl"), &history).unwrap();
    std::fs::write(
        h.dir.join("cache/last_conversations.json"),
        json!({"/work/zoo": A, "/work/other": B}).to_string(),
    )
    .unwrap();
    std::fs::write(
        h.dir.join("cache/conversation_metadata.json"),
        serde_json::to_string_pretty(
            &json!({"conversations": {A: {"summary": {}}, B: {"summary": {}}}}),
        )
        .unwrap(),
    )
    .unwrap();
    let a = h.agy();
    let gone = a
        .delete_transcript(&h.dir.join("brain").join(A), A)
        .unwrap()
        .unwrap()
        .join("\n");
    for what in [
        "2 line(s) of history.jsonl",
        "last_conversations.json",
        "conversation_metadata.json",
        "its row",
    ] {
        assert!(gone.contains(what), "{what}: {gone}");
    }
    let everything = |dir: &Path, id: &str| -> bool {
        let mut hit = false;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.file_name().unwrap().to_string_lossy().contains(id) {
                    hit = true;
                }
                if p.is_dir() {
                    stack.push(p);
                } else if std::fs::read(&p).is_ok_and(|b| String::from_utf8_lossy(&b).contains(id))
                {
                    hit = true;
                }
            }
        }
        hit
    };
    assert!(
        !everything(&h.dir, A),
        "a trace of the forgotten conversation is left"
    );
    assert!(everything(&h.dir, B), "the other conversation is kept");
    let rest = std::fs::read_to_string(h.dir.join("history.jsonl")).unwrap();
    assert_eq!(rest.lines().count(), 2);
    assert!(rest.contains("typed in B") && rest.contains("typed before"));
}

#[test]
fn a_restored_archive_is_a_conversation_agy_can_open_again() {
    let h = Home::new();
    h.conversation(A, "/work/zoo", false, 3, "");
    let a = h.agy();
    let folder = h.dir.join("brain").join(A);
    let tmp = tempfile::tempdir().unwrap();
    let copy = tmp.path().join(A);
    a.archive(&folder, &copy).unwrap();
    assert!(copy.join("eye-summary.json.zst").is_file());
    // agy deletes the conversation (everything but the files of other conversations)
    a.delete_transcript(&folder, A).unwrap();
    assert!(!h.dir.join(format!("conversations/{A}.db")).exists());
    let restoring = h.dir.join("brain").join(format!("{A}.restoring"));
    a.unarchive(&copy, &restoring).unwrap();
    std::fs::rename(&restoring, &folder).unwrap();
    // its database is where agy keeps it and its row is back, as it was
    let live_db = h.dir.join(format!("conversations/{A}.db"));
    let steps: i64 = Connection::open(&live_db)
        .unwrap()
        .query_row("SELECT count(*) FROM steps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(steps, 3);
    let c = Connection::open(h.dir.join("conversation_summaries.db")).unwrap();
    let (title, blob, parent): (String, Vec<u8>, String) = c
        .query_row(
            "SELECT title, raw_summary, parent_conversation_id FROM conversation_summaries WHERE conversation_id = ?1",
            [A],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (title.as_str(), blob, parent.as_str()),
        ("Quokka census", vec![10, 11, 12], "")
    );
    assert!(!folder.join("eye-conversation.db").exists());
    assert!(!folder.join("eye-summary.json").exists());
    assert_eq!(a.transcripts().len(), 1, "it is a session again");
    // a copy read for a board (not agy's own folder) keeps its snapshot and writes nothing of agy's
    let elsewhere = tmp.path().join("read");
    a.unarchive(&copy, &elsewhere).unwrap();
    assert!(elsewhere.join("eye-conversation.db").is_file());
}

#[test]
fn a_database_agy_closed_is_still_read() {
    // agy's databases are in WAL mode; with no process on them the -wal and -shm files are gone, and
    // SQLite cannot open such a file read-only
    let h = Home::new();
    h.conversation(A, "/work/zoo", false, 3, "");
    for f in [
        "conversation_summaries.db",
        &format!("conversations/{A}.db"),
    ] {
        let c = Connection::open(h.dir.join(f)).unwrap();
        c.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))
            .unwrap();
        drop(c);
        for ext in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(h.dir.join(format!("{f}{ext}")));
        }
    }
    let a = h.agy();
    let t = a.transcripts().pop().unwrap();
    let opts_tokens = HashSet::new();
    let d = a
        .parse_transcript(
            &t,
            &ParseOpts {
                job_tokens: &opts_tokens,
                ignore_dirs: &[],
                since: None,
                carry: None,
                last_msg_id: None,
            },
        )
        .unwrap();
    assert_eq!(d.ai_title.as_deref(), Some("Quokka census"));
    let mut h2 = Home::new();
    h2.conversation(B, "/work/zoo", true, 9, "");
    let c = Connection::open(h2.dir.join(format!("conversations/{B}.db"))).unwrap();
    c.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))
        .unwrap();
    drop(c);
    let _ = std::fs::remove_file(h2.dir.join(format!("conversations/{B}.db-wal")));
    let _ = std::fs::remove_file(h2.dir.join(format!("conversations/{B}.db-shm")));
    h2.open(B);
    assert_eq!(
        live(&h2.agy(), B).unwrap().waiting_for.as_deref(),
        Some("permission prompt")
    );
}

#[test]
fn install_adds_a_plugin_and_the_skill_and_uninstall_takes_only_them() {
    let h = Home::new();
    let a = h.agy();
    let config = h.dir.parent().unwrap().join("config");
    let bin = Path::new("/Users/you/.local/bin/agent-eye");
    let ctx = |dry_run| InstallCtx {
        eye_bin: bin,
        dry_run,
    };
    // somebody else's plugin and skill stay
    std::fs::create_dir_all(config.join("plugins/mine")).unwrap();
    std::fs::write(config.join("plugins/mine/hooks.json"), "{}").unwrap();
    std::fs::create_dir_all(config.join("skills/other")).unwrap();
    std::fs::write(config.join("skills/other/SKILL.md"), "x").unwrap();

    let dry = a.install(&ctx(true)).unwrap();
    assert!(
        dry.rows[0].text.contains("would add hooks"),
        "{}",
        dry.rows[0].text
    );
    assert!(
        !config.join("plugins/agent-eye").exists(),
        "a dry run writes nothing"
    );
    let rep = a.install(&ctx(false)).unwrap();
    assert_eq!(
        rep.rows[0].text,
        "hooks added (plugin agent-eye), skill added"
    );
    let hooks: Value = serde_json::from_str(
        &std::fs::read_to_string(config.join("plugins/agent-eye/hooks.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(hooks["agent-eye"].as_object().unwrap().len(), 2);
    let skill = std::fs::read_to_string(config.join("skills/eye/SKILL.md")).unwrap();
    assert!(
        skill.contains("name: eye") && skill.contains("Antigravity") && !skill.contains("Claude")
    );
    assert_eq!(
        a.install(&ctx(false)).unwrap().rows[0].text,
        "hooks ok in plugin agent-eye, skill ok"
    );
    // doctor sees the hooks; a damaged file is named
    let checks = a.doctor(&ctx(false));
    assert!(
        checks
            .iter()
            .any(|c| c.ok && c.msg.contains("hooks present")),
        "{checks:?}"
    );
    std::fs::write(
        config.join("plugins/agent-eye/hooks.json"),
        r#"{"agent-eye":{"Stop":[]}}"#,
    )
    .unwrap();
    let checks = a.doctor(&ctx(false));
    assert!(
        checks
            .iter()
            .any(|c| !c.ok && c.msg.contains("SessionStart")),
        "{checks:?}"
    );
    // the daemon's refresh updates an installed skill and adds none
    std::fs::write(config.join("skills/eye/SKILL.md"), "old").unwrap();
    assert!(a.refresh_skill().unwrap().is_some());
    assert_eq!(
        std::fs::read_to_string(config.join("skills/eye/SKILL.md")).unwrap(),
        skill
    );
    // uninstall
    let rep = a.uninstall(&ctx(false)).unwrap();
    assert_eq!(rep.rows[0].text, "hooks removed, skill removed");
    assert!(!config.join("plugins/agent-eye").exists() && !config.join("skills/eye").exists());
    assert!(
        config.join("plugins/mine/hooks.json").is_file()
            && config.join("skills/other/SKILL.md").is_file()
    );
    assert!(a.refresh_skill().unwrap().is_none(), "never adds one");
    assert!(a.uninstall(&ctx(false)).unwrap().rows.is_empty());
}

#[test]
fn a_plugin_of_the_same_name_that_is_not_eyes_is_left_alone() {
    let h = Home::new();
    let a = h.agy();
    let config = h.dir.parent().unwrap().join("config");
    std::fs::create_dir_all(config.join("plugins/agent-eye")).unwrap();
    std::fs::write(
        config.join("plugins/agent-eye/plugin.json"),
        r#"{"name":"theirs"}"#,
    )
    .unwrap();
    let ctx = InstallCtx {
        eye_bin: Path::new("/x/agent-eye"),
        dry_run: false,
    };
    assert!(a.preflight(&ctx).is_err());
    assert!(a.install(&ctx).is_err());
    assert!(config.join("plugins/agent-eye/plugin.json").is_file());
}
