use super::preview::wrap;
use super::*;
use crate::agents::claude::Claude;
use crate::agents::codex::Codex;
use crate::agents::{AgentRef, Agents};
use crate::tui::render_text;
use crate::tui::test_support::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn the_preview_shortens_the_home_folder() {
    let line = "cd '/Users/you/code/a b' && claude --resume abcd-1".to_string();
    assert_eq!(
        shown_line(line.clone(), Some("/Users/you")),
        "cd ~/'code/a b' && claude --resume abcd-1"
    );
    assert_eq!(shown_line(line.clone(), Some("/Users/other")), line);
    assert_eq!(shown_line(line.clone(), None), line);
}

#[test]
fn frames_at_three_sizes() {
    for (w, h) in [(80, 24), (120, 40), (200, 50)] {
        let mut a = app("");
        let t = render_text(&mut a, w, h).unwrap();
        println!("---- {w}x{h}, empty query ----\n{t}");
        for l in t.lines() {
            assert!(l.chars().count() <= w as usize);
        }
        assert!(t.contains("Esc"));
        let mut a = app("catalog");
        let t = render_text(&mut a, w, h).unwrap();
        println!("---- {w}x{h}, query 'catalog' ----\n{t}");
        assert!(t.lines().nth(1).unwrap().contains("Catalog migration"));
    }
}

/// The picker over Claude Code and Codex CLI, its second row a Codex session.
fn mixed_app() -> App {
    let mut a = app("");
    let claude: AgentRef = std::sync::Arc::new(Claude::new("/nonexistent/c".into()));
    let codex: AgentRef = std::sync::Arc::new(Codex::new("/nonexistent/x".into()));
    a.agents = Agents::from_vec(vec![claude, codex]);
    a.idx.rows[1].session.as_mut().unwrap().agent = "codex".into();
    a
}

#[test]
fn with_several_agents_the_rows_and_the_preview_name_the_agent() {
    let mut a = mixed_app();
    let t = render_text(&mut a, 80, 30).unwrap();
    let row = |t: &str, title: &str| t.lines().find(|l| l.contains(title)).unwrap().to_string();
    let codex = row(&t, "Weekly report skill");
    let claude = row(&t, "Catalog migration");
    assert!(
        codex.contains("Codex") && !codex.contains("Claude"),
        "{codex}"
    );
    assert!(
        claude.contains("Claude") && !claude.contains("Codex"),
        "{claude}"
    );
    assert!(
        row(&t, "Try the Zephyr").contains("Claude"),
        "an idea belongs to the default agent"
    );
    for l in t.lines() {
        assert!(l.chars().count() <= 80, "{l}");
    }
    // the preview of the selected row says it too
    a.key(key(KeyCode::Down));
    let t = render_text(&mut a, 80, 30).unwrap();
    assert!(t.contains("Running · Codex · "), "{t}");
    // a narrow list has no room for the column and stays readable
    let t = render_text(&mut a, 50, 20).unwrap();
    assert!(!row(&t, "Weekly report").contains("Codex"), "{t}");
}

#[test]
fn the_preview_says_the_days_worked_once_they_are_counted() {
    use crate::worked::{store, WorkEvent};
    let mut a = app("");
    let t = render_text(&mut a, 100, 30).unwrap();
    assert!(!t.contains("worked "), "{t}");
    let at = a.now - 3_600_000;
    let events = [
        WorkEvent {
            ts: at,
            prompt: true,
        },
        WorkEvent {
            ts: at + 600_000,
            prompt: false,
        },
    ];
    let id = a.current().unwrap().id.clone();
    store::record(&a.db.conn, &id, &events, true).unwrap();
    let t = render_text(&mut a, 100, 30).unwrap();
    assert!(t.contains("worked 1 day · 10 m · last "), "{t}");
}

#[test]
fn with_one_agent_nothing_is_added_to_the_rows_or_the_preview() {
    let mut a = app("");
    let t = render_text(&mut a, 80, 30).unwrap();
    assert!(!t.contains("Codex"));
    let list: String = t
        .lines()
        .filter(|l| l.contains("Weekly report") || l.contains("Catalog migration"))
        .collect();
    assert!(!list.contains("Claude"), "{list}");
    assert!(
        !t.contains("Needs you · Claude") && !t.contains("· Claude ·"),
        "{t}"
    );
}

#[test]
fn an_empty_picker_says_where_sessions_come_from() {
    let mut a = app("");
    a.idx.rows.clear();
    a.research();
    let t = render_text(&mut a, 80, 24).unwrap();
    assert!(
        t.contains("no sessions yet: start one in Claude Code, then run eye again"),
        "{t}"
    );
    // a search with no hit is another message, and a narrow terminal wraps instead of cutting
    let mut a = app("zzzz");
    let t = render_text(&mut a, 80, 24).unwrap();
    assert!(t.contains("no session matches") && !t.contains("no sessions yet"));
    let mut a = app("");
    a.idx.rows.clear();
    a.research();
    let t = render_text(&mut a, 40, 20).unwrap();
    assert!(t.contains("then run eye again"), "{t}");
}

