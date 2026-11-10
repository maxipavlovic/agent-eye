use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::{scan, ScanStats};
use agent_eye::search;
use std::io::Write;
use std::path::{Path, PathBuf};

const S1: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const S2: &str = "bbbbbbbb-0000-4000-8000-000000000002";
const FIXTURE: &str = include_str!("fixtures/full.jsonl");

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
        let home = tmp.path().join("home");
        let proj = claude.join("projects/-tmp-demo");
        std::fs::create_dir_all(proj.join(S1).join("subagents")).unwrap();
        std::fs::write(proj.join(format!("{S1}.jsonl")), FIXTURE).unwrap();
        std::fs::write(
            proj.join(S1).join("subagents/agent-a1.meta.json"),
            r#"{"agentType":"general-purpose","description":"Review the widget","finished":true}"#,
        )
        .unwrap();
        std::fs::write(
            proj.join(S1).join("subagents/agent-a2.meta.json"),
            r#"{"agentType":"Explore","description":"Still going","finished":false}"#,
        )
        .unwrap();
        let line = |ts: &str, ty: &str, text: &str| {
            format!(
                "{{\"type\":\"{ty}\",\"timestamp\":\"{ts}\",\"cwd\":\"/tmp/demo\",\"sessionId\":\"{S2}\",\"message\":{{\"content\":{}}}}}\n",
                if ty == "user" {
                    format!("\"{text}\"")
                } else {
                    format!("[{{\"type\":\"text\",\"text\":\"{text}\"}}]")
                }
            )
        };
        let s2 = line(
            "2026-11-01T10:01:40.000Z",
            "user",
            "Do the background chore",
        ) + &line("2026-11-01T10:02:40.000Z", "assistant", "Chore done");
        std::fs::write(proj.join(format!("{S2}.jsonl")), s2).unwrap();
        let job = claude.join("jobs/beef0001");
        std::fs::create_dir_all(&job).unwrap();
        std::fs::write(
            job.join("state.json"),
            format!(
                r#"{{"state":"done","detail":"Job finished fine","name":"Background chore","sessionId":"{S2}","createdAt":"2026-11-01T10:01:30Z","updatedAt":"2026-11-01T11:00:00Z"}}"#
            ),
        )
        .unwrap();
        std::fs::write(claude.join("jobs/pins.json"), "[]").unwrap();
        std::fs::create_dir_all(claude.join("sessions")).unwrap();
        std::fs::write(
            claude.join("sessions/1.json"),
            format!(r#"{{"pid":{},"sessionId":"{S1}","cwd":"/tmp/demo","status":"idle","updatedAt":5}}"#, std::process::id()),
        )
        .unwrap();
        std::fs::write(claude.join("sessions/999.key"), "SECRET").unwrap();
        let hist = [
            r#"{{"display":"find the zebra widget","project":"/tmp/gone","sessionId":"cccccccc-0000-4000-8000-000000000003","timestamp":1790000000000}}"#.replace("{{", "{").replace("}}", "}"),
            r#"{{"display":"and the zebra footer","project":"/tmp/gone","sessionId":"cccccccc-0000-4000-8000-000000000003","timestamp":1790000100000}}"#.replace("{{", "{").replace("}}", "}"),
            format!(r#"{{"display":"already indexed prompt","project":"/tmp/demo","sessionId":"{S1}","timestamp":1790000200000}}"#),
            r#"{"display":"no session id","project":"/x"}"#.to_string(),
        ]
        .join("\n")
            + "\n";
        std::fs::write(claude.join("history.jsonl"), hist).unwrap();
        let paths = Paths { home };
        paths.ensure_home().unwrap();
        let mut cfg = Config::load_or_create(&paths.config_path()).unwrap();
        cfg.override_agent_dir("claude", claude.clone());
        cfg.outputs.jira_projects = vec!["PROJ".into(), "ZED".into()];
        let db = Db::open(&paths.db_path()).unwrap();
        Env {
            _tmp: tmp,
            claude,
            paths,
            cfg,
            db,
        }
    }

    fn scan(&mut self) -> ScanStats {
        scan(&mut self.db, &self.cfg, false).unwrap()
    }

    fn s1_path(&self) -> PathBuf {
        self.claude
            .join("projects")
            .join("-tmp-demo")
            .join(format!("{S1}.jsonl"))
    }
}

