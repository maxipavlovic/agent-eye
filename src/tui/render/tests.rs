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