#[test]
fn the_preview_says_why_a_session_cannot_be_resumed() {
    let mut a = app("");
    a.idx.rows[0].session.as_mut().unwrap().cwd = Some("/tmp/a\nb".into());
    let t = render_text(&mut a, 200, 40).unwrap();
    assert!(t.contains("control"), "{t}");
    assert!(!t.contains("Resume "), "{t}");
}

#[test]
fn live_session_preview_says_so() {
    let mut a = app("");
    a.idx.rows[0].session.as_mut().unwrap().live_status = Some("busy".into());
    let t = render_text(&mut a, 120, 40).unwrap();
    assert!(t.contains("Open now in another terminal"));
}

#[test]
fn the_next_line_does_not_repeat_its_owner() {
    for (stored, shown) in [
        ("You: review the PR", "review the PR"),
        ("Sam: review the PR", "review the PR"),
        ("Claude: fix the test", "fix the test"),
        ("Dana: approve the budget", "Dana: approve the budget"),
    ] {
        let mut a = app("");
        a.user = crate::config::UserCfg { name: "Sam".into() };
        a.idx.rows[0].session.as_mut().unwrap().next_step = Some(stored.into());
        let t = render_text(&mut a, 140, 40).unwrap();
        let line = t.lines().find(|l| l.contains("Next (")).unwrap_or("");
        assert!(line.contains(shown), "{stored}: {line}");
        assert!(
            !line.contains("You:") && !line.contains("Sam:") && !line.contains("Claude:"),
            "{stored}: {line}"
        );
    }
}

#[test]
fn conversation_view_reports_missing_transcripts_and_ideas() {
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    let (_tmp, mut a) = talk_app(1);
    a.key(ctrl('t'));
    a.key(key(KeyCode::Down));
    let t = render_text(&mut a, 120, 20).unwrap();
    assert!(
        t.contains("Conversation: no transcript or archived copy"),
        "{t}"
    );
    for c in "hyd".chars() {
        a.key(key(KeyCode::Char(c)));
    }
    let t = render_text(&mut a, 120, 20).unwrap();
    assert!(t.contains("an idea has no conversation"), "{t}");
}

#[test]
fn preview_shows_the_token_line() {
    let mut a = app("");
    a.idx.rows[0].session.as_mut().unwrap().tokens = Some(crate::model::Tokens {
        input: 1_000_000,
        output: 500_000,
        cache_write: 300_000,
        cache_read: 100_000,
        subagents_total: 410_000,
    });
    let t = render_text(&mut a, 120, 40).unwrap();
    assert!(t.contains("Tokens 2.3M (sub-agents 410k)"), "{t}");
}

#[test]
fn preview_says_where_a_session_started_unless_it_was_the_terminal() {
    let mut a = app("");
    {
        let s = a.idx.rows[0].session.as_mut().unwrap();
        s.origin = Some("app".into());
        s.origin_raw = Some("sdk-ts".into());
    }
    let t = render_text(&mut a, 120, 40).unwrap();
    assert!(t.contains("started from: an app (sdk-ts)"), "{t}");
    a.idx.rows[0].session.as_mut().unwrap().origin = Some("terminal".into());
    let t = render_text(&mut a, 120, 40).unwrap();
    assert!(!t.contains("started from"), "{t}");
}

#[test]
fn wrap_breaks_at_spaces_and_drops_control_characters() {
    assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
    assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    assert_eq!(wrap("a\n\nb", 4), ["a", "", "b"]);
    assert_eq!(wrap("x\u{1b}[31mred", 20), ["x[31mred"]);
    assert_eq!(wrap("", 5), [""]);
    for l in wrap("日本語のテキストを折り返す テスト", 6) {
        assert!(l.width() <= 6, "{l}");
    }
}

#[test]
fn narrow_and_tiny_terminals_do_not_panic() {
    for (w, h) in [(20, 5), (40, 8), (60, 12), (1, 1), (100, 3)] {
        let mut a = app("cat");
        let t = render_text(&mut a, w, h).unwrap();
        for l in t.lines() {
            assert!(l.chars().count() <= w as usize);
        }
    }
}

