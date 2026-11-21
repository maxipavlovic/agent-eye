//! OpenCode on a par with Codex: live status through the plugin's lines, summaries, archive and
//! restore, the `/eye` command and the plugin's install. Checked on a real OpenCode 2.3.2 (the
//! fixtures in `tests/fixtures/opencode-real` are its real output with invented text); here a fake
//! `opencode` script stands in for the program.

use agent_eye::agents::opencode::OpenCode;
use agent_eye::agents::{Agent, Agents, HeadlessRequest, InstallCtx};
use agent_eye::daemon::drain_queue;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/opencode-real")
            .join(name),
    )
    .unwrap()
}

struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    dir: PathBuf,
    fake: PathBuf,
}

impl World {
    /// A data folder with a database of the real schema, and a fake `opencode` that logs its calls
    /// to `calls.log` and behaves as `body` says.
    fn new(body: &str) -> World {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let dir = root.join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        let c = Connection::open(dir.join("opencode.db")).unwrap();
        c.execute_batch(&fixture("schema.sql")).unwrap();
        drop(c);
        let fake = root.join("fake-opencode");
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> '{r}/calls.log'\ncase \"$1 $2\" in\n{body}\nesac\n",
            r = root.display()
        );
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        World {
            _tmp: tmp,
            root,
            dir,
            fake,
        }
    }

    fn adapter(&self) -> OpenCode {
        OpenCode::new(self.dir.clone())
            .with_executable(self.fake.to_str().unwrap())
            .with_config_dir(self.root.join("config/opencode"))
    }

    fn sql(&self, q: &str, p: impl rusqlite::Params) {
        let c = Connection::open(self.dir.join("opencode.db")).unwrap();
        c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        c.execute(q, p).unwrap();
    }

    fn session(&self, id: &str, busy: Option<i64>, idle: Option<i64>, archived: Option<i64>) {
        self.sql(
            "INSERT INTO session_v2(id, project_id, slug, directory, title, version, time_created, time_updated, time_busy, time_idle, time_archived)
             VALUES(?1,'global','s','/work/demo','A title','2.3.2',1,?2,?3,?4,?5)",
            (id, busy.or(idle).unwrap_or(1), busy, idle, archived),
        );
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }
}

fn now() -> i64 {
    agent_eye::util::now_ms()
}

#[test]
fn live_status_comes_from_the_turn_times_and_an_archived_session_is_not_a_session() {
    let w = World::new("*) ;;");
    let n = now();
    w.session("ses_busy", Some(n - 5_000), None, None);
    w.session("ses_idle", Some(n - 60_000), Some(n - 30_000), None);
    w.session("ses_cold", Some(n - 9_000_000), Some(n - 8_000_000), None);
    w.session("ses_arch", Some(n - 5_000), None, Some(n - 1));
    let oc = w.adapter();
    let mut live: Vec<(String, String)> = oc
        .live_sessions()
        .into_iter()
        .map(|s| (s.session_id, s.status.unwrap()))
        .collect();
    live.sort();
    assert_eq!(
        live,
        [
            ("ses_busy".to_string(), "busy".to_string()),
            ("ses_idle".to_string(), "idle".to_string())
        ]
    );
    assert!(
        !oc.live_status_shows_prompts(),
        "the plugin shows prompts, not the row"
    );
    let ids: Vec<String> = oc.transcripts().into_iter().map(|t| t.session_id).collect();
    assert!(
        ids.contains(&"ses_busy".to_string()) && !ids.contains(&"ses_arch".to_string()),
        "{ids:?}"
    );
}

fn db_with_session(root: &Path, id: &str) -> (Db, Paths) {
    let paths = Paths {
        home: root.join("home"),
    };
    paths.ensure_home().unwrap();
    let db = Db::open(&paths.db_path()).unwrap();
    db.conn
        .execute(
            "INSERT INTO session(id, agent, project_dir, transcript_path, transcript_state, live_status) VALUES(?1,'opencode','p','/x/opencode.db#x','present','busy')",
            [id],
        )
        .unwrap();
    (db, paths)
}

