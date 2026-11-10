//! The Codex CLI adapter, from rollouts in the shape Codex 0.167 writes (`tests/fixtures/codex-home`:
//! a paginated session with tool calls and tokens, a legacy one with an IDE-made prompt and a
//! compaction, a renamed one, a fork, a sub-agent, a `codex exec` run that stays hidden, an
//! archived session, and two sessions recorded by a real Codex 0.167) through the scanner, search,
//! conversations, summaries' tail, resume lines, the archive and the board.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;

use agent_eye::agents::{self, Agents};
use agent_eye::archive::{self, Archived};
use agent_eye::config::Config;
use agent_eye::conversation;
use agent_eye::db::Db;
use agent_eye::model::Session;
use agent_eye::paths::Paths;
use agent_eye::scan::{scan, ScanStats};
use agent_eye::search;
use agent_eye::server::{new_state, router};
use agent_eye::summarize;
use axum::body::Body;
use axum::http::Request;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
/// A paginated session: tool calls, tokens, a commit, a patch, a changed folder.
const A: &str = "019a6b2e-7c10-7a41-9d3e-5f0c2b7a1e44";
/// A legacy session, renamed with `/rename`.
const B: &str = "019a6a00-1111-7aaa-8bbb-000000000001";
/// A `codex exec` run: hidden.
const C: &str = "019a6c00-2222-7bbb-8ccc-000000000002";
/// A sub-agent of A.
const D: &str = "019a6b2f-3333-7ccc-8ddd-000000000003";
/// A fork of A with a turn still open.
const E: &str = "019a6d00-4444-7ddd-8eee-000000000004";
/// A session `codex archive` moved away.
const F: &str = "019a5f00-5555-7eee-8fff-000000000005";
/// Two sessions of a real Codex 0.167 (sanitized): code mode tools, one prompt, a change to `calc.py`.
const R1: &str = "01a1c43f-c156-7973-bc1f-98ddad8d4ad8";
const R2: &str = "01a1c441-54b9-7610-b102-c889d7d3e6f8";