fn count(db: &Db, sql: &str) -> i64 {
    db.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

#[test]
fn recaps_stored_with_claudes_tail_are_cleaned_once() {
    const TAIL: &str = "\n(disable recaps in /config)";
    let mut e = Env::new();
    e.scan();
    let set = |e: &Env, recap: &str| {
        e.db.conn
            .execute("UPDATE session SET recap=?1 WHERE id=?2", [recap, S1])
            .unwrap();
    };
    let recap = |e: &Env| e.db.session_by_id(S1).unwrap().unwrap().recap.unwrap();
    // what the scan stores is clean already
    assert!(!recap(&e).contains("disable recaps"));
    // an older row is cleaned by the first scan after the upgrade, which is the one that has
    // not cleaned this agent's rows yet
    e.db.conn
        .execute("DELETE FROM meta WHERE key='recaps_cleaned:claude'", [])
        .unwrap();
    set(&e, &format!("Goal: old row.{TAIL}"));
    e.scan();
    assert_eq!(recap(&e), "Goal: old row.");
    // and only that one
    set(&e, &format!("Goal: left alone.{TAIL}"));
    e.scan();
    assert!(recap(&e).ends_with("(disable recaps in /config)"));
}

#[test]
fn full_scan_builds_everything() {
    let mut e = Env::new();
    let st = e.scan();
    assert_eq!(st.files_seen, 2);
    assert_eq!(st.files_parsed, 2);

    let s = e.db.session_by_id(S1).unwrap().unwrap();
    assert_eq!(s.title.as_deref(), Some("My custom"));
    assert_eq!(s.title_source.as_deref(), Some("custom"));
    assert_eq!(s.cwd.as_deref(), Some("/tmp/demo"));
    assert_eq!(s.git_branch.as_deref(), Some("feature/x"));
    assert_eq!(
        s.kind.as_deref(),
        Some("bg"),
        "system line with sessionKind bg"
    );
    assert_eq!(s.prompt_count, 2);
    assert!(s.recap.as_deref().unwrap().starts_with("Goal: migrate"));
    assert_eq!(s.transcript_state.as_deref(), Some("present"));
    assert_eq!(s.live_status.as_deref(), Some("idle"));
    assert!(s.last_assistant_text.as_deref().unwrap().ends_with('?'));

    let outs = e.db.outputs_for(S1).unwrap();
    let has = |k: &str, v: &str| outs.iter().any(|o| o.kind == k && o.value == v);
    assert!(has("artifact", "https://claude.ai/artifact/abc12345-def"));
    assert!(has("commit", "1a2b3c4"));
    assert!(has(
        "pr",
        "https://bitbucket.org/acme/repo/pull-requests/61"
    ));
    assert!(has("jira", "PROJ-12345"));
    assert!(!has("folder", "/tmp/demo/src"), "temp folders are dropped");
    assert!(!outs
        .iter()
        .any(|o| o.value.contains("FAKEFAKE") || o.value == "PROJ-99999"));

    let streams = e.db.streams_for(S1).unwrap();
    let st = |k: &str, key: &str| {
        streams
            .iter()
            .find(|x| x.kind == k && x.key == key)
            .cloned()
    };
    assert_eq!(
        st("child", "agent-a1").unwrap().status.as_deref(),
        Some("done")
    );
    assert_eq!(
        st("child", "agent-a1").unwrap().title.as_deref(),
        Some("Review the widget")
    );
    assert_eq!(
        st("child", "agent-a2").unwrap().status.as_deref(),
        Some("running")
    );
    let art = st("output", "artifact:https://claude.ai/artifact/abc12345-def").unwrap();
    assert_eq!(art.status.as_deref(), Some("published"));
    assert_eq!(art.title.as_deref(), Some("Catalog plan"));
    assert_eq!(
        st(
            "output",
            "pr:https://bitbucket.org/acme/repo/pull-requests/61"
        )
        .unwrap()
        .status
        .as_deref(),
        Some("opened")
    );
    // background job: child of S1 via the bg-child rule, recap from the job detail
    let c = e.db.session_by_id(S2).unwrap().unwrap();
    assert_eq!(c.kind.as_deref(), Some("bg"));
    assert_eq!(c.parent_session_id.as_deref(), Some(S1));
    assert_eq!(c.recap.as_deref(), Some("Job finished fine"));
    assert_eq!(c.title.as_deref(), Some("Background chore"));
    let job_stream = st("child", S2).unwrap();
    assert_eq!(job_stream.status.as_deref(), Some("done"));
    assert_eq!(job_stream.link.as_deref(), Some(S2));

    // history import: only the session without a transcript
    let h =
        e.db.session_by_id("cccccccc-0000-4000-8000-000000000003")
            .unwrap()
            .unwrap();
    assert_eq!(h.kind.as_deref(), Some("history_only"));
    assert_eq!(h.transcript_state.as_deref(), Some("deleted"));
    assert_eq!(h.prompt_count, 2);
    assert_eq!(count(&e.db, "SELECT count(*) FROM session"), 3);

    // search
    let w = |s: &str| vec![s.to_string()];
    let r = search::find(&e.db, &w("zebra"), 10, false, 0).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].session.kind.as_deref(), Some("history_only"));
    let r = search::find(&e.db, &["catalog".into(), "widget".into()], 10, false, 0).unwrap();
    assert_eq!(r[0].session.id, S1);
    assert!(search::find(&e.db, &w("FAKEFAKEFAKE"), 10, false, 0)
        .unwrap()
        .is_empty());
    assert!(!search::find(&e.db, &w("PROJ-12345"), 10, false, 0)
        .unwrap()
        .is_empty());
    assert!(!search::find(&e.db, &w("pull-requests"), 10, false, 0)
        .unwrap()
        .is_empty());
    assert!(!search::find(&e.db, &w("Chore"), 10, false, 0)
        .unwrap()
        .is_empty());
}