fn queue(paths: &Paths, lines: &[&str]) {
    use std::io::Write;
    let p = agent_eye::hook::queue_path(paths);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

#[test]
fn the_lines_the_plugin_wrote_set_and_clear_the_wait() {
    let w = World::new("*) ;;");
    let (db, paths) = db_with_session(&w.root, "ses_demo1");
    let agents = Agents::from_vec(vec![Arc::new(w.adapter())]);
    let text = fixture("plugin-queue.jsonl");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        5,
        "SessionStart, UserPromptSubmit, PermissionRequest, PermissionReplied, Stop"
    );
    let wait = || -> (Option<i64>, Option<String>, Option<i64>) {
        db.conn
            .query_row("SELECT waiting_since, waiting_note, waiting_prompt FROM session WHERE id='ses_demo1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap()
    };
    queue(&paths, &lines[..3]);
    assert_eq!(drain_queue(&db, &paths, &agents).unwrap(), 3);
    let (since, note, prompt) = wait();
    assert!(
        since.is_some() && prompt == Some(1),
        "asked: {since:?} {prompt:?}"
    );
    assert_eq!(note.as_deref(), Some("allow shell: echo hi13 > out13.txt"));
    queue(&paths, &lines[3..]);
    drain_queue(&db, &paths, &agents).unwrap();
    let (since, note, prompt) = wait();
    assert_eq!((since, note, prompt), (None, None, Some(0)), "answered");
}

#[test]
fn a_permission_that_arrives_before_the_session_is_indexed_is_applied_after_the_scan() {
    let w = World::new("*) ;;");
    let paths = Paths {
        home: w.root.join("home"),
    };
    paths.ensure_home().unwrap();
    let db = Db::open(&paths.db_path()).unwrap();
    let agents = Agents::from_vec(vec![Arc::new(w.adapter())]);
    let text = fixture("plugin-queue.jsonl");
    queue(&paths, &text.lines().take(3).collect::<Vec<_>>());
    drain_queue(&db, &paths, &agents).unwrap();
    // the scan finds the session a moment later
    db.conn
        .execute(
            "INSERT INTO session(id, agent, project_dir, transcript_path, transcript_state) VALUES('ses_demo1','opencode','p','/x/opencode.db#x','present')",
            [],
        )
        .unwrap();
    agent_eye::daemon::apply_unplaced(&db, &agents).unwrap();
    let note: Option<String> = db
        .conn
        .query_row(
            "SELECT waiting_note FROM session WHERE id='ses_demo1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(note.as_deref(), Some("allow shell: echo hi13 > out13.txt"));
}

#[test]
fn the_hook_turns_the_plugins_payload_into_the_line_in_the_fixture() {
    let w = World::new("*) ;;");
    let paths = Paths {
        home: w.root.join("home"),
    };
    let oc = w.adapter();
    let payload = json!({"session_id": "ses_demo1", "tool_name": "shell", "resources": ["echo hi13 > out13.txt"]});
    agent_eye::hook::run(
        &paths,
        &oc,
        "PermissionRequest",
        payload.to_string().as_bytes(),
    );
    let written: Value = serde_json::from_str(
        std::fs::read_to_string(agent_eye::hook::queue_path(&paths))
            .unwrap()
            .trim(),
    )
    .unwrap();
    let real: Value =
        serde_json::from_str(fixture("plugin-queue.jsonl").lines().nth(2).unwrap()).unwrap();
    for k in ["event", "session_id", "tool_name", "tool_detail", "agent"] {
        assert_eq!(written[k], real[k], "{k}");
    }
}

fn events(session: &str, text: &str) -> String {
    [
        json!({"type": "step_start", "sessionID": session, "part": {"messageID": "m"}}),
        json!({"type": "text", "sessionID": session, "part": {"messageID": "m", "text": text}}),
        json!({"type": "step_finish", "sessionID": session, "part": {"tokens": {"input": 10, "output": 2, "reasoning": 0, "cache": {"read": 0, "write": 0}}, "cost": 0}}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n")
}

const GOOD: &str = r#"{"status":"done","state":"All done.","next":"Nothing.","topics":[]}"#;

/// A fake whose first `run` answers with `first` and the following ones with `then`.
fn asking(first: &str, then: &str) -> World {
    let body = "\"run --standalone\") cat > \"$(dirname \"$0\")/prompt-$$.txt\"; n=$(cat \"$(dirname \"$0\")/n\" 2>/dev/null || echo 0); echo $((n+1)) > \"$(dirname \"$0\")/n\"; if [ \"$n\" = 0 ]; then cat \"$(dirname \"$0\")/first.jsonl\"; else cat \"$(dirname \"$0\")/then.jsonl\"; fi;;\n\"run --help\") echo '--format --archived --title --standalone';;\n\"models \") printf 'anthropic/claude-sonnet-4-5\\nopencode/big-free\\nopencode/x-flash-free\\n';;\n*) ;;";
    let w = World::new(body);
    std::fs::write(w.root.join("first.jsonl"), events("ses_ask1", first)).unwrap();
    std::fs::write(w.root.join("then.jsonl"), events("ses_ask2", then)).unwrap();
    w
}

fn request<'a>(schema: &'a Value, work: &'a Path, model: &'a str) -> HeadlessRequest<'a> {
    HeadlessRequest {
        model,
        system: "Summarize.",
        input: "A session about nothing.",
        schema,
        work_dir: work,
        timeout: Duration::from_secs(20),
    }
}

#[test]
fn ask_asks_for_json_validates_retries_once_and_deletes_what_it_made() {
    let w = asking("Sure, here you go: not json at all", GOOD);
    let schema = agent_eye::summarize::schema();
    let work = w.root.join("work");
    let r = w.adapter().ask(&request(&schema, &work, "free")).unwrap();
    assert_eq!(r.output["status"], "done");
    assert_eq!(r.cost_usd, Some(0.0));
    let calls = w.calls();
    let runs: Vec<&String> = calls
        .iter()
        .filter(|c| c.starts_with("run ") && c.contains("--archived"))
        .collect();
    assert_eq!(runs.len(), 2, "one retry: {calls:?}");
    for r in &runs {
        assert!(
            r.contains("--archived")
                && r.contains("--format json")
                && r.contains("-m opencode/x-flash-free"),
            "{r}"
        );
    }
    assert!(
        calls.contains(&"session delete ses_ask1 --standalone".to_string()),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"session delete ses_ask2 --standalone".to_string()),
        "{calls:?}"
    );
    let prompts: Vec<String> = std::fs::read_dir(&w.root)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("prompt-"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    assert!(prompts
        .iter()
        .all(|p| p.contains("JSON schema") && p.contains("A session about nothing.")));
    assert!(
        prompts
            .iter()
            .any(|p| p.contains("previous answer was not valid")),
        "the retry says why"
    );
    assert!(
        std::fs::read_dir(&work).unwrap().next().is_none(),
        "the scratch folder is removed"
    );
}

#[test]
fn an_answer_that_breaks_the_schema_twice_is_an_error_and_a_named_model_is_used() {
    let w = asking(
        r#"{"status":"bogus","state":"x","next":"y","topics":[]}"#,
        r#"{"status":"done"}"#,
    );
    let schema = agent_eye::summarize::schema();
    let e = w
        .adapter()
        .ask(&request(&schema, &w.root.join("work"), "opencode/mine"))
        .unwrap_err();
    assert!(format!("{e:#}").contains("twice"), "{e:#}");
    let calls = w.calls();
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("run ") && c.contains("-m opencode/mine")),
        "{calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.starts_with("models")),
        "a named model needs no lookup"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.starts_with("session delete"))
            .count(),
        2
    );
}