#[test]
fn the_preview_names_what_the_session_left_behind_and_ctrl_l_lists_it() {
    use crate::loose::{self, Kind, LooseEnd};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut a = app("");
    let id = a.current().unwrap().id.clone();
    let it = |key: &str, kind, what: &str, line: &str| LooseEnd {
        key: key.into(),
        session_id: id.clone(),
        kind,
        line: line.into(),
        detail: what.into(),
        since_ms: 1,
    };
    let items = [
        it(
            "proc:1:1",
            Kind::Process,
            "node vite (pid 1, port 4321)",
            "node vite (pid 1) is still serving port 4321",
        ),
        it(
            "proc:2:1",
            Kind::Process,
            "node api (pid 2, port 4350)",
            "node api (pid 2) is still serving port 4350",
        ),
        it(
            "unpushed:/r:abc",
            Kind::Unpushed,
            "3 commits in r",
            "3 commits in r are on no remote",
        ),
    ];
    // nothing stored: no line, and Ctrl-L says so
    let t = render_text(&mut a, 170, 30).unwrap();
    assert!(!t.contains("Left behind"), "{t}");
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    a.key(ctrl_l);
    assert_eq!(a.msg.as_deref(), Some("nothing left behind"));
    loose::replace(&a.db, &items, a.now).unwrap();
    a.detail.loose = loose::list(&a.db, Some(&id), false).unwrap();
    let t = render_text(&mut a, 170, 30).unwrap();
    assert!(
        t.contains("Left behind: 2 servers (ports 4321, 4350), 3 commits not pushed"),
        "{t}"
    );
    // the list, with the keys in the footer
    a.key(ctrl_l);
    let t = render_text(&mut a, 170, 30).unwrap();
    assert!(t.contains("› node api") || t.contains("› node vite"), "{t}");
    assert!(
        t.contains("s stop   r remove   c copy command   i ignore"),
        "{t}"
    );
    // stop asks first with the exact words; anything but y cancels
    a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert!(
        a.msg
            .as_deref()
            .unwrap()
            .ends_with("(pid 1, port 4321)? y/N")
            || a.msg
                .as_deref()
                .unwrap()
                .ends_with("(pid 2, port 4350)? y/N")
    );
    a.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert_eq!(a.msg.as_deref(), Some("cancelled"));
    assert_eq!(
        loose::list(&a.db, None, false).unwrap().len(),
        3,
        "nothing was done"
    );
    // a commit line cannot be stopped or removed, only copied
    a.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert!(a
        .msg
        .as_deref()
        .unwrap()
        .starts_with("that does not apply to unpushed"));
    // ignore asks, y does it, and the preview line shrinks
    a.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    assert_eq!(a.msg.as_deref(), Some("Stop listing 3 commits in r? y/N"));
    a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(a.detail.loose.len(), 2);
    assert_eq!(loose::list(&a.db, None, false).unwrap().len(), 2);
    // a stale process key is refused by the safety check, not killed; send asks and leaves the picker
    let stale = format!("proc:{}:1", std::process::id());
    loose::replace(
        &a.db,
        &[it(&stale, Kind::Process, "me (pid 1)", "me")],
        a.now,
    )
    .unwrap();
    a.loose.as_mut().unwrap().items = loose::list(&a.db, Some(&id), false).unwrap();
    a.loose.as_mut().unwrap().sel = 0;
    a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(
        a.msg.as_deref().unwrap().contains("another process"),
        "{:?}",
        a.msg
    );
    loose::replace(
        &a.db,
        &[it(
            "uncommitted:/r",
            Kind::Uncommitted,
            "2 modified files in r",
            "x",
        )],
        a.now,
    )
    .unwrap();
    a.detail.loose = loose::list(&a.db, Some(&id), false).unwrap();
    a.key(ctrl_l);
    a.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    assert!(a.msg.as_deref().unwrap().ends_with("left behind? y/N"));
    let out = a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(out, Some(crate::tui::Outcome::SendBack(id)));
}

#[test]
fn a_night_run_shows_a_plain_night_mark_in_the_row_and_its_report_line_in_the_preview() {
    let mut a = app("");
    let id = a.current().unwrap().id.clone();
    let folder = "/home/x/.agent-eye/night/7/wt";
    for r in a.idx.rows.iter_mut().filter(|r| r.id == id) {
        r.session.as_mut().unwrap().cwd = Some(folder.into());
    }
    a.db.conn
        .execute(
            "INSERT INTO night_run(kind, agent, folder, prompt, night, status, run_session_id, branch, report, started_at, ended_at, created_at)
             VALUES('idea','claude','/x','p','2026-11-21','done',?1,'night/20261121-x','Added the line.\nSecond line.',0,2460000,1)",
            [&id],
        )
        .unwrap();
    let d = crate::pick::detail(&a.db, a.current().unwrap());
    a.detail = d;
    let t = render_text(&mut a, 170, 30).unwrap();
    assert!(
        t.lines()
            .any(|l| l.contains("night") && !l.contains("Night run")),
        "{t}"
    );
    assert!(t.contains("Night run"), "{t}");
    assert!(t.contains("done, 41 min, branch night/20261121-x"), "{t}");
    assert!(t.contains("Added the line."), "{t}");
    assert!(!t.contains("Second line."), "{t}");
}