struct World {
    _tmp: tempfile::TempDir,
    codex: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        fixtures::copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex-home"),
            &codex,
        );
        World::around(tmp, codex)
    }

    /// An empty Codex folder.
    fn empty() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        World::around(tmp, codex)
    }

    fn around(tmp: tempfile::TempDir, codex: PathBuf) -> World {
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::parse(&format!(
            "[agents]\nenabled = [\"codex\"]\n[agents.codex]\ndir = \"{}\"\n[outputs]\njira_projects = [\"APP\"]\n",
            codex.display()
        ))
        .unwrap();
        cfg.summary.enabled = true;
        let db = Db::open(&paths.db_path()).unwrap();
        World {
            _tmp: tmp,
            codex,
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

    /// Where the rollout of a thread is in the tree.
    fn rollout(&self, id: &str) -> PathBuf {
        let mut stack = vec![self.codex.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.to_string_lossy().contains(id) {
                    return p;
                }
            }
        }
        panic!("no rollout of {id}");
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

fn hits(db: &Db, word: &str) -> Vec<String> {
    search::find(db, &[word.to_string()], 10, false, 0)
        .unwrap()
        .into_iter()
        .map(|h| h.session.id)
        .collect()
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Value {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:7420")
        .header("x-eye-token", TOKEN)
        .header("origin", "http://127.0.0.1:7420")
        .header("content-type", "application/json")
        .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

#[test]
fn the_scan_finds_the_sessions_a_person_has_and_leaves_the_rest() {
    let mut w = World::new();
    let st = w.scan();
    assert_eq!(
        (st.files_seen, st.files_parsed),
        (6, 6),
        "A, B, E and F, and the two real ones"
    );
    for id in [A, B, E, F, R1, R2] {
        assert_eq!(w.session(id).agent, "codex", "{id}");
        assert_eq!(w.session(id).transcript_state.as_deref(), Some("present"));
    }
    assert!(
        w.db.session_by_id(C).unwrap().is_none(),
        "a codex exec run is hidden, as in Codex's own picker"
    );
    assert!(
        w.db.session_by_id(D).unwrap().is_none(),
        "a sub-agent is not a card"
    );
    // a second scan reads nothing
    assert_eq!(w.scan().files_parsed, 0);
}

#[test]
fn a_paginated_session_is_read_in_full() {
    let mut w = World::new();
    w.scan();
    let s = w.session(A);
    assert_eq!(
        s.title.as_deref(),
        Some("Fix the flaky retry loop, see APP-123")
    );
    assert_eq!(s.title_source.as_deref(), Some("prompt"));
    assert_eq!(
        s.first_prompt.as_deref(),
        Some("Fix the flaky retry loop, see APP-123")
    );
    assert_eq!((s.prompt_count, s.reply_count), (2, 3));
    assert_eq!(
        s.cwd.as_deref(),
        Some("/Users/you/code/app/server"),
        "the folder of the last turn, where a resume belongs"
    );
    assert_eq!(s.git_branch.as_deref(), Some("APP-123-retry"));
    assert_eq!(s.project_dir.as_deref(), Some("2026-11"));
    assert_eq!(s.last_assistant_text.as_deref(), Some("Added the entry."));
    assert_eq!(
        s.last_user_text.as_deref(),
        Some("Yes, add the changelog entry")
    );
    let at = |t: &str| agent_eye::util::iso_to_ms(t);
    assert_eq!(
        (s.started_at, s.last_activity_at),
        (
            at("2026-11-09T14:03:11.482Z"),
            at("2026-11-09T14:10:05.300Z")
        )
    );

    let outs = w.db.outputs_for(A).unwrap();
    let values = |kind: &str| -> Vec<String> {
        outs.iter()
            .filter(|o| o.kind == kind)
            .map(|o| o.value.clone())
            .collect()
    };
    assert_eq!(
        values("commit"),
        ["3f2c1ab"],
        "the commit that failed is not one"
    );
    assert_eq!(values("pr"), ["https://github.com/org/app/pull/42"]);
    assert_eq!(values("jira"), ["APP-123"]);
    let mut folders = values("folder");
    folders.sort();
    assert_eq!(
        folders,
        [
            "/Users/you/code/app/server",
            "/Users/you/code/app/server/tests"
        ],
        "the folders of the files the patch changed"
    );
}

/// What the summary excerpt and the conversation of a session say, to check that no source of the
/// model's own programs is in them.
fn what_is_shown(w: &World, id: &str) -> String {
    let agents = w.agents();
    let s = w.session(id);
    let agent = agents.of_session(&s).unwrap();
    let mut shown = summarize::build_excerpt(&w.db, agent.as_ref(), &s)
        .unwrap()
        .text;
    for m in conversation::read(&s, &w.paths, agent.as_ref()).unwrap() {
        shown.push_str(&m.text);
    }
    shown
}

/// Real rollouts of Codex 0.167 (`tests/fixtures/codex-home/sessions/2026/11/10`, with the paths
/// and ids made up): every tool runs through code mode, so the model's own record of a call is a
/// JavaScript program, and what ran is in the `item_completed` items next to it.
#[test]
fn a_real_session_is_read_from_its_items_not_from_the_programs() {
    let mut w = World::new();
    w.scan();
    let s = w.session(R1);
    assert_eq!(
        s.first_prompt.as_deref(),
        Some("Look at calc.py and add a subtract function next to add, then tell me in one sentence what you changed.")
    );
    assert_eq!(
        s.title.as_deref(),
        Some("Look at calc.py and add a subtract function next to add, then tell me in one sen"),
        "the first prompt, cut to a title"
    );
    assert_eq!(s.title_source.as_deref(), Some("prompt"));
    assert_eq!((s.prompt_count, s.reply_count), (1, 2));
    assert_eq!(s.cwd.as_deref(), Some("/Users/you/code/demo"));
    assert_eq!(s.git_branch.as_deref(), Some("master"));
    assert_eq!(s.project_dir.as_deref(), Some("2026-11"));
    assert_eq!(
        s.last_assistant_text.as_deref(),
        Some("Added `subtract(a, b)` beside `add`; it returns `a - b`.")
    );

    // 44924 tokens: 44759 in (37120 of them from the cache) and 165 out
    let t = s.tokens.unwrap();
    assert_eq!(
        (t.input, t.cache_read, t.cache_write, t.output),
        (7_639, 37_120, 0, 165)
    );
    assert_eq!(t.total(), 44_924);
    let t2 = w.session(R2).tokens.unwrap();
    assert_eq!(
        (t2.input, t2.cache_read, t2.output),
        (75_195 - 66_688, 66_688, 398)
    );

    // the change to calc.py is a folder output, once though the file records it twice
    let outs = w.db.outputs_for(R1).unwrap();
    let kinds: Vec<(&str, &str)> = outs
        .iter()
        .map(|o| (o.kind.as_str(), o.value.as_str()))
        .collect();
    assert_eq!(kinds, [("folder", "/Users/you/code/demo")]);
    let kinds2: Vec<String> =
        w.db.outputs_for(R2)
            .unwrap()
            .iter()
            .map(|o| format!("{} {}", o.kind, o.value))
            .collect();
    assert_eq!(kinds2, ["folder /Users/you/code/demo"]);

    // one prompt and a change: worth a card, not the archive
    assert_eq!(card_of(&w, R1).0, "paused");
    assert_eq!(card_of(&w, R2).0, "paused");

    // the conversation counts what ran (a read and a patch), not the two programs and their outputs
    let agents = w.agents();
    let agent = agents.of_session(&s).unwrap();
    let all = conversation::read(&s, &w.paths, agent.as_ref()).unwrap();
    let got: Vec<(&str, u32)> = all.iter().map(|m| (m.role, m.tools)).collect();
    assert_eq!(got, [("user", 0), ("assistant", 2)]);
    let all2 = conversation::read(&w.session(R2), &w.paths, agent.as_ref()).unwrap();
    assert_eq!(all2[1].tools, 4, "a search, two reads and a patch");

    // the summary's tail says what was run and changed
    let ex = summarize::build_excerpt(&w.db, agent.as_ref(), &s).unwrap();
    for want in [
        "[user] Look at calc.py and add a subtract function",
        "> $ sed -n '1,200p' calc.py\n",
        "> edit /Users/you/code/demo/calc.py\n",
        "[assistant] Added `subtract(a, b)` beside `add`; it returns `a - b`.",
    ] {
        assert!(ex.text.contains(want), "{want}\n{}", ex.text);
    }
    let ex2 = summarize::build_excerpt(&w.db, agent.as_ref(), &w.session(R2)).unwrap();
    for want in [
        "> $ rg --files -g 'calc.py' -g 'AGENTS.md'\n",
        "> $ cat calc.py\n",
        "> edit /Users/you/code/demo/calc.py\n",
    ] {
        assert!(ex2.text.contains(want), "{want}\n{}", ex2.text);
    }
    for id in [R1, R2] {
        let shown = what_is_shown(&w, id);
        for js in [
            "await ",
            "tools.",
            "const r ",
            "text(r",
            "max_output_tokens",
        ] {
            assert!(!shown.contains(js), "{id}: {js}\n{shown}");
        }
    }

    // a session that is not open resumes in its folder
    let agents = w.agents();
    assert_eq!(
        agents.resume_line(&s).as_deref(),
        Some(format!("cd '/Users/you/code/demo' && codex resume {R1}").as_str())
    );
}

#[test]
fn tokens_are_the_last_total_and_sub_agents_are_counted_apart() {
    let mut w = World::new();
    w.scan();
    let t = w.session(A).tokens.unwrap();
    // the last total of the thread (20000 in, 15000 of them cached, 1500 out), not a sum of the two
    assert_eq!(
        (t.input, t.cache_read, t.cache_write, t.output),
        (5_000, 15_000, 0, 1_500)
    );
    assert_eq!(t.subagents_total, 3_200, "the sub-agent's own thread total");
    let b = w.session(B).tokens.unwrap();
    assert_eq!((b.input, b.cache_read, b.output), (300, 100, 80));
    let streams = w.db.streams_for(A).unwrap();
    let child = streams
        .iter()
        .find(|s| s.kind == "child")
        .expect("the sub-agent is a child stream");
    assert_eq!(
        (
            child.key.as_str(),
            child.title.as_deref(),
            child.detail.as_deref(),
            child.status.as_deref()
        ),
        (D, Some("Hume"), Some("explorer"), Some("done"))
    );
}

#[test]
fn a_legacy_session_has_its_ide_context_stripped_and_its_rename_as_the_title() {
    let mut w = World::new();
    w.scan();
    let s = w.session(B);
    assert_eq!(
        s.title.as_deref(),
        Some("Docs cleanup"),
        "from session_index.jsonl"
    );
    assert_eq!(s.title_source.as_deref(), Some("custom"));
    assert_eq!(
        s.first_prompt.as_deref(),
        Some("Tidy the docs index and fix broken links")
    );
    assert_eq!((s.prompt_count, s.reply_count), (2, 2));
    assert_eq!(s.cwd.as_deref(), Some("/Users/you/code/docs"));
    assert_eq!(s.git_branch, None, "a legacy session has no git info");
}

#[test]
fn a_rename_changes_the_title_though_the_rollout_did_not_change() {
    let mut w = World::new();
    w.scan();
    assert_eq!(w.session(A).title_source.as_deref(), Some("prompt"));
    append(
        &w.codex.join("session_index.jsonl"),
        &format!(
            "{}\n",
            json!({"id": A, "thread_name": "Retry work", "updated_at": "2026-11-10T08:00:00.000Z"})
        ),
    );
    // a rename touches session_index.jsonl, so the named sessions are read again (A among them)
    assert!(w.scan().files_parsed >= 1);
    assert_eq!(w.session(A).title.as_deref(), Some("Retry work"));
    assert_eq!(w.session(A).title_source.as_deref(), Some("custom"));
    assert_eq!(w.session(A).prompt_count, 2, "nothing was counted twice");
    // taking the name away gives the prompt back
    append(
        &w.codex.join("session_index.jsonl"),
        &format!(
            "{}\n",
            json!({"id": A, "thread_name": "", "updated_at": "2026-11-10T09:00:00.000Z"})
        ),
    );
    assert!(w.scan().files_parsed >= 1);
    assert_eq!(
        w.session(A).title.as_deref(),
        Some("Fix the flaky retry loop, see APP-123")
    );
}

#[test]
fn an_archived_session_is_a_session_too() {
    let mut w = World::new();
    w.scan();
    let s = w.session(F);
    assert!(s
        .transcript_path
        .as_deref()
        .unwrap()
        .contains("archived_sessions/rollout-2026-10-30"));
    assert_eq!(s.project_dir.as_deref(), Some("2026-10"));
    assert_eq!(
        s.first_prompt.as_deref(),
        Some("Port the old billing script to Python 3")
    );
    // a session Codex archives later keeps its row: the file only moves
    let live = w.rollout(A);
    let name = live.file_name().unwrap().to_owned();
    std::fs::create_dir_all(w.codex.join("archived_sessions")).unwrap();
    std::fs::rename(&live, w.codex.join("archived_sessions").join(name)).unwrap();
    w.scan();
    let a = w.session(A);
    assert!(a
        .transcript_path
        .as_deref()
        .unwrap()
        .contains("archived_sessions"));
    assert_eq!(a.transcript_state.as_deref(), Some("present"));
    assert_eq!(
        a.prompt_count, 2,
        "read again from the new place, not added"
    );
}

#[test]
fn search_finds_what_was_said_and_not_what_was_injected() {
    let mut w = World::new();
    w.scan();
    assert_eq!(hits(&w.db, "retry"), [A]);
    assert_eq!(hits(&w.db, "jittered"), [E]);
    assert_eq!(hits(&w.db, "changelog"), [A]);
    assert_eq!(hits(&w.db, "quickstart"), [B]);
    assert_eq!(hits(&w.db, "billing"), [F]);
    assert!(
        hits(&w.db, "linter").is_empty(),
        "AGENTS.md is injected, not said"
    );
    assert!(hits(&w.db, "environment_context").is_empty());
    assert!(
        hits(&w.db, "SECRET-COMPACTION-TEXT").is_empty(),
        "a compaction is not text"
    );
    assert!(hits(&w.db, "summary").is_empty());
    assert!(
        hits(&w.db, "repo").is_empty(),
        "the exec run's words are not indexed"
    );
}

#[test]
fn a_session_that_grows_is_read_on_from_where_it_stopped() {
    let mut w = World::new();
    let path = w.rollout(A);
    let full = std::fs::read_to_string(&path).unwrap();
    let split = full
        .match_indices('\n')
        .map(|(i, _)| i + 1)
        .find(|&i| {
            full[i..].contains("\"turn_id\":\"t2\"")
                && full[i..].starts_with('{')
                && full[i..]
                    .lines()
                    .next()
                    .unwrap()
                    .contains("\"turn_id\":\"t2\"")
        })
        .unwrap();
    let (turn1, turn2) = full.split_at(split);
    // turn 1, and the first bytes of the next line (it is still being written)
    let partial = &turn2[..40];
    std::fs::write(&path, format!("{turn1}{partial}")).unwrap();
    w.scan();
    let s = w.session(A);
    assert_eq!((s.prompt_count, s.reply_count), (1, 2));
    assert_eq!(s.cwd.as_deref(), Some("/Users/you/code/app"));
    let t = s.tokens.unwrap();
    assert_eq!((t.input, t.cache_read, t.output), (4_000, 8_000, 900));
    assert!(hits(&w.db, "added entry").is_empty());
    let cursor = w.db.cursor_get(&path.to_string_lossy()).unwrap().unwrap();
    assert_eq!(
        cursor.marker,
        turn1.len().to_string(),
        "the half line is left for later"
    );

    // the line is finished and turn 2 follows, in another folder
    std::fs::write(&path, &full).unwrap();
    let st = w.scan();
    assert_eq!(st.files_parsed, 1);
    let s = w.session(A);
    assert_eq!(
        (s.prompt_count, s.reply_count),
        (2, 3),
        "nothing counted twice"
    );
    assert_eq!(s.cwd.as_deref(), Some("/Users/you/code/app/server"));
    let t = s.tokens.unwrap();
    assert_eq!(
        (t.input, t.cache_read, t.output),
        (5_000, 15_000, 1_500),
        "the new total replaces the old one, it is not added to it"
    );
    assert_eq!(
        hits(&w.db, "added entry"),
        [A],
        "what turn 2 said is searchable now"
    );
}

#[test]
fn a_session_that_only_gained_lines_does_not_start_again() {
    let mut w = World::new();
    w.scan();
    let path = w.rollout(E);
    let marker_before =
        w.db.cursor_get(&path.to_string_lossy())
            .unwrap()
            .unwrap()
            .marker;
    append(
        &path,
        &format!(
            "{}\n",
            json!({"timestamp": "2026-11-09T18:05:00.000Z", "ordinal": 4, "type": "event_msg",
                   "payload": {"type": "item_completed", "turn_id": "e1",
                               "item": {"type": "AgentMessage", "id": "m9", "phase": "final_answer",
                                        "content": [{"type": "Text", "text": "Jitter added; tests pass."}]}}})
        ),
    );
    w.scan();
    let s = w.session(E);
    assert_eq!((s.prompt_count, s.reply_count), (1, 1));
    let marker_after =
        w.db.cursor_get(&path.to_string_lossy())
            .unwrap()
            .unwrap()
            .marker;
    assert!(marker_after.parse::<u64>().unwrap() > marker_before.parse::<u64>().unwrap());
    assert_eq!(hits(&w.db, "jitter"), [E]);
}

#[test]
fn a_rollout_that_shrank_is_read_again_from_the_start() {
    let mut w = World::new();
    w.scan();
    assert_eq!(hits(&w.db, "added entry"), [A]);
    let path = w.rollout(A);
    let full = std::fs::read_to_string(&path).unwrap();
    let turn1: String = full
        .lines()
        .take_while(|l| !l.contains("\"turn_id\":\"t2\""))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&path, turn1).unwrap();
    w.scan();
    let s = w.session(A);
    assert_eq!((s.prompt_count, s.reply_count), (1, 2));
    assert!(
        hits(&w.db, "added entry").is_empty(),
        "the words that are gone are not found"
    );
    let t = s.tokens.unwrap();
    assert_eq!((t.input, t.output), (4_000, 900));
}

#[test]
fn garbage_and_unknown_lines_do_not_stop_a_read() {
    let mut w = World::empty();
    let dir = w.codex.join("sessions/2026/11/09");
    std::fs::create_dir_all(&dir).unwrap();
    let id = "019a7000-0000-7000-8000-000000000001";
    let text = [
        json!({"timestamp": "2026-11-09T10:00:00.000Z", "type": "session_meta", "payload": {"id": id, "cwd": "/w/x", "source": "cli"}}).to_string(),
        "{not json".to_string(),
        json!({"timestamp": "2026-11-09T10:00:01.000Z", "type": "from_the_future", "payload": {"x": 1}}).to_string(),
        json!({"timestamp": "2026-11-09T10:00:02.000Z", "type": "event_msg", "payload": {"type": "user_message", "message": "still reads this one"}}).to_string(),
        "[1,2,3]".to_string(),
    ]
    .join("\n")
        + "\n";
    std::fs::write(
        dir.join(format!("rollout-2026-11-09T11-00-00-{id}.jsonl")),
        text,
    )
    .unwrap();
    w.scan();
    let s = w.session(id);
    assert_eq!((s.prompt_count, s.cwd.as_deref()), (1, Some("/w/x")));
}

#[test]
fn the_conversation_and_the_summary_tail_are_the_real_exchange() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    let s = w.session(A);
    let agent = agents.of_session(&s).unwrap();
    let all = conversation::read(&s, &w.paths, agent.as_ref()).unwrap();
    let got: Vec<(&str, &str, u32)> = all
        .iter()
        .map(|m| (m.role, m.text.as_str(), m.tools))
        .collect();
    assert_eq!(
        got,
        [
            ("user", "Fix the flaky retry loop, see APP-123", 0),
            (
                "assistant",
                "I'll look at the retry code first.\n\nDone. I opened https://github.com/org/app/pull/42 for review. Want me to also add a changelog entry?",
                4
            ),
            ("user", "Yes, add the changelog entry", 0),
            ("assistant", "Added the entry.", 0),
        ]
    );
    let ex = summarize::build_excerpt(&w.db, agent.as_ref(), &s).unwrap();
    for want in [
        "[user] Fix the flaky retry loop, see APP-123",
        "> $ rg -n retry server",
        "> edit /Users/you/code/app/server/retry.py, /Users/you/code/app/server/tests/test_retry.py",
        "> $ git add -A && git commit -m \"Fix retry loop\"",
        "> error: exit 1 nothing to commit, working tree clean",
        "[assistant] Added the entry.",
        "## Sub-agents\n- Hume (done)",
    ] {
        assert!(ex.text.contains(want), "{want}\n{}", ex.text);
    }
    assert!(!ex.text.contains("AGENTS.md") && !ex.text.contains("environment_context"));
    assert!(ex.system.contains("Codex session"), "{}", ex.system);
    // the compaction is a note in the tail of a session that has one, and its text is not
    let b = w.session(B);
    let ex = summarize::build_excerpt(&w.db, agent.as_ref(), &b).unwrap();
    assert!(ex.text.contains("> context compacted"), "{}", ex.text);
    assert!(!ex.text.contains("SECRET-COMPACTION-TEXT"));
}

#[test]
fn resume_and_fork_run_codex_in_the_last_folder() {
    let w = {
        let mut w = World::new();
        w.scan();
        w
    };
    let agents = w.agents();
    let s = w.session(A);
    assert_eq!(
        agents.resume_line(&s).as_deref(),
        Some(format!("cd '/Users/you/code/app/server' && codex resume {A}").as_str())
    );
    assert_eq!(
        agents.fork_line(&s).as_deref(),
        Some(format!("cd '/Users/you/code/app/server' && codex fork {A}").as_str())
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    let app = w.app();
    let card = rt.block_on(call(&app, "GET", &format!("/api/cards/{A}"), None));
    assert_eq!(card["agent"], "codex");
    assert_eq!(card["agent_name"], "Codex");
    assert_eq!(
        card["resume_cmd"],
        format!("cd '/Users/you/code/app/server' && codex resume {A}")
    );
    let plan = rt.block_on(call(
        &app,
        "POST",
        &format!("/api/cards/{A}/resume"),
        Some(json!({"dry_run": true, "fork": true})),
    ));
    assert_eq!(
        plan["command"],
        format!("cd '/Users/you/code/app/server' && codex fork {A}")
    );
    let agents_api = rt.block_on(call(&app, "GET", "/api/agents", None));
    assert_eq!(
        agents_api,
        json!({"agents": [{"id": "codex", "name": "Codex CLI", "short": "Codex", "default": true}]})
    );
}

#[test]
fn the_archive_keeps_a_rollout_and_puts_it_back_where_it_was() {
    let mut w = World::new();
    w.scan();
    let agents = w.agents();
    let path = w.rollout(A);
    let original = std::fs::read(&path).unwrap();
    let Archived::Copied(bytes) = archive::archive_session(&w.db, &w.paths, &agents, A).unwrap()
    else {
        panic!("not copied");
    };
    assert!(bytes > 0);
    let copy = w
        .paths
        .home
        .join(format!("archive/@codex/2026-11/{A}.jsonl.zst"));
    assert!(copy.is_file(), "{}", copy.display());
    // Codex deletes the session: it is read from the copy, then restored to its own place
    std::fs::remove_file(&path).unwrap();
    w.scan();
    assert_eq!(w.session(A).transcript_state.as_deref(), Some("archived"));
    let s = w.session(A);
    let agent = agents.of_session(&s).unwrap();
    let all = conversation::read(&s, &w.paths, agent.as_ref()).unwrap();
    assert_eq!(all.len(), 4);
    let back = archive::restore(&w.db, &w.paths, &agents, A).unwrap();
    assert_eq!(back, path);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    w.scan();
    assert_eq!(w.session(A).transcript_state.as_deref(), Some("present"));
}

#[test]
fn a_session_is_open_while_codex_holds_its_writer_lock() {
    let mut w = World::new();
    let locks = w.codex.join("thread-writer-locks");
    std::fs::create_dir_all(&locks).unwrap();
    let hold = |id: &str| {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(locks.join(format!("{id}.lock")))
            .unwrap();
        f.try_lock().unwrap();
        f
    };
    w.scan();
    assert_eq!(
        w.session(E).live_status,
        None,
        "no lock held, no live session"
    );
    // E has a turn open (task_started, nothing after it), A's last turn is complete, B is not open
    let (_e, _a) = (hold(E), hold(A));
    w.scan();
    assert_eq!(w.session(E).live_status.as_deref(), Some("busy"));
    assert_eq!(w.session(A).live_status.as_deref(), Some("idle"));
    assert_eq!(w.session(B).live_status, None);
    // the board puts the working one in Running
    let rt = tokio::runtime::Runtime::new().unwrap();
    let board = rt.block_on(call(&w.app(), "GET", "/api/board", None));
    let running: Vec<&str> = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "running")
        .unwrap()["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(running, [E]);
    // a caller that never scanned (`eye mark --current`) sees the open sessions too
    let fresh = w.agents();
    let live = fresh[0].live_sessions();
    let got: Vec<(&str, Option<&str>)> = live
        .iter()
        .map(|l| (l.session_id.as_str(), l.status.as_deref()))
        .collect();
    assert_eq!(got, [(A, Some("idle")), (E, Some("busy"))]);
    // Codex closes them. A test that starts a program at this moment lends it our descriptors until
    // it runs, and a lock lives as long as one of them does, so give it a moment.
    drop((_e, _a));
    for _ in 0..100 {
        w.scan();
        if w.session(E).live_status.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(w.session(E).live_status, None);
}

/// The board's card of a session: its column, whether a prompt blocks it, and the next step.
fn card_of(w: &World, id: &str) -> (String, bool, Option<String>) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let c = rt.block_on(call(&w.app(), "GET", &format!("/api/cards/{id}"), None));
    (
        c["column"].as_str().unwrap().to_string(),
        c["blocked"].as_bool().unwrap(),
        c["next"].as_str().map(String::from),
    )
}

/// One prompt and no outputs is archive material once the session is closed; while Codex holds
/// its lock the session is open and somebody's work in progress.
#[test]
fn an_open_session_with_one_prompt_is_paused_not_archived() {
    let mut w = World::empty();
    let id = "019a7000-0000-7000-8000-0000000000aa";
    let dir = w.codex.join("sessions/2026/11/09");
    std::fs::create_dir_all(&dir).unwrap();
    let line = |t: &str, p: Value| {
        json!({"timestamp": "2026-11-09T10:00:00.000Z", "type": t, "payload": p}).to_string()
    };
    std::fs::write(
        dir.join(format!("rollout-2026-11-09T11-00-00-{id}.jsonl")),
        [
            line(
                "session_meta",
                json!({"id": id, "cwd": "/w/x", "source": "cli"}),
            ),
            line(
                "event_msg",
                json!({"type": "user_message", "message": "Look at the logs"}),
            ),
            line(
                "event_msg",
                json!({"type": "agent_message", "message": "Nothing odd in them."}),
            ),
            line(
                "event_msg",
                json!({"type": "task_complete", "turn_id": "t1"}),
            ),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    w.scan();
    assert_eq!(card_of(&w, id).0, "archive", "closed and trivial");

    let locks = w.codex.join("thread-writer-locks");
    std::fs::create_dir_all(&locks).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(locks.join(format!("{id}.lock")))
        .unwrap();
    held.try_lock().unwrap();
    w.scan();
    assert_eq!(w.session(id).live_status.as_deref(), Some("idle"));
    assert_eq!(card_of(&w, id).0, "paused", "open now");
    drop(held);
}

#[test]
fn a_session_stopped_on_a_permission_prompt_needs_you_until_a_tool_runs() {
    let mut w = World::new();
    let locks = w.codex.join("thread-writer-locks");
    std::fs::create_dir_all(&locks).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(locks.join(format!("{E}.lock")))
        .unwrap();
    held.try_lock().unwrap();
    // what the daemon does on a tick: drain the hook lines, then scan
    let tick = |w: &mut World, event: &str, payload: Value| {
        let agents = w.agents();
        agent_eye::hook::run(
            &w.paths,
            agents[0].as_ref(),
            event,
            payload.to_string().as_bytes(),
        );
        agent_eye::daemon::drain_queue(&w.db, &w.paths, &agents).unwrap();
        w.scan();
    };
    w.scan();
    // the rollout says a turn is open: working
    assert_eq!(w.session(E).live_status.as_deref(), Some("busy"));
    assert_eq!(card_of(&w, E), ("running".into(), false, None));

    // Codex stops on an approval prompt and writes nothing to the rollout: the hook is all there is
    tick(
        &mut w,
        "PermissionRequest",
        json!({"session_id": E, "turn_id": "t4", "tool_name": "Bash",
               "tool_input": {"command": "cargo test --all"}}),
    );
    assert_eq!(w.session(E).live_status.as_deref(), Some("waiting"));
    assert_eq!(
        card_of(&w, E),
        (
            "needs_you".into(),
            true,
            Some("You: allow Bash: cargo test --all".into())
        )
    );
    // another scan with nothing new keeps it so
    w.scan();
    assert_eq!(card_of(&w, E).0, "needs_you");

    // the tool ran, so the prompt was answered: working again
    tick(
        &mut w,
        "PostToolUse",
        json!({"session_id": E, "tool_name": "Bash", "tool_input": {"command": "cargo test --all"},
               "tool_response": {"output": "test result: ok"}}),
    );
    assert_eq!(w.session(E).live_status.as_deref(), Some("busy"));
    assert_eq!(card_of(&w, E), ("running".into(), false, None));
    let s = w.session(E);
    assert!(s.waiting_since.is_none() && !s.waiting_prompt);
    let queued = std::fs::read_to_string(agent_eye::hook::queue_path(&w.paths)).unwrap();
    assert!(
        !queued.contains("test result"),
        "tool output is not stored: {queued}"
    );

    // a second prompt blocks again, and the turn ending clears it
    tick(
        &mut w,
        "PermissionRequest",
        json!({"session_id": E, "tool_name": "apply_patch",
               "tool_input": {"command": "*** Begin Patch\n*** Update File: a.py\n*** End Patch"}}),
    );
    assert_eq!(card_of(&w, E).0, "needs_you");
    tick(
        &mut w,
        "Stop",
        json!({"session_id": E, "last_assistant_message": "Done."}),
    );
    assert_eq!(card_of(&w, E), ("running".into(), false, None));
    drop(held);
}

#[test]
fn an_idea_started_in_codex_is_linked_to_the_session_that_begins_in_its_folder() {
    let mut w = World::new();
    w.scan();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let app = w.app();
    let idea = rt.block_on(call(
        &app,
        "POST",
        "/api/ideas",
        Some(json!({"title": "Jitter", "folder": "/Users/you/code/app", "prompt": "Try a jittered backoff instead"})),
    ));
    assert_eq!(
        (idea["agent"].clone(), idea["agent_name"].clone()),
        (Value::Null, json!("Codex"))
    );
    let id = idea["idea_id"].as_i64().unwrap();
    let plan = rt.block_on(call(
        &app,
        "POST",
        &format!("/api/ideas/{id}/start"),
        Some(json!({"dry_run": true})),
    ));
    assert_eq!(
        plan["command"],
        "cd '/Users/you/code/app' && codex 'Try a jittered backoff instead'"
    );
    assert!(plan["session_id"].is_null(), "Codex picks the id itself");
    let bg = rt.block_on(call(
        &app,
        "POST",
        &format!("/api/ideas/{id}/start"),
        Some(json!({"dry_run": true, "background": true})),
    ));
    assert!(bg["error"].as_str().unwrap().contains("no --bg"), "{bg}");

    // what a start does before the terminal opens, a minute before E began in that folder
    agent_eye::ideas::record_start(&w.db, id, None).unwrap();
    let before_e = agent_eye::util::iso_to_ms("2026-11-09T17:59:00.000Z").unwrap();
    w.db.conn
        .execute(
            "UPDATE meta SET value=?2 WHERE key=?1",
            rusqlite::params![format!("idea_fg:{id}"), before_e.to_string()],
        )
        .unwrap();
    w.scan();
    let linked = agent_eye::ideas::get(&w.db, id).unwrap().unwrap();
    assert_eq!(linked.session_id.as_deref(), Some(E));
    assert_eq!(w.session(E).idea_id, Some(id));
    assert!(agent_eye::ideas::list_open(&w.db).unwrap().is_empty());
}

mod setup {
    use super::*;
    use agent_eye::agents::codex::Codex;
    use agent_eye::agents::{Agent, InstallCtx};
    use std::os::unix::fs::PermissionsExt;

    /// The hooks.json of somebody who already has hooks, and a notify program in config.toml.
    const USERS_HOOKS: &str = r#"{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "/home/you/bin/audit-shell"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/notify/bin/notify stop",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
"#;
    const USERS_CONFIG: &str =
        "model = \"gpt-6\"\nnotify = [\"/opt/notify/bin/notify\", \"codex\"]\n";
    const EYE: &str = "/home/you/.local/bin/agent-eye";

    /// A program that prints what `codex --version` prints.
    fn stub_codex(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let p = dir.join("bin/codex");
        std::fs::write(&p, "#!/bin/sh\necho 'codex-cli 0.167.0'\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn adapter(codex: &Path, exe: &Path) -> Codex {
        Codex::new(codex.to_path_buf()).with_executable(&exe.to_string_lossy())
    }

    fn ctx(dry_run: bool) -> InstallCtx<'static> {
        InstallCtx {
            eye_bin: Path::new(EYE),
            dry_run,
        }
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn hooks_are_merged_beside_the_users_own_and_taken_out_again() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("hooks.json"), USERS_HOOKS).unwrap();
        std::fs::write(codex.join("config.toml"), USERS_CONFIG).unwrap();
        let a = adapter(&codex, &stub_codex(tmp.path()));

        // a dry run says what it would do and writes nothing
        let rep = a.install(&ctx(true)).unwrap().body(true);
        assert!(rep.contains("would add hooks (6 events) to "), "{rep}");
        assert!(
            rep.contains("\"PostToolUse\""),
            "the diff is in a dry run: {rep}"
        );
        assert!(
            rep.contains("Next: start Codex and run /hooks once"),
            "the trust step is said: {rep}"
        );
        assert_eq!(
            std::fs::read_to_string(codex.join("hooks.json")).unwrap(),
            USERS_HOOKS
        );
        assert!(!codex.join("skills").exists());

        let done = a.install(&ctx(false)).unwrap();
        let rep = done.body(false);
        assert!(
            rep.contains("hooks added (6 events) to ") && rep.contains("(backup "),
            "{rep}"
        );
        assert!(
            !rep.contains("PostToolUse"),
            "no diff without asking: {rep}"
        );
        assert_eq!(rep.matches("Next: ").count(), 1, "{rep}");
        assert!(rep.contains("run /hooks once"), "{rep}");
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(codex.join("hooks.json")).unwrap())
                .unwrap();
        let mine = |ev: &str| -> Vec<Value> {
            v["hooks"][ev]
                .as_array()
                .unwrap()
                .iter()
                .filter(|g| g.to_string().contains("agent-eye hook"))
                .cloned()
                .collect()
        };
        for ev in [
            "SessionStart",
            "UserPromptSubmit",
            "PermissionRequest",
            "PostToolUse",
            "Stop",
            "SessionEnd",
        ] {
            let ours = mine(ev);
            assert_eq!(ours.len(), 1, "{ev}");
            assert_eq!(
                ours[0]["hooks"][0]["command"],
                format!("{EYE} hook {ev} --agent codex")
            );
            assert_eq!(ours[0]["hooks"][0]["type"], "command");
        }
        assert!(
            v["hooks"].get("Notification").is_none(),
            "Codex has no Notification hook"
        );
        // the user's own entries are where they were, with their own settings
        assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        assert_eq!(
            v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "/home/you/bin/audit-shell"
        );
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["timeout"], 10);
        // the notify setting is Codex's own and stays as it was, byte for byte
        assert_eq!(
            std::fs::read_to_string(codex.join("config.toml")).unwrap(),
            USERS_CONFIG
        );
        let backups: Vec<String> = files(&codex)
            .into_iter()
            .filter(|n| n.starts_with("hooks.json.eye-backup-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(codex.join(&backups[0])).unwrap(),
            USERS_HOOKS
        );
        let skill = std::fs::read_to_string(codex.join("skills/eye/SKILL.md")).unwrap();
        assert!(skill.starts_with("---\nname: eye\n") && skill.contains("Codex"));

        // a second install changes nothing
        let after = std::fs::read_to_string(codex.join("hooks.json")).unwrap();
        let again = a.install(&ctx(false)).unwrap().body(true);
        assert!(again.contains("hooks ok (6 events), skill ok"), "{again}");
        assert!(!again.contains("Next:"), "nothing new to trust: {again}");
        assert_eq!(
            std::fs::read_to_string(codex.join("hooks.json")).unwrap(),
            after
        );
        assert_eq!(
            files(&codex)
                .iter()
                .filter(|n| n.contains("eye-backup"))
                .count(),
            1
        );

        // the checks see the hooks, and say plainly that trust is not theirs to see
        let checks = a.doctor(&ctx(true));
        let says = |s: &str| checks.iter().any(|c| c.ok && c.msg.contains(s));
        assert!(
            says("hooks present for all 6 events") && says("cannot see"),
            "{checks:?}"
        );
        assert!(says("codex-cli 0.167.0"), "{checks:?}");

        // uninstall takes out eye's and only eye's
        let rep = a.uninstall(&ctx(false)).unwrap().body(true);
        assert!(rep.contains("hooks removed (6 entries) from "), "{rep}");
        assert!(rep.contains("skill removed"), "{rep}");
        let back: Value =
            serde_json::from_str(&std::fs::read_to_string(codex.join("hooks.json")).unwrap())
                .unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(USERS_HOOKS).unwrap());
        assert!(!codex.join("skills/eye").exists());
        assert_eq!(
            std::fs::read_to_string(codex.join("config.toml")).unwrap(),
            USERS_CONFIG
        );
    }

    #[test]
    fn hooks_from_before_they_had_a_name_are_updated_and_trust_is_asked_again() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        let a = adapter(&codex, &stub_codex(tmp.path()));
        a.install(&ctx(false)).unwrap();
        let path = codex.join("hooks.json");
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let events = [
            "SessionStart",
            "UserPromptSubmit",
            "PermissionRequest",
            "PostToolUse",
            "Stop",
            "SessionEnd",
        ];
        for ev in events {
            v["hooks"][ev][0]["hooks"][0]
                .as_object_mut()
                .unwrap()
                .remove("statusMessage");
        }
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let rep = a.install(&ctx(false)).unwrap();
        let body = rep.body(true);
        assert!(body.contains("hooks updated (6 events) in "), "{body}");
        assert_eq!(rep.next.len(), 1, "the /hooks note, once: {body}");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for ev in events {
            assert_eq!(
                v["hooks"][ev].as_array().unwrap().len(),
                1,
                "{ev}: no second entry"
            );
            assert_eq!(
                v["hooks"][ev][0]["hooks"][0]["statusMessage"], "agent-eye",
                "{ev}"
            );
        }
        assert!(a
            .install(&ctx(false))
            .unwrap()
            .body(true)
            .contains("hooks ok (6 events)"));
    }

    #[test]
    fn an_install_from_before_post_tool_use_is_brought_up_to_date() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        let a = adapter(&codex, &stub_codex(tmp.path()));
        a.install(&ctx(false)).unwrap();
        let path = codex.join("hooks.json");
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let entry = v["hooks"]["PostToolUse"][0]["hooks"][0].clone();
        assert_eq!(
            entry,
            json!({"type": "command", "command": format!("{EYE} hook PostToolUse --agent codex"),
                   "statusMessage": "agent-eye", "async": true, "timeout": 5})
        );
        v["hooks"].as_object_mut().unwrap().remove("PostToolUse");
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        // doctor names what is missing, and install adds only that
        let checks = a.doctor(&ctx(true));
        assert!(
            checks
                .iter()
                .any(|c| !c.ok && c.msg.starts_with("hooks missing: PostToolUse (run ")),
            "{checks:?}"
        );
        let rep = a.install(&ctx(false)).unwrap();
        let text = rep.body(true);
        assert!(text.contains("hooks added (PostToolUse) to "), "{text}");
        assert!(
            text.contains("Next: "),
            "a new hook is to be trusted: {text}"
        );
        assert!(a
            .doctor(&ctx(true))
            .iter()
            .any(|c| c.ok && c.msg.contains("all 6 events")));
    }

    #[test]
    fn a_hooks_file_that_is_not_json_is_never_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        let a = adapter(&codex, &stub_codex(tmp.path()));
        for bad in ["{nope", "[]", "{\"hooks\": {\"Stop\": \"x\"}}"] {
            std::fs::write(codex.join("hooks.json"), bad).unwrap();
            assert!(a.preflight(&ctx(false)).is_err(), "{bad}");
            assert!(a.install(&ctx(false)).is_err(), "{bad}");
            assert_eq!(
                std::fs::read_to_string(codex.join("hooks.json")).unwrap(),
                bad
            );
        }
        std::fs::write(codex.join("hooks.json"), "{nope").unwrap();
        let checks = a.doctor(&ctx(true));
        assert!(
            checks
                .iter()
                .any(|c| !c.ok && c.msg.starts_with("hooks.json:")),
            "{checks:?}"
        );
    }

    #[test]
    fn nothing_is_written_for_a_machine_without_codex() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("no-codex");
        let a = Codex::new(codex.clone()).with_executable("/nonexistent/codex");
        let rep = a.install(&ctx(false)).unwrap().body(true);
        assert!(rep.contains("codex   not found (no "), "{rep}");
        assert!(!rep.contains("Next:"), "nothing to trust: {rep}");
        assert!(
            !codex.exists(),
            "no folder is made for an agent that is not there"
        );
        assert!(
            a.uninstall(&ctx(false)).unwrap().is_empty(),
            "nothing to say either"
        );
        let checks = a.doctor(&ctx(true));
        assert!(
            checks.len() == 1 && !checks[0].ok && checks[0].msg.contains("Codex CLI not found")
        );
    }

    #[test]
    fn the_hook_command_the_install_writes_reaches_the_codex_adapter() {
        let mut w = World::new();
        w.scan();
        let home = w.paths.home.clone();
        let rollout = w.rollout(A);
        let q = tmp_queue(&w);
        // what Codex sends a hook: the thread id as session_id, the rollout as transcript_path
        let run = |event: &str, payload: Value| {
            let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_agent-eye"));
            c.args(["hook", event, "--agent", "codex"])
                .env("EYE_HOME", &home)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped());
            let mut child = c.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.to_string().as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(
                out.status.success() && out.stdout.is_empty(),
                "a hook prints nothing"
            );
        };
        run(
            "PermissionRequest",
            json!({"session_id": A, "turn_id": "t3", "transcript_path": rollout, "cwd": "/Users/you/code/app/server",
                   "hook_event_name": "PermissionRequest", "model": "gpt-6", "permission_mode": "default",
                   "tool_name": "Bash", "tool_input": {"command": "cargo test --all", "description": "run the tests"}}),
        );
        let lines: Vec<Value> = std::fs::read_to_string(&q)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["agent"], "codex");
        assert_eq!(lines[0]["event"], "PermissionRequest");
        assert_eq!(lines[0]["session_id"], A);
        assert_eq!(lines[0]["tool_detail"], "cargo test --all");

        // the daemon applies it: the session blocks on the request, a question waits, a prompt clears
        let drain = |w: &World| {
            agent_eye::daemon::drain_queue(&w.db, &w.paths, &w.agents()).unwrap();
        };
        drain(&w);
        let s = w.session(A);
        assert_eq!(
            s.waiting_note.as_deref(),
            Some("allow Bash: cargo test --all")
        );
        assert!(s.waiting_prompt && s.waiting_since.is_some());
        run(
            "Stop",
            json!({"session_id": A, "turn_id": "t3", "hook_event_name": "Stop",
                   "last_assistant_message": "Tests pass. Shall I open the PR?"}),
        );
        drain(&w);
        let s = w.session(A);
        assert_eq!(
            s.waiting_note.as_deref(),
            Some("answer: Shall I open the PR?")
        );
        assert!(!s.waiting_prompt);
        run(
            "UserPromptSubmit",
            json!({"session_id": A, "hook_event_name": "UserPromptSubmit", "prompt": "yes"}),
        );
        drain(&w);
        let s = w.session(A);
        assert_eq!((s.waiting_since, s.waiting_note), (None, None));
    }

    fn tmp_queue(w: &World) -> PathBuf {
        agent_eye::hook::queue_path(&w.paths)
    }
}

