//! Rows and an app for the picker tests.

use super::state::App;
use crate::agents::Agents;
use crate::db::Db;
use crate::model::{Column, Idea, Session};
use crate::paths::Paths;
use crate::pick::{Index, Kind, Row};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub(super) fn sess(id: &str, title: &str, col: Column, mins_ago: i64, now: i64) -> Row {
    let s = Session {
        id: id.into(),
        title: Some(title.into()),
        cwd: Some("/tmp/demo/webapp".into()),
        git_branch: Some("feature/x".into()),
        last_activity_at: Some(now - mins_ago * 60_000),
        state_line: Some(
            "The SPA catalog page is wired to the new endpoint; tests pass locally.".into(),
        ),
        next_step: Some("Review the draft PR and merge it before the Friday release.".into()),
        kind: Some("interactive".into()),
        transcript_state: Some("present".into()),
        ..Default::default()
    };
    Row {
        kind: Kind::Session,
        id: id.into(),
        title: title.into(),
        title_lc: title.to_lowercase(),
        notes_lc: String::new(),
        project: "webapp".into(),
        folder: s.cwd.clone(),
        column: col,
        last: s.last_activity_at.unwrap(),
        hidden: false,
        sort: 0,
        session: Some(s),
        idea: None,
        remind: None,
    }
}

pub(super) fn app(query: &str) -> App {
    let now = 1_800_000_000_000;
    let db = Db::open_memory().unwrap();
    let mut rows = vec![
        sess(
            "aaaaaaaa-0000-4000-8000-000000000001",
            "Catalog migration for the new marketplace",
            Column::NeedsYou,
            90,
            now,
        ),
        sess(
            "bbbbbbbb-0000-4000-8000-000000000002",
            "Weekly report skill for Dana",
            Column::Running,
            5,
            now,
        ),
        sess(
            "cccccccc-0000-4000-8000-000000000003",
            "PROJ-123 catalog certification program with a very long title that must be cut",
            Column::Waiting,
            60 * 30,
            now,
        ),
        sess(
            "dddddddd-0000-4000-8000-000000000004",
            "Старая задача про каталог",
            Column::Paused,
            60 * 24 * 12,
            now,
        ),
        sess(
            "eeeeeeee-0000-4000-8000-000000000005",
            "Billing fixes",
            Column::Done,
            60 * 24 * 40,
            now,
        ),
    ];
    rows.push(Row {
        kind: Kind::Idea,
        id: "idea:1".into(),
        title: "Try the Zephyr demo again".into(),
        title_lc: "try the zephyr demo again".into(),
        notes_lc: "notes".into(),
        project: "auth".into(),
        folder: Some("/tmp/demo/auth".into()),
        column: Column::Ideas,
        last: now,
        hidden: false,
        sort: 0,
        session: None,
        idea: Some(Idea {
            id: 1,
            title: "Try the Zephyr demo again".into(),
            notes: Some("Run the prototype and write down what is missing".into()),
            folder: Some("/tmp/demo/auth".into()),
            ..Default::default()
        }),
        remind: None,
    });
    let mut a = App::new(db, Index { rows }, query, false, now);
    a.agents = Agents::from_vec(vec![std::sync::Arc::new(
        crate::agents::claude::Claude::new("/nonexistent/c".into()),
    )]);
    a.detail.outputs = vec![crate::model::Output {
        id: 1,
        session_id: "x".into(),
        kind: "pr".into(),
        value: "pr1".into(),
        url: Some("https://bitbucket.org/acme/webapp/pull-requests/42".into()),
        first_seen_at: None,
        title: None,
    }];
    a
}

pub(super) fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

/// An app whose first row has a transcript on disk with `n` question/answer pairs.
pub(super) fn talk_app(n: usize) -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    let t = tmp.path().join("t.jsonl");
    let mut body = String::new();
    for i in 0..n {
        body.push_str(&format!(
            "{{\"type\":\"user\",\"timestamp\":\"2026-11-08T10:00:00.000Z\",\"message\":{{\"content\":\"question {i}\"}}}}\n{{\"type\":\"assistant\",\"timestamp\":\"2026-11-08T10:00:05.000Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"answer {i}\"}},{{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"Bash\",\"input\":{{}}}}]}}}}\n"
        ));
    }
    std::fs::write(&t, body).unwrap();
    let mut a = app("");
    a.paths = Some(paths);
    a.idx.rows[0].session.as_mut().unwrap().transcript_path = Some(t.to_string_lossy().into());
    (tmp, a)
}
