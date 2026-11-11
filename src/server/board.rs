//! The board and the archive view.

use super::api::{hidden_set, open_db, wants_all, AllQ, ApiResult};
use super::card::{card_json, idea_card, CardCtx};
use super::Shared;
use crate::agents::Agents;
use crate::config::Config;
use crate::db::Db;
use crate::hide;
use crate::ideas;
use crate::model::{Column, Output, Stream};
use crate::util::now_ms;
use anyhow::Result;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

pub(super) const COLUMN_TITLES: [(Column, &str); 6] = [
    (Column::Ideas, "Ideas"),
    (Column::Running, "Running"),
    (Column::NeedsYou, "Needs you"),
    (Column::Waiting, "Waiting on others"),
    (Column::Paused, "Paused"),
    (Column::Done, "Done"),
];

/// A Done session idle for longer than `[board] done_days` leaves the board for the archive view.
pub(super) fn aged_out(column: &str, last_ms: i64, now: i64, cfg: &Config) -> bool {
    column == "done"
        && cfg.board.done_days > 0
        && now - last_ms > i64::from(cfg.board.done_days) * 24 * 3600 * 1000
}

/// A card's place in its lane: cards with a due reminder first, each group newest first (a due
/// card by the time it came due).
type Place = (bool, i64, Value);

fn place(card: Value, last_ms: i64) -> Place {
    let due = card["remind_due"].as_bool().unwrap_or(false);
    let at = if due {
        card["remind_at"].as_i64()
    } else {
        None
    };
    (due, at.unwrap_or(last_ms), card)
}

/// The board: the columns with their cards, newest first (due reminders on top), and the archive count.
pub fn board_json(db: &Db, cfg: &Config, agents: &Agents, now: i64, all: bool) -> Result<Value> {
    let cx = CardCtx {
        user: &cfg.user,
        agents,
    };
    let streams = db.all_streams()?;
    let outputs = db.all_outputs()?;
    let empty_s: Vec<Stream> = vec![];
    let empty_o: Vec<Output> = vec![];
    let mut buckets: HashMap<&'static str, Vec<Place>> = HashMap::new();
    let mut archive_count = 0usize;
    let hidden = hidden_set(db, cfg, all)?;
    for s in db.all_sessions()? {
        if hidden.contains(&s.id) {
            continue;
        }
        let card = card_json(
            &s,
            streams.get(&s.id).unwrap_or(&empty_s),
            outputs.get(&s.id).unwrap_or(&empty_o),
            now,
            false,
            cx,
        );
        let col = card["column"].as_str().unwrap_or("paused");
        if col == "archive" || aged_out(col, s.last_activity_at.unwrap_or(0), now, cfg) {
            archive_count += 1;
            continue;
        }
        let key = COLUMN_TITLES
            .iter()
            .find(|(c, _)| c.as_str() == col)
            .map(|(c, _)| c.as_str())
            .unwrap_or("paused");
        buckets
            .entry(key)
            .or_default()
            .push(place(card, s.last_activity_at.unwrap_or(0)));
    }
    // an idea whose reminder is due is in Needs you, still an idea card
    let enabled = cfg.enabled_agent_ids();
    let mut idea_cards = vec![];
    for i in ideas::list_open(db)?
        .iter()
        .filter(|i| !hide::idea_off(&enabled, i))
    {
        let card = idea_card(i, agents, cfg, now);
        if card["remind_due"] == true {
            let key = Column::NeedsYou.as_str();
            buckets.entry(key).or_default().push(place(card, 0));
        } else {
            idea_cards.push(card);
        }
    }
    let mut cols = vec![];
    for (c, title) in COLUMN_TITLES {
        let mut cards: Vec<Value> = if c == Column::Ideas {
            std::mem::take(&mut idea_cards)
        } else {
            let mut v = buckets.remove(c.as_str()).unwrap_or_default();
            v.sort_by_key(|(due, t, _)| (std::cmp::Reverse(*due), std::cmp::Reverse(*t)));
            v.into_iter().map(|(_, _, c)| c).collect()
        };
        cards.shrink_to_fit();
        cols.push(json!({"id": c.as_str(), "title": title, "cards": cards}));
    }
    Ok(json!({"columns": cols, "archive_count": archive_count, "generated_at": now}))
}

pub(super) async fn api_board(State(st): State<Shared>, Query(q): Query<AllQ>) -> ApiResult {
    let db = open_db(&st)?;
    Ok(Json(board_json(
        &db,
        &st.cfg,
        &st.agents,
        now_ms(),
        wants_all(&q.all),
    )?))
}

#[derive(Deserialize)]
pub(super) struct ArchiveQ {
    offset: Option<usize>,
    limit: Option<usize>,
    all: Option<String>,
}

