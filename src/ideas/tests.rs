use super::*;
use crate::agents::claude::Claude;
use std::sync::Arc;

fn claude_only() -> Agents {
    Agents::from_vec(vec![Arc::new(Claude::new("/nonexistent/c".into()))])
}

fn new<'a>(title: &'a str, folder: Option<&'a str>) -> NewIdea<'a> {
    NewIdea {
        title,
        folder,
        ..NewIdea::default()
    }
}

#[test]
fn add_list_plan() {
    let db = Db::open_memory().unwrap();
    let agents = claude_only();
    assert!(add(&db, &new("  ", None)).is_err());
    let i = add(
        &db,
        &NewIdea {
            notes: Some("notes here"),
            prompt: Some("do it"),
            ..new("Try it", Some("/tmp/x y"))
        },
    )
    .unwrap();
    assert_eq!(list_open(&db).unwrap().len(), 1);
    let p = plan_start(&agents, &i, false).unwrap();
    let id = p.session_id.clone().unwrap();
    assert_eq!(id.len(), 36);
    assert!(p
        .command()
        .starts_with("cd '/tmp/x y' && claude --session-id "));
    // the prompt is sent, the notes never are
    assert!(p.command().ends_with("-n 'Try it' 'do it'"));
    assert!(!p.command().contains("notes here"));
    let b = plan_start(&agents, &i, true).unwrap();
    assert!(b.session_id.is_none() && b.command().contains("--bg"));
    // without a prompt the agent opens with nothing sent; the background needs one
    let quiet = add(
        &db,
        &NewIdea {
            notes: Some("just notes"),
            ..new("Quiet", Some("/tmp/x"))
        },
    )
    .unwrap();
    assert!(plan_start(&agents, &quiet, false)
        .unwrap()
        .command()
        .ends_with("-n Quiet"));
    assert!(plan_start(&agents, &quiet, true).is_err());
    let j = add(&db, &new("No folder", None)).unwrap();
    assert!(plan_start(&agents, &j, false).is_err());
    archive(&db, i.id).unwrap();
    assert_eq!(list_open(&db).unwrap().len(), 2);
}

#[test]
fn an_idea_starts_in_the_agent_it_names() {
    let db = Db::open_memory().unwrap();
    let agents = claude_only();
    let i = add(&db, &new("Try it", Some("/tmp/x"))).unwrap();
    assert_eq!(i.agent, None);
    assert_eq!(agent_of(&agents, &i).unwrap().id(), "claude");
    let named = add(
        &db,
        &NewIdea {
            agent: Some("claude"),
            ..new("Named", Some("/tmp/x"))
        },
    )
    .unwrap();
    assert_eq!(named.agent.as_deref(), Some("claude"));
    let off = add(
        &db,
        &NewIdea {
            agent: Some("elsewhere"),
            ..new("Off", Some("/tmp/x"))
        },
    )
    .unwrap();
    assert!(plan_start(&agents, &off, false)
        .unwrap_err()
        .to_string()
        .contains("not enabled"));
    // no agent enabled: nothing to start in
    assert!(plan_start(&Agents::default(), &i, false).is_err());
}

#[test]
fn start_dry_run_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::with_claude_dir(&tmp.path().join("c"));
    let db = Db::open_memory().unwrap();
    let folder = tmp.path().to_string_lossy().into_owned();
    let i = add(
        &db,
        &NewIdea {
            prompt: Some("go"),
            ..new("Try it", Some(&folder))
        },
    )
    .unwrap();
    let paths = Paths {
        home: tmp.path().join("eye"),
    };
    let dry = |background| StartOpts {
        background,
        dry_run: true,
        ..StartOpts::default()
    };
    let st = start(&db, &cfg, &paths, i.id, &dry(false)).unwrap();
    assert!(st.dry_run && st.command.contains("--session-id"));
    assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
    let bg = start(&db, &cfg, &paths, i.id, &dry(true)).unwrap();
    assert!(bg.command.contains("claude --bg") && bg.session_id.is_none());
}