#[test]
fn the_default_summary_model_is_the_free_one() {
    assert_eq!(OpenCode::new("/x".into()).default_summary_model(), "free");
}

/// A world whose database has the real session of the export fixture, and a fake whose `session
/// export` prints that export and whose `session import` puts the snapshot database back.
fn restorable() -> (World, String) {
    let export = fixture("session-export.json");
    let id = serde_json::from_str::<Value>(&export).unwrap()["info"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let w = World::new("*) ;;");
    let body = format!(
        "\"session export\") cat '{r}/export.json';;\n\"session import\") cp \"$3\" '{r}/imported.json'; cp '{r}/snapshot.db' '{r}/opencode/opencode.db';;\n*) ;;",
        r = w.root.display()
    );
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{r}/calls.log'\ncase \"$1 $2\" in\n{body}\nesac\n",
        r = w.root.display()
    );
    std::fs::write(&w.fake, script).unwrap();
    std::fs::write(w.root.join("export.json"), &export).unwrap();
    w.session(&id, None, Some(now() - 1000), None);
    w.sql("INSERT INTO session_message(id, session_id, type, seq, time_created, time_updated, data) VALUES('msg_1',?1,'user',1,1,1,?2)", (&id, json!({"text": "hello there", "files": []}).to_string()));
    std::fs::copy(w.dir.join("opencode.db"), w.root.join("snapshot.db")).unwrap();
    (w, id)
}