mod summaries {
    use super::*;
    use agent_eye::agents::codex::Codex;
    use agent_eye::agents::{Agent, HeadlessRequest};
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::time::Duration;

    /// A `codex` that knows `exec --help`, records how it was run and answers the way a summary
    /// is answered, into the file it is given with `-o`.
    fn stub(dir: &Path, help: &str, body: &str) -> PathBuf {
        let p = dir.join("codex");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nd=\"$(dirname \"$0\")\"\n\
                 if [ \"$1\" = exec ] && [ \"$2\" = --help ]; then echo '{help}'; exit 0; fi\n\
                 printf '%s\\n' \"$@\" > \"$d/args.log\"\ncat > \"$d/stdin.log\"\npwd > \"$d/cwd.log\"\n\
                 while [ $# -gt 0 ]; do case \"$1\" in -o) out=\"$2\"; shift ;; --output-schema) schema=\"$2\"; shift ;; esac; shift; done\n\
                 cp \"$schema\" \"$d/schema.log\"\n{body}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    const HELP: &str = "Usage: codex exec [OPTIONS] [PROMPT] --ephemeral --ignore-user-config --ignore-rules --skip-git-repo-check --output-schema <FILE> -o, --output-last-message <FILE>";
    const ANSWER: &str = r#"printf '%s' '{"status":"needs_you","state":"Retry loop fixed, PR open","next":"You: review the pull request","topics":[]}' > "$out""#;

    fn request<'a>(work: &'a Path, schema: &'a Value, model: &'a str) -> HeadlessRequest<'a> {
        HeadlessRequest {
            model,
            system: "You report where the work stands.",
            input: "# Session\nthe excerpt",
            schema,
            work_dir: work,
            timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn a_summary_is_codex_exec_in_a_read_only_sandbox_that_keeps_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = stub(tmp.path(), HELP, ANSWER);
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let c = Codex::new(tmp.path().join("codex-home")).with_executable(&exe.to_string_lossy());
        let schema = summarize::schema();
        let reply = c.ask(&request(&work, &schema, "gpt-6-luna")).unwrap();
        assert_eq!(reply.output["status"], "needs_you");
        assert_eq!(reply.output["next"], "You: review the pull request");

        let args = std::fs::read_to_string(tmp.path().join("args.log")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(
            args[..11],
            [
                "exec",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--skip-git-repo-check",
                "-s",
                "read-only",
                "-m",
                "gpt-6-luna",
                "-c",
                "model_reasoning_effort=low"
            ]
        );
        assert_eq!(args[11], "--output-schema");
        assert_eq!((args[13], args[15]), ("-o", "-"), "the prompt is on stdin");
        let stdin = std::fs::read_to_string(tmp.path().join("stdin.log")).unwrap();
        assert!(
            stdin.contains("You report where the work stands.") && stdin.contains("the excerpt")
        );
        // it ran in a folder of its own below the work folder, which is gone again
        let cwd = std::fs::read_to_string(tmp.path().join("cwd.log")).unwrap();
        let cwd = std::fs::canonicalize(cwd.trim()).unwrap_or_else(|_| PathBuf::from(cwd.trim()));
        assert!(
            cwd.starts_with(std::fs::canonicalize(&work).unwrap()),
            "{}",
            cwd.display()
        );
        assert_eq!(
            std::fs::read_dir(&work).unwrap().count(),
            0,
            "nothing left behind"
        );
        // the schema it was given is the strict one
        let given: Value =
            serde_json::from_str(&std::fs::read_to_string(tmp.path().join("schema.log")).unwrap())
                .unwrap();
        assert_eq!(given["additionalProperties"], false);
        assert_eq!(
            given["required"],
            json!(["status", "state", "next", "topics"])
        );
        assert!(!given.to_string().contains("maxLength"));
    }

    #[test]
    fn an_old_codex_that_lacks_a_flag_is_told_so_before_anything_is_run() {
        let tmp = tempfile::tempdir().unwrap();
        let old = HELP.replace("--ignore-rules ", "");
        let exe = stub(tmp.path(), &old, ANSWER);
        let work = tmp.path().join("work");
        let c = Codex::new(tmp.path().join("h")).with_executable(&exe.to_string_lossy());
        let schema = summarize::schema();
        let e = c
            .ask(&request(&work, &schema, "m"))
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("no --ignore-rules") && e.contains("update Codex"),
            "{e}"
        );
        assert!(
            !tmp.path().join("args.log").exists(),
            "exec itself was not run"
        );
        // and doctor says it too
        let ctx = agent_eye::agents::InstallCtx {
            eye_bin: Path::new("/x/agent-eye"),
            dry_run: true,
        };
        let checks = c.doctor(&ctx);
        assert!(
            checks.iter().any(|k| !k.ok
                && k.msg.starts_with("summaries:")
                && k.msg.contains("--ignore-rules")),
            "{checks:?}"
        );
    }