pub(super) async fn api_archive(State(st): State<Shared>, Query(q): Query<ArchiveQ>) -> ApiResult {
    let db = open_db(&st)?;
    let now = now_ms();
    let streams = db.all_streams()?;
    let outputs = db.all_outputs()?;
    let hidden = hidden_set(&db, &st.cfg, wants_all(&q.all))?;
    let mut rows: Vec<(i64, Value)> = db
        .all_sessions()?
        .iter()
        .filter(|s| !hidden.contains(&s.id))
        .map(|s| {
            let c = card_json(
                s,
                streams.get(&s.id).map(Vec::as_slice).unwrap_or(&[]),
                outputs.get(&s.id).map(Vec::as_slice).unwrap_or(&[]),
                now,
                false,
                st.card_ctx(),
            );
            (s.last_activity_at.unwrap_or(0), c)
        })
        .filter(|(t, c)| {
            c["column"] == "archive"
                || aged_out(c["column"].as_str().unwrap_or(""), *t, now, &st.cfg)
        })
        .collect();
    rows.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    let total = rows.len();
    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let cards: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(_, c)| c)
        .collect();
    Ok(Json(
        json!({"cards": cards, "total": total, "offset": offset}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ideas;
    use crate::server::test_support::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn board_shape_and_card_rules() {
        let e = env();
        let now = now_ms();
        add_session(
            &e.db,
            "aaaaaaaa-1",
            "PROJ-100 fix login",
            Some("needs_you"),
            5,
            now,
        );
        add_session(&e.db, "bbbbbbbb-2", "Old chat", None, 1, now - 1000);
        add_session(
            &e.db,
            "cccccccc-3",
            "Done thing",
            Some("done"),
            4,
            now - 5000,
        );
        add_output(&e.db, "aaaaaaaa-1", "jira", "ABC-1", None, 1);
        add_output(&e.db, "aaaaaaaa-1", "jira", "ABC-2", None, 2);
        add_output(&e.db, "aaaaaaaa-1", "jira", "ABC-3", None, 3);
        add_output(&e.db, "aaaaaaaa-1", "jira", "PROJ-100", None, 4);
        add_output(
            &e.db,
            "aaaaaaaa-1",
            "pr",
            "PR #7",
            Some("https://bitbucket.org/a/b/pull-requests/7"),
            5,
        );
        for i in 0..7 {
            e.db.conn
                .execute(
                    "INSERT INTO stream(session_id,kind,key,title,status) VALUES('aaaaaaaa-1','child',?1,?1,?2)",
                    rusqlite::params![format!("c{i}"), if i < 4 { "done" } else { "running" }],
                )
                .unwrap();
            e.db.conn
                .execute(
                    "INSERT INTO stream(session_id,kind,key,title,status) VALUES('aaaaaaaa-1','topic',?1,?1,'active')",
                    [format!("t{i}")],
                )
                .unwrap();
        }
        ideas::add(
            &e.db,
            &ideas::NewIdea {
                title: "An idea",
                notes: Some("n"),
                folder: Some("/tmp"),
                ..Default::default()
            },
        )
        .unwrap();

        let (st, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(st, StatusCode::OK);
        let cols = b["columns"].as_array().unwrap();
        let ids: Vec<&str> = cols.iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            ["ideas", "running", "needs_you", "waiting", "paused", "done"]
        );
        assert_eq!(cols[0]["cards"].as_array().unwrap().len(), 1);
        assert_eq!(cols[2]["cards"][0]["id"], "aaaaaaaa-1");
        assert_eq!(cols[5]["cards"][0]["id"], "cccccccc-3");
        assert_eq!(b["archive_count"], 1);

        let card = &cols[2]["cards"][0];
        assert!(card["branch"].is_null());
        assert_eq!(card["child_count"], 7);
        assert_eq!(card["child_done"], 4);
        assert_eq!(card["topics"].as_array().unwrap().len(), 5);
        assert_eq!(card["topics_more"], 2);
        let outs = card["outputs"].as_array().unwrap();
        assert_eq!(outs[0]["kind"], "pr");
        let jira: Vec<&str> = outs
            .iter()
            .filter(|o| o["kind"] == "jira")
            .map(|o| o["value"].as_str().unwrap())
            .collect();
        assert_eq!(jira, ["PROJ-100", "ABC-1", "ABC-2"]);

        let (_, full) = call(&e, req("GET", "/api/cards/aaaaaaaa", None)).await;
        let n_jira = full["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["kind"] == "jira")
            .count();
        assert_eq!(n_jira, 4);
        assert_eq!(full["streams"].as_array().unwrap().len(), 14);
        assert_eq!(
            full["resume_cmd"],
            "cd '/tmp/demo' && claude --resume aaaaaaaa-1"
        );

        let (_, arch) = call(&e, req("GET", "/api/archive?limit=10", None)).await;
        assert_eq!(arch["total"], 1);
        assert_eq!(arch["cards"][0]["id"], "bbbbbbbb-2");
    }

    #[tokio::test]
    async fn old_done_sessions_leave_the_board_for_the_archive() {
        let e = env();
        let now = now_ms();
        let day = 24 * 3600 * 1000;
        add_session(
            &e.db,
            "aaaaaaaa-1",
            "Fresh done",
            Some("done"),
            4,
            now - 2 * day,
        );
        add_session(
            &e.db,
            "bbbbbbbb-2",
            "Stale done",
            Some("done"),
            4,
            now - 9 * day,
        );
        add_session(
            &e.db,
            "cccccccc-3",
            "Stale paused",
            Some("paused"),
            4,
            now - 90 * day,
        );
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(card_ids(&b, "done"), ["aaaaaaaa-1"]);
        assert_eq!(card_ids(&b, "paused"), ["cccccccc-3"]);
        assert_eq!(b["archive_count"], 1);
        let (_, a) = call(&e, req("GET", "/api/archive", None)).await;
        assert_eq!(a["total"], 1);
        assert_eq!(a["cards"][0]["id"], "bbbbbbbb-2");

        // 0 = no limit
        let mut cfg = Config::default();
        cfg.board.done_days = 0;
        let e = env_cfg(cfg);
        add_session(
            &e.db,
            "bbbbbbbb-2",
            "Stale done",
            Some("done"),
            4,
            now - 400 * day,
        );
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(card_ids(&b, "done"), ["bbbbbbbb-2"]);
        assert_eq!(b["archive_count"], 0);
        let (_, a) = call(&e, req("GET", "/api/archive", None)).await;
        assert_eq!(a["total"], 0);

        // a shorter limit
        let mut cfg = Config::default();
        cfg.board.done_days = 1;
        let e = env_cfg(cfg);
        add_session(
            &e.db,
            "aaaaaaaa-1",
            "Two days",
            Some("done"),
            4,
            now - 2 * day,
        );
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert!(card_ids(&b, "done").is_empty());
        assert_eq!(b["archive_count"], 1);
        let (_, a) = call(&e, req("GET", "/api/archive", None)).await;
        assert_eq!(a["cards"][0]["id"], "aaaaaaaa-1");
    }

    #[tokio::test]
    async fn trivial_sessions_are_hidden_unless_all() {
        let e = env();
        let now = now_ms();
        add_session(&e.db, "aaaaaaaa-1", "clear", Some("paused"), 1, now);
        add_session(&e.db, "bbbbbbbb-2", "Real work", Some("paused"), 1, now);
        add_session(&e.db, "cccccccc-3", "typo", Some("paused"), 1, now);
        for (id, first) in [
            ("aaaaaaaa-1", "/clear"),
            ("bbbbbbbb-2", "Find the session with Catalog migration."),
            ("cccccccc-3", "exirt"),
        ] {
            e.db.conn
                .execute(
                    "UPDATE session SET first_prompt=?2, last_assistant_text='ok' WHERE id=?1",
                    rusqlite::params![id, first],
                )
                .unwrap();
            crate::db::fts_write(&e.db.conn, id, first, "", true, 10_000, 10_000).unwrap();
        }
        let n = |v: &Value| -> usize {
            v["columns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["cards"].as_array().unwrap().len())
                .sum()
        };
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(n(&b), 1);
        let (_, b) = call(&e, req("GET", "/api/board?all=1", None)).await;
        assert_eq!(n(&b), 3);
        // detail by id still works for a hidden session
        let (st, _) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(st, StatusCode::OK);
        // manual unhide beats the rule, manual hide beats a real session
        e.db.set_hidden("aaaaaaaa-1", Some(false)).unwrap();
        e.db.set_hidden("bbbbbbbb-2", Some(true)).unwrap();
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        let ids: Vec<String> = b["columns"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| {
                c["cards"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x["id"].as_str().unwrap().to_string())
            })
            .collect();
        assert_eq!(ids, vec!["aaaaaaaa-1".to_string()]);
        // archive and search follow the same rule
        e.db.conn
            .execute("UPDATE session SET archived=1", [])
            .unwrap();
        let (_, a) = call(&e, req("GET", "/api/archive", None)).await;
        assert_eq!(a["total"], 1);
        let (_, a) = call(&e, req("GET", "/api/archive?all=1", None)).await;
        assert_eq!(a["total"], 3);
        let (_, r) = call(&e, req("GET", "/api/search?q=exirt", None)).await;
        assert_eq!(r["results"].as_array().unwrap().len(), 0);
        let (_, r) = call(&e, req("GET", "/api/search?q=exirt&all=1", None)).await;
        assert_eq!(r["results"].as_array().unwrap().len(), 1);
    }
}