#[test]
fn the_archive_keeps_opencodes_export_and_restore_puts_the_session_back() {
    let (w, id) = restorable();
    let oc = w.adapter();
    let t = oc
        .transcripts()
        .into_iter()
        .find(|t| t.session_id == id)
        .unwrap();
    assert!(oc.owns_transcript(&t.path), "restoring is offered");
    let copy = w.root.join("copy.json.zst");
    oc.archive(&t.path, &copy).unwrap();
    assert!(w
        .calls()
        .contains(&format!("session export {id} --standalone")));
    // OpenCode deletes the session
    oc.delete_transcript(&t.path, &id).unwrap();
    assert!(!oc.transcript_exists(&t.path));
    oc.restore_archived(&copy, &t.path)
        .expect("offered")
        .unwrap();
    assert!(oc.transcript_exists(&t.path), "back in the database");
    let sent: Value =
        serde_json::from_str(&std::fs::read_to_string(w.root.join("imported.json")).unwrap())
            .unwrap();
    assert_eq!(
        sent["info"]["id"],
        id.as_str(),
        "what was exported is what is imported"
    );
    assert!(w
        .calls()
        .iter()
        .any(|c| c.starts_with("session import ") && c.ends_with("--standalone")));
    // the session is there now: a second restore does nothing
    let again = oc.restore_archived(&copy, &t.path).unwrap().unwrap_err();
    assert!(again.to_string().contains("still in OpenCode"), "{again}");
}

#[test]
fn a_copy_made_without_opencode_reads_but_does_not_restore() {
    let (w, id) = restorable();
    std::fs::write(
        &w.fake,
        "#!/bin/sh\ncase \"$1 $2\" in\n\"session export\") echo nope >&2; exit 1;;\nesac\n",
    )
    .unwrap();
    let oc = w.adapter();
    let t = oc
        .transcripts()
        .into_iter()
        .find(|t| t.session_id == id)
        .unwrap();
    let copy = w.root.join("copy.json.zst");
    oc.archive(&t.path, &copy).unwrap();
    oc.delete_transcript(&t.path, &id).unwrap();
    let e = oc.restore_archived(&copy, &t.path).unwrap().unwrap_err();
    assert!(
        e.to_string().contains("without `opencode session export`"),
        "{e}"
    );
    // still readable
    let dest = w.root.join("read-copy");
    oc.unarchive(&copy, &dest).unwrap();
    assert_eq!(oc.read_conversation(&dest).unwrap()[0].text, "hello there");
}

fn ctx(eye: &Path) -> InstallCtx<'_> {
    InstallCtx {
        eye_bin: eye,
        dry_run: false,
    }
}