#[test]
fn rescan_is_a_noop_and_append_reads_only_new_lines() {
    let mut e = Env::new();
    e.scan();
    let st = e.scan();
    assert_eq!(st.files_parsed, 0);
    assert_eq!(st.history_prompts, 0);

    let before = std::fs::metadata(e.s1_path()).unwrap().len();
    let add = concat!(
        "{\"type\":\"user\",\"timestamp\":\"2026-11-02T10:00:00.000Z\",\"cwd\":\"/tmp/demo\",\"sessionId\":\"aaaaaaaa-0000-4000-8000-000000000001\",\"message\":{\"content\":\"second round please, PROJ-77777\"}}\n",
        "{\"type\":\"assistant\",\"timestamp\":\"2026-11-02T10:01:00.000Z\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"all quokka done.\"}]}}\n"
    );
    append(&e.s1_path(), add);
    let st = e.scan();
    assert_eq!(st.files_parsed, 1);
    assert_eq!(st.bytes_read, add.len() as u64);
    assert_eq!(st.lines, 2);
    let s = e.db.session_by_id(S1).unwrap().unwrap();
    assert_eq!(s.prompt_count, 3);
    assert_eq!(s.title.as_deref(), Some("My custom"));
    assert!(s.last_assistant_text.as_deref().unwrap().contains("quokka"));
    assert!(s.recap.as_deref().unwrap().starts_with("Goal:"));
    assert!(e
        .db
        .outputs_for(S1)
        .unwrap()
        .iter()
        .any(|o| o.value == "PROJ-77777"));
    // old text still searchable, new text too
    let w = |s: &str| vec![s.to_string()];
    assert_eq!(
        search::find(&e.db, &w("quokka"), 5, false, 0)
            .unwrap()
            .len(),
        1
    );
    assert!(!search::find(&e.db, &w("migrate"), 5, false, 0)
        .unwrap()
        .is_empty());
    let cur =
        e.db.cursor_get(&e.s1_path().to_string_lossy())
            .unwrap()
            .unwrap();
    assert_eq!(cur.marker, (before + add.len() as u64).to_string());
}