    #[test]
    fn a_failed_run_a_wrong_answer_and_a_slow_codex_are_errors_with_a_reason() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        let schema = summarize::schema();
        let ask = |body: &str, timeout: Duration| {
            let exe = stub(tmp.path(), HELP, body);
            let c = Codex::new(tmp.path().join("h")).with_executable(&exe.to_string_lossy());
            let mut req = request(&work, &schema, "m");
            req.timeout = timeout;
            c.ask(&req).unwrap_err().to_string()
        };
        let long = Duration::from_secs(30);
        let e = ask("echo 'not logged in' >&2; exit 3", long);
        assert!(
            e.contains("exited with") && e.contains("not logged in"),
            "{e}"
        );
        let e = ask("printf '%s' 'Sorry, no.' > \"$out\"", long);
        assert!(
            e.contains("did not answer with JSON") && e.contains("Sorry"),
            "{e}"
        );
        let e = ask("exit 0", long);
        assert!(e.contains("wrote no answer"), "{e}");
        let e = ask("exec sleep 5", Duration::from_millis(300));
        assert!(e.contains("timed out"), "{e}");
        let e = Codex::new(tmp.path().join("h"))
            .with_executable("/nonexistent/codex")
            .ask(&request(&work, &schema, "m"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("starting"), "{e}");
    }