#[test]
fn install_adds_the_plugin_and_the_command_next_to_other_ones_and_uninstall_takes_only_them() {
    let w = World::new("*) ;;");
    let cfg = w.root.join("config/opencode");
    std::fs::create_dir_all(cfg.join("plugins")).unwrap();
    std::fs::create_dir_all(cfg.join("commands")).unwrap();
    std::fs::write(cfg.join("plugins/other.js"), "export default {}\n").unwrap();
    std::fs::write(cfg.join("commands/mine.md"), "mine\n").unwrap();
    std::fs::write(cfg.join("opencode.json"), "{\"model\":\"x/y\"}\n").unwrap();
    let eye = Path::new("/opt/bin/agent-eye");
    let oc = w.adapter();
    oc.preflight(&ctx(eye)).unwrap();
    let rep = oc.install(&ctx(eye)).unwrap();
    assert_eq!(rep.rows[0].text, "plugin added, /eye command added");
    assert!(rep.next[0].contains("opencode reload"), "{:?}", rep.next);
    let plugin = std::fs::read_to_string(cfg.join("plugins/agent-eye.js")).unwrap();
    assert!(
        plugin.contains("const EYE = \"/opt/bin/agent-eye\"") && !plugin.contains("__EYE_BIN__")
    );
    let cmd = std::fs::read_to_string(cfg.join("commands/eye.md")).unwrap();
    assert!(
        cmd.starts_with("---\ndescription: ")
            && cmd.contains("$ARGUMENTS")
            && !cmd.contains("Claude")
    );
    // again: nothing changes
    let rep = oc.install(&ctx(eye)).unwrap();
    assert_eq!(rep.rows[0].text, "plugin ok, /eye command ok");
    assert!(rep.next.is_empty());
    // doctor
    let checks = oc.doctor(&ctx(eye));
    assert!(
        checks
            .iter()
            .any(|c| c.ok && c.msg.starts_with("plugin in place")),
        "{checks:?}"
    );
    assert!(
        checks
            .iter()
            .any(|c| c.ok && c.msg == "/eye command in place"),
        "{checks:?}"
    );
    let moved = oc.doctor(&ctx(Path::new("/elsewhere/agent-eye")));
    assert!(
        moved.iter().any(|c| !c.ok && c.msg.contains("out of date")),
        "{moved:?}"
    );
    // a new version of eye updates both
    assert_eq!(
        oc.refresh_skill_with(Path::new("/new/agent-eye")).unwrap(),
        Some(agent_eye::agents::skill_file::State::Updated)
    );
    assert!(std::fs::read_to_string(cfg.join("plugins/agent-eye.js"))
        .unwrap()
        .contains("/new/agent-eye"));
    // uninstall
    let rep = oc.uninstall(&ctx(eye)).unwrap();
    assert_eq!(rep.rows[0].text, "plugin removed, /eye command removed");
    assert!(!cfg.join("plugins/agent-eye.js").exists() && !cfg.join("commands/eye.md").exists());
    assert!(cfg.join("plugins/other.js").exists() && cfg.join("commands/mine.md").exists());
    assert_eq!(
        std::fs::read_to_string(cfg.join("opencode.json")).unwrap(),
        "{\"model\":\"x/y\"}\n"
    );
}

#[test]
fn a_file_of_somebody_else_with_eyes_names_is_never_overwritten_or_removed() {
    let w = World::new("*) ;;");
    let cfg = w.root.join("config/opencode");
    std::fs::create_dir_all(cfg.join("plugins")).unwrap();
    std::fs::write(cfg.join("plugins/agent-eye.js"), "// mine\n").unwrap();
    let eye = Path::new("/opt/bin/agent-eye");
    let oc = w.adapter();
    let e = oc.preflight(&ctx(eye)).unwrap_err();
    assert!(e.to_string().contains("is not eye's plugin"), "{e}");
    oc.uninstall(&ctx(eye)).unwrap();
    assert_eq!(
        std::fs::read_to_string(cfg.join("plugins/agent-eye.js")).unwrap(),
        "// mine\n"
    );
}

#[test]
fn without_opencode_install_says_so_and_writes_nothing() {
    let w = World::new("*) ;;");
    let oc = OpenCode::new(w.root.join("nothing-here"))
        .with_executable("/nonexistent/opencode")
        .with_config_dir(w.root.join("config/opencode"));
    let rep = oc.install(&ctx(Path::new("/opt/bin/agent-eye"))).unwrap();
    assert!(rep.rows[0].text.contains("not found"));
    assert!(!w.root.join("config/opencode").exists());
}