#[test]
fn truncated_last_line_is_picked_up_once_complete() {
    let mut e = Env::new();
    e.scan();
    let full = "{\"type\":\"user\",\"timestamp\":\"2026-11-02T10:00:00.000Z\",\"message\":{\"content\":\"half written prompt\"}}\n";
    let (a, b) = full.split_at(40);
    append(&e.s1_path(), a);
    e.scan();
    assert_eq!(e.db.session_by_id(S1).unwrap().unwrap().prompt_count, 2);
    append(&e.s1_path(), b);
    e.scan();
    assert_eq!(e.db.session_by_id(S1).unwrap().unwrap().prompt_count, 3);
    e.scan();
    assert_eq!(e.db.session_by_id(S1).unwrap().unwrap().prompt_count, 3);
}

#[test]
fn shrunk_file_is_reread_from_start() {
    let mut e = Env::new();
    e.scan();
    let short = FIXTURE.lines().take(8).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(e.s1_path(), short).unwrap();
    let st = e.scan();
    assert_eq!(st.files_parsed, 1);
    let s = e.db.session_by_id(S1).unwrap().unwrap();
    assert_eq!(s.prompt_count, 1);
    // outputs from the dropped part are gone, search text rebuilt
    assert!(!e
        .db
        .outputs_for(S1)
        .unwrap()
        .iter()
        .any(|o| o.kind == "commit"));
    let w = vec!["rename".to_string()];
    assert!(search::find(&e.db, &w, 5, false, 0).unwrap().is_empty());
}

#[test]
fn transcript_deleted_later_and_exclusion() {
    let mut e = Env::new();
    e.scan();
    std::fs::remove_file(e.s1_path()).unwrap();
    e.scan();
    let s = e.db.session_by_id(S1).unwrap().unwrap();
    assert_eq!(s.transcript_state.as_deref(), Some("deleted"));

    let mut e2 = Env::new();
    e2.cfg.exclude.paths = vec!["/tmp/demo".into()];
    e2.scan();
    assert!(e2.db.session_by_id(S1).unwrap().is_none());
    assert!(e2
        .db
        .session_by_id("cccccccc-0000-4000-8000-000000000003")
        .unwrap()
        .is_some());
}

#[test]
fn full_flag_rebuilds_without_double_counting() {
    let mut e = Env::new();
    e.scan();
    let st = scan(&mut e.db, &e.cfg, true).unwrap();
    assert_eq!(st.files_parsed, 2);
    let s = e.db.session_by_id(S1).unwrap().unwrap();
    assert_eq!(s.prompt_count, 2);
    let h =
        e.db.session_by_id("cccccccc-0000-4000-8000-000000000003")
            .unwrap()
            .unwrap();
    assert_eq!(h.prompt_count, 2);
    assert_eq!(count(&e.db, "SELECT count(*) FROM session_fts"), 3);
}

/// One transcript line of an assistant message block; Claude Code repeats the usage on every
/// block of the same message id.
fn usage_line(id: &str, sidechain: bool, u: [i64; 4]) -> String {
    format!(
        "{{\"type\":\"assistant\",\"isSidechain\":{sidechain},\"timestamp\":\"2026-11-03T10:00:00.000Z\",\"message\":{{\"id\":\"{id}\",\"content\":[{{\"type\":\"text\",\"text\":\"x\"}}],\"usage\":{{\"input_tokens\":{},\"output_tokens\":{},\"cache_creation_input_tokens\":{},\"cache_read_input_tokens\":{}}}}}}}\n",
        u[0], u[1], u[2], u[3]
    )
}

fn tokens(e: &Env) -> agent_eye::model::Tokens {
    e.db.session_by_id(S1).unwrap().unwrap().tokens.unwrap()
}