    #[test]
    fn summaries_by_codex_are_written_and_use_its_own_cheap_model() {
        let tmp = tempfile::tempdir().unwrap();
        let w = World::new();
        let home = w.paths.home.clone();
        stub(tmp.path(), HELP, ANSWER);
        let eye = |args: &[&str]| {
            let out = Command::new(env!("CARGO_BIN_EXE_agent-eye"))
                .args(args)
                .env("EYE_HOME", &home)
                // the stub is the `codex` on PATH, found the way a real one is
                .env("PATH", format!("{}:/usr/bin:/bin", tmp.path().display()))
                .env_remove("EYE_CODEX_BIN")
                .env_remove("CODEX_HOME")
                .env("NO_COLOR", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        std::fs::write(
            home.join("config.toml"),
            format!(
                "[agents]\nenabled = [\"codex\"]\n[agents.codex]\ndir = \"{}\"\n[summary]\nagent = \"codex\"\n",
                w.codex.display()
            ),
        )
        .unwrap();
        eye(&["scan"]);
        let out = eye(&["summarize", A, "--json"]);
        let done: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(done[0]["result"], "summarized", "{out}");
        let args = std::fs::read_to_string(tmp.path().join("args.log")).unwrap();
        assert!(
            args.contains("-m\ngpt-6-luna\n"),
            "no model set: Codex's own default\n{args}"
        );
        let db = Db::open(&home.join("eye.db")).unwrap();
        let s = db.session_by_id(A).unwrap().unwrap();
        assert_eq!(s.state_line.as_deref(), Some("Retry loop fixed, PR open"));
        assert_eq!(s.ai_status.as_deref(), Some("needs_you"));
        let payload: String = db
            .conn
            .query_row("SELECT payload FROM event WHERE kind='summary'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(payload.contains("\"model\":\"gpt-6-luna\""), "{payload}");
        // a model in the config is used as it is
        let cfg = std::fs::read_to_string(home.join("config.toml")).unwrap();
        std::fs::write(
            home.join("config.toml"),
            cfg.replace(
                "agent = \"codex\"",
                "agent = \"codex\"\nmodel = \"gpt-6-terra\"",
            ),
        )
        .unwrap();
        eye(&["summarize", A, "--force", "--json"]);
        let args = std::fs::read_to_string(tmp.path().join("args.log")).unwrap();
        assert!(args.contains("-m\ngpt-6-terra\n"), "{args}");
    }
}

mod install_cli {
    use super::*;
    use std::process::{Command, Output, Stdio};