#[test]
fn background_job_takes_over_the_idea() {
    let tmp = tempfile::tempdir().unwrap();
    let claude_dir = tmp.path().join("c");
    std::fs::create_dir_all(claude_dir.join("jobs/30194328")).unwrap();
    std::fs::write(
        claude_dir.join("jobs/30194328/state.json"),
        r#"{"state":"done","sessionId":"30194328-e28b-49bc-8d12-e13b2b0670db"}"#,
    )
    .unwrap();
    let agents = Agents::from_vec(vec![Arc::new(Claude::new(claude_dir))]);
    let db = Db::open_memory().unwrap();
    let i = add(
        &db,
        &NewIdea {
            prompt: Some("go"),
            ..new("Try it", Some("/tmp/x"))
        },
    )
    .unwrap();
    crate::db::meta_set(&db.conn, &format!("idea_bg:{}", i.id), "30194328").unwrap();
    db.conn
        .execute(
            "INSERT INTO session(id) VALUES('30194328-e28b-49bc-8d12-e13b2b0670db')",
            [],
        )
        .unwrap();
    link_sessions(&db.conn, &agents).unwrap();
    let i = get(&db, i.id).unwrap().unwrap();
    assert_eq!(
        i.session_id.as_deref(),
        Some("30194328-e28b-49bc-8d12-e13b2b0670db")
    );
    assert!(list_open(&db).unwrap().is_empty());
    let linked: Option<i64> = db
        .conn
        .query_row("SELECT idea_id FROM session", [], |r| r.get(0))
        .unwrap();
    assert_eq!(linked, Some(i.id));
}

/// A session row as the scanner leaves it: its agent, folder and start time.
fn session(db: &Db, id: &str, agent: &str, cwd: &str, started: i64) {
    session_with(db, id, agent, cwd, started, "go");
}

fn session_with(db: &Db, id: &str, agent: &str, cwd: &str, started: i64, first: &str) {
    db.conn
        .execute(
            "INSERT INTO session(id, agent, cwd, started_at, first_prompt) VALUES(?1,?2,?3,?4,?5)",
            params![id, agent, cwd, started, first],
        )
        .unwrap();
}

fn codex_only() -> Agents {
    Agents::from_vec(vec![Arc::new(crate::agents::codex::Codex::new(
        "/nonexistent/codex".into(),
    ))])
}

fn pending(db: &Db, id: i64) -> bool {
    db.meta_get(&format!("idea_fg:{id}")).unwrap().is_some()
}