#[test]
fn tokens_count_each_message_once_and_add_up_incrementally() {
    let mut e = Env::new();
    e.scan();
    let base = tokens(&e);
    // a message written as two lines counts once; a sidechain line in the main file is skipped
    append(
        &e.s1_path(),
        &(usage_line("m1", false, [10, 20, 30, 40])
            + &usage_line("m1", false, [10, 20, 30, 40])
            + &usage_line("side", true, [999, 999, 999, 999])),
    );
    e.scan();
    // the same id on the first line of the next scan is still the same message
    append(
        &e.s1_path(),
        &(usage_line("m1", false, [10, 20, 30, 40]) + &usage_line("m2", false, [1, 2, 3, 4])),
    );
    e.scan();
    let t = tokens(&e);
    assert_eq!(t.input, base.input + 11);
    assert_eq!(t.output, base.output + 22);
    assert_eq!(t.cache_write, base.cache_write + 33);
    assert_eq!(t.cache_read, base.cache_read + 44);
    assert_eq!(t.subagents_total, 0);
    assert_eq!(t.total(), t.main_total());

    // sub-agent transcripts are a separate sub-total, read incrementally
    let sub = e
        .claude
        .join("projects")
        .join("-tmp-demo")
        .join(S1)
        .join("subagents/agent-a1.jsonl");
    std::fs::write(
        &sub,
        usage_line("s1", true, [5, 5, 5, 5]) + &usage_line("s1", true, [5, 5, 5, 5]),
    )
    .unwrap();
    e.scan();
    assert_eq!(tokens(&e).subagents_total, 20);
    append(&sub, &usage_line("s2", true, [1, 1, 1, 1]));
    e.scan();
    let t2 = tokens(&e);
    assert_eq!(t2.subagents_total, 24);
    assert_eq!(t2.total(), t.main_total() + 24);
    assert_eq!(t2.input, t.input, "sub-agents stay out of the main counts");

    // a full rescan recomputes the same numbers
    scan(&mut e.db, &e.cfg, true).unwrap();
    assert_eq!(tokens(&e), t2);
    // and so does a rescan that changes nothing
    e.scan();
    assert_eq!(tokens(&e), t2);
}

#[test]
fn tokens_are_rebuilt_when_the_transcript_shrinks() {
    let mut e = Env::new();
    append(&e.s1_path(), &usage_line("m1", false, [7, 0, 0, 0]));
    e.scan();
    let before = tokens(&e);
    assert!(before.input >= 7);
    let short = FIXTURE.lines().take(8).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(e.s1_path(), short).unwrap();
    e.scan();
    assert!(tokens(&e).input < before.input);
}

#[test]
fn a_database_from_before_read_markers_goes_on_where_it_stopped() {
    let mut e = Env::new();
    let sub = e
        .claude
        .join("projects/-tmp-demo")
        .join(S1)
        .join("subagents/agent-a1.jsonl");
    std::fs::write(&sub, usage_line("s1", true, [5, 5, 5, 5])).unwrap();
    e.scan();
    let (prompts, base) = (
        e.db.session_by_id(S1).unwrap().unwrap().prompt_count,
        tokens(&e),
    );
    assert_eq!(base.subagents_total, 20);

    // what the build before markers left behind: real byte offsets and no marker column
    e.db.conn
        .execute_batch(
            "UPDATE cursor SET offset=CAST(marker AS INTEGER);
             ALTER TABLE cursor DROP COLUMN marker;
             UPDATE meta SET value='8' WHERE key='schema_version';",
        )
        .unwrap();
    e.db = Db::open(&e.paths.db_path()).unwrap();

    // the first scan after the upgrade reads nothing again
    let st = e.scan();
    assert_eq!(
        (st.files_parsed, st.bytes_read, st.history_prompts),
        (0, 0, 0)
    );
    assert_eq!(
        e.db.session_by_id(S1).unwrap().unwrap().prompt_count,
        prompts
    );
    assert_eq!(tokens(&e), base);

    // what is appended after it is read from the stored offset, and counted once
    let add = usage_line("m9", false, [1, 2, 3, 4]);
    append(&e.s1_path(), &add);
    let st = e.scan();
    assert_eq!((st.files_parsed, st.bytes_read), (1, add.len() as u64));
    let t = tokens(&e);
    assert_eq!((t.input, t.output), (base.input + 1, base.output + 2));
    append(&sub, &usage_line("s2", true, [1, 1, 1, 1]));
    e.scan();
    assert_eq!(tokens(&e).subagents_total, 24);
    // the cursors now carry markers
    let cur =
        e.db.cursor_get(&e.s1_path().to_string_lossy())
            .unwrap()
            .unwrap();
    assert_eq!(
        cur.marker,
        std::fs::metadata(e.s1_path()).unwrap().len().to_string()
    );
}