    /// A machine: a home folder, a place for eye's program and one for the service file.
    struct Machine {
        tmp: tempfile::TempDir,
    }

    impl Machine {
        fn new() -> Machine {
            let m = Machine {
                tmp: tempfile::tempdir().unwrap(),
            };
            std::fs::create_dir_all(m.home()).unwrap();
            m
        }

        fn home(&self) -> PathBuf {
            self.tmp.path().join("home")
        }

        fn eye(&self, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_agent-eye"))
                .env("HOME", self.home())
                .env_remove("EYE_HOME")
                .env_remove("EYE_CLAUDE_DIR")
                .env_remove("EYE_CLAUDE_BIN")
                .env_remove("EYE_CODEX_BIN")
                .env_remove("CODEX_HOME")
                // no agent program on PATH: an agent is there when its folder is
                .env("PATH", "/usr/bin:/bin")
                .env("NO_COLOR", "1")
                .args(args)
                .stdin(Stdio::null())
                .output()
                .unwrap()
        }

        fn setup(&self, cmd: &str, extra: &[&str]) -> String {
            let bin = self.tmp.path().join("bin");
            let svc = self.tmp.path().join("svc");
            let mut args = vec![
                cmd,
                "--no-service",
                "--bin-dir",
                bin.to_str().unwrap(),
                "--service-dir",
                svc.to_str().unwrap(),
            ];
            args.extend(extra);
            let o = self.eye(&args);
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            assert!(o.status.success(), "{text}");
            text
        }