#[test]
fn a_start_the_agent_names_no_session_for_links_to_the_next_session_in_the_folder() {
    let db = Db::open_memory().unwrap();
    let agents = codex_only();
    let i = add(
        &db,
        &NewIdea {
            prompt: Some("go"),
            ..new("Try it", Some("/work/p/"))
        },
    )
    .unwrap();
    let plan = plan_start(&agents, &i, false).unwrap();
    assert!(plan.session_id.is_none());
    record_start(&db, i.id, plan.session_id.as_deref()).unwrap();
    assert!(pending(&db, i.id) && get(&db, i.id).unwrap().unwrap().session_id.is_none());
    let t0: i64 = db
        .meta_get(&format!("idea_fg:{}", i.id))
        .unwrap()
        .unwrap()
        .parse()
        .unwrap();
    // sessions that do not belong to it: older, in another folder, of another agent
    session(&db, "old-session", "codex", "/work/p", t0 - 5_000);
    session(&db, "elsewhere", "codex", "/work/other", t0 + 1_000);
    session(&db, "claude-one", "claude", "/work/p", t0 + 1_000);
    link_sessions(&db.conn, &agents).unwrap();
    assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
    assert!(pending(&db, i.id), "still waiting for its session");
    // the session it started, then a later one in the same folder
    session(&db, "the-one", "codex", "/work/p", t0 + 2_000);
    session(&db, "a-later-one", "codex", "/work/p", t0 + 90_000);
    link_sessions(&db.conn, &agents).unwrap();
    assert_eq!(
        get(&db, i.id).unwrap().unwrap().session_id.as_deref(),
        Some("the-one")
    );
    assert!(!pending(&db, i.id));
    let linked: Option<i64> = db
        .conn
        .query_row("SELECT idea_id FROM session WHERE id='the-one'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(linked, Some(i.id));
    // a second idea in the same folder does not take the session of the first
    let j = add(
        &db,
        &NewIdea {
            prompt: Some("go"),
            ..new("Again", Some("/work/p"))
        },
    )
    .unwrap();
    record_start(&db, j.id, None).unwrap();
    db.conn
        .execute(
            "UPDATE meta SET value=?2 WHERE key=?1",
            params![format!("idea_fg:{}", j.id), t0.to_string()],
        )
        .unwrap();
    link_sessions(&db.conn, &agents).unwrap();
    assert_eq!(
        get(&db, j.id).unwrap().unwrap().session_id.as_deref(),
        Some("a-later-one")
    );
}

#[test]
fn a_session_of_another_terminal_in_the_folder_does_not_take_the_link() {
    let db = Db::open_memory().unwrap();
    let agents = codex_only();
    let i = add(
        &db,
        &NewIdea {
            prompt: Some("Try a jittered backoff"),
            ..new("Jitter", Some("/work/p"))
        },
    )
    .unwrap();
    record_start(&db, i.id, None).unwrap();
    let t0: i64 = db
        .meta_get(&format!("idea_fg:{}", i.id))
        .unwrap()
        .unwrap()
        .parse()
        .unwrap();
    session_with(
        &db,
        "somebody-else",
        "codex",
        "/work/p",
        t0 + 1_000,
        "Fix the build",
    );
    link_sessions(&db.conn, &agents).unwrap();
    assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
    assert!(pending(&db, i.id));
    session_with(
        &db,
        "the-one",
        "codex",
        "/work/p",
        t0 + 5_000,
        "Try a jittered backoff",
    );
    link_sessions(&db.conn, &agents).unwrap();
    assert_eq!(
        get(&db, i.id).unwrap().unwrap().session_id.as_deref(),
        Some("the-one")
    );
    // a prompt eye prefixed with `Task: ` (one word, or one that starts with a dash) still matches
    assert!(starts_with_prompt("Task: review", "review"));
    assert!(starts_with_prompt("  Try it  ", "Try it"));
    assert!(!starts_with_prompt("Try it again", "Try it"));
    let long = "x".repeat(500);
    assert!(
        starts_with_prompt(&long, &long),
        "a long prompt is compared by its start"
    );
}

#[test]
fn a_start_that_never_got_a_session_is_let_go_or_taken_back() {
    let db = Db::open_memory().unwrap();
    let agents = codex_only();
    let i = add(
        &db,
        &NewIdea {
            prompt: Some("go"),
            ..new("Stale", Some("/work/p"))
        },
    )
    .unwrap();
    record_start(&db, i.id, None).unwrap();
    // an hour later nobody typed into it
    let long_ago = (now_ms() - LINK_WINDOW_MS - 1_000).to_string();
    db.conn
        .execute(
            "UPDATE meta SET value=?2 WHERE key=?1",
            params![format!("idea_fg:{}", i.id), long_ago],
        )
        .unwrap();
    link_sessions(&db.conn, &agents).unwrap();
    assert!(!pending(&db, i.id));
    assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
    // a launch that failed is taken back
    record_start(&db, i.id, None).unwrap();
    undo_start(&db, i.id).unwrap();
    assert!(!pending(&db, i.id));
    // an idea that was archived or deleted leaves nothing behind
    record_start(&db, i.id, None).unwrap();
    archive(&db, i.id).unwrap();
    link_sessions(&db.conn, &agents).unwrap();
    assert!(!pending(&db, i.id));
    // an agent that lets eye choose the id never waits
    let c = add(
        &db,
        &NewIdea {
            prompt: Some("go"),
            ..new("Claude", Some("/work/p"))
        },
    )
    .unwrap();
    let claude = claude_only();
    let plan = plan_start(&claude, &c, false).unwrap();
    record_start(&db, c.id, plan.session_id.as_deref()).unwrap();
    assert!(!pending(&db, c.id));
    assert_eq!(get(&db, c.id).unwrap().unwrap().session_id, plan.session_id);
}

#[test]
fn edit_changes_only_what_is_given() {
    let db = Db::open_memory().unwrap();
    let i = add(
        &db,
        &NewIdea {
            notes: Some("mine"),
            ..new("Try it", Some("/tmp/x"))
        },
    )
    .unwrap();
    let e = |f: fn(&mut Edit)| {
        let mut e = Edit::default();
        f(&mut e);
        e
    };
    let i2 = edit(&db, i.id, &e(|e| e.prompt = Some(" do it ".into()))).unwrap();
    assert_eq!(
        (
            i2.title.as_str(),
            i2.notes.as_deref(),
            i2.prompt.as_deref(),
            i2.folder.as_deref()
        ),
        ("Try it", Some("mine"), Some("do it"), Some("/tmp/x"))
    );
    let i3 = edit(&db, i.id, &e(|e| e.notes = Some("  ".into()))).unwrap();
    assert_eq!((i3.notes, i3.prompt.as_deref()), (None, Some("do it")));
    let i4 = edit(&db, i.id, &e(|e| e.agent = Some("claude".into()))).unwrap();
    assert_eq!(i4.agent.as_deref(), Some("claude"));
    let i5 = edit(&db, i.id, &e(|e| e.agent = Some(String::new()))).unwrap();
    assert_eq!(i5.agent, None);
    assert!(edit(&db, i.id, &e(|e| e.title = Some(" ".into()))).is_err());
    assert!(edit(&db, 99, &Edit::default()).is_err());
}

#[test]
fn a_start_whose_session_never_came_stays_an_idea() {
    let db = Db::open_memory().unwrap();
    let i = add(&db, &new("Try it", Some("/tmp/x"))).unwrap();
    set_session(&db, i.id, Some("sid-1")).unwrap();
    let i = get(&db, i.id).unwrap().unwrap();
    assert_eq!(list_open(&db).unwrap().len(), 1);
    assert_eq!(started_session(&db, &i).unwrap(), None);
    // starting again reuses the id, so a late first message still lands on the idea
    assert_eq!(
        plan_start(&claude_only(), &i, false)
            .unwrap()
            .session_id
            .as_deref(),
        Some("sid-1")
    );
    db.conn
        .execute("INSERT INTO session(id) VALUES('sid-1')", [])
        .unwrap();
    assert!(list_open(&db).unwrap().is_empty());
    assert_eq!(started_session(&db, &i).unwrap().as_deref(), Some("sid-1"));
}

#[test]
fn start_plan_survives_nasty_strings() {
    let nasty = "it's \"q\" \\ $(touch /tmp/pwn) `id`\nline2";
    let mut idea = Idea {
        id: 1,
        title: nasty.into(),
        prompt: Some("--dangerously-skip-permissions $(id)".into()),
        folder: Some(format!("/{nasty}")),
        session_id: None,
        ..Default::default()
    };
    let agents = claude_only();
    let p = plan_start(&agents, &idea, true).unwrap();
    assert!(p
        .command()
        .contains("'Task: --dangerously-skip-permissions $(id)'"));
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(
            p.command()
                .replace("cd ", "printf '%s\\n' ")
                .replace("&& claude", "; printf '%s\\n' claude"),
        )
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with(&format!("/{nasty}")));
    idea.prompt = None;
    assert!(plan_start(&agents, &idea, false)
        .unwrap()
        .command()
        .contains("--session-id "));
}