        fn entries(&self) -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(self.home())
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }

        fn config(&self) -> String {
            std::fs::read_to_string(self.home().join(".agent-eye/config.toml")).unwrap_or_default()
        }
    }

    #[test]
    fn a_machine_with_only_codex_gets_a_working_install_and_no_claude_folder() {
        let m = Machine::new();
        std::fs::create_dir_all(m.home().join(".codex")).unwrap();
        let out = m.setup("install", &[]);
        assert!(
            out.contains("agents  Codex CLI (enabled in ~/.agent-eye/config.toml)"),
            "{out}"
        );
        assert!(
            out.contains("codex   hooks added (6 events) to ~/.codex/hooks.json, skill added"),
            "{out}"
        );
        assert_eq!(
            out.matches("Next: start Codex and run /hooks once").count(),
            1,
            "the trust step is said once: {out}"
        );
        assert!(!out.to_lowercase().contains("claude"), "{out}");
        assert_eq!(
            m.entries(),
            [".agent-eye", ".codex"],
            "no ~/.claude is made"
        );
        assert!(
            m.config().contains("enabled = [\"codex\"]"),
            "{}",
            m.config()
        );
        let hooks = std::fs::read_to_string(m.home().join(".codex/hooks.json")).unwrap();
        assert!(
            hooks.contains("agent-eye hook Stop --agent codex"),
            "{hooks}"
        );
        assert!(m.home().join(".codex/skills/eye/SKILL.md").is_file());
        assert!(
            m.tmp.path().join("svc").read_dir().unwrap().count() == 1,
            "the service file"
        );
        // the summaries are written by the agent that is there, and the checks name only it
        let doctor = m.eye(&["doctor"]);
        let text = String::from_utf8_lossy(&doctor.stdout).into_owned();
        assert!(text.contains("agent: Codex CLI (codex)"), "{text}");
        assert!(
            text.contains("summaries are written by Codex CLI"),
            "{text}"
        );
        assert!(text.contains("hooks present for all 6 events"), "{text}");
        assert!(!text.contains("Claude Code"), "{text}");
        // and it goes away again
        let out = m.setup("uninstall", &[]);
        assert!(out.contains("hooks removed (6 entries) from "), "{out}");
        assert!(!out.to_lowercase().contains("claude"), "{out}");
        assert!(!m.home().join(".codex/skills/eye").exists());
        let hooks: Value = serde_json::from_str(
            &std::fs::read_to_string(m.home().join(".codex/hooks.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(hooks, json!({}));
    }

    #[test]
    fn a_dry_run_shows_the_choice_and_writes_nothing() {
        let m = Machine::new();
        std::fs::create_dir_all(m.home().join(".codex")).unwrap();
        let out = m.setup("install", &["--dry-run"]);
        assert!(out.contains("dry run: nothing was changed"), "{out}");
        assert!(
            out.contains("agents  Codex CLI (would enable it in ~/.agent-eye/config.toml)"),
            "{out}"
        );
        assert!(out.contains("would add hooks (6 events)"), "{out}");
        assert_eq!(m.entries(), [".codex"]);
        assert_eq!(
            std::fs::read_dir(m.home().join(".codex")).unwrap().count(),
            0
        );
    }

    #[test]
    fn a_machine_with_both_follows_both() {
        let m = Machine::new();
        std::fs::create_dir_all(m.home().join(".codex")).unwrap();
        std::fs::create_dir_all(m.home().join(".claude")).unwrap();
        let out = m.setup("install", &[]);
        assert!(
            out.contains(
                "agents  Claude Code, Codex CLI (both enabled in ~/.agent-eye/config.toml)"
            ),
            "{out}"
        );
        assert!(
            m.config().contains("enabled = [\"claude\", \"codex\"]"),
            "{}",
            m.config()
        );
        let claude = std::fs::read_to_string(m.home().join(".claude/settings.json")).unwrap();
        assert!(
            claude.contains("agent-eye hook Stop\""),
            "Claude's own form: {claude}"
        );
        assert!(!claude.contains("--agent"));
        let codex = std::fs::read_to_string(m.home().join(".codex/hooks.json")).unwrap();
        assert!(codex.contains("agent-eye hook Stop --agent codex"));
    }

    #[test]
    fn a_machine_with_only_claude_is_installed_as_before() {
        let m = Machine::new();
        std::fs::create_dir_all(m.home().join(".claude")).unwrap();
        let out = m.setup("install", &[]);
        assert!(
            out.contains("agents  Claude Code (enabled in ~/.agent-eye/config.toml)"),
            "{out}"
        );
        assert!(!out.contains("codex"), "nothing about Codex: {out}");
        assert_eq!(
            m.entries(),
            [".agent-eye", ".claude"],
            "no ~/.codex is made"
        );
        assert!(m.home().join(".claude/skills/eye/SKILL.md").is_file());
        let codex = m.setup("uninstall", &[]);
        assert!(
            !codex.contains("codex") && !codex.contains("Codex"),
            "{codex}"
        );
    }

    #[test]
    fn an_existing_config_keeps_its_agents_and_hears_about_the_other_one() {
        let m = Machine::new();
        std::fs::create_dir_all(m.home().join(".claude")).unwrap();
        std::fs::create_dir_all(m.home().join(".agent-eye")).unwrap();
        let mine = "[agents]\nenabled = [\"claude\"]\n\n[summary]\nmodel = \"haiku\"\n";
        std::fs::write(m.home().join(".agent-eye/config.toml"), mine).unwrap();
        // Claude only: nothing to say, nothing changed
        let out = m.setup("install", &[]);
        assert!(
            out.contains("agents  Claude Code (from ~/.agent-eye/config.toml)\n"),
            "{out}"
        );
        assert!(!out.contains("Codex"), "{out}");
        assert_eq!(m.config(), mine);
        // Codex arrives later: it is mentioned, not switched on
        std::fs::create_dir_all(m.home().join(".codex")).unwrap();
        let out = m.setup("install", &[]);
        assert!(
            out.contains(
                "Codex CLI found but not followed: add \"codex\" to [agents] enabled to follow it"
            ),
            "{out}"
        );
        assert_eq!(m.config(), mine);
        assert!(
            !m.home().join(".codex/hooks.json").exists(),
            "no hooks for an agent that is off"
        );
    }
}
