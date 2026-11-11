//! Reminders: set or clear a "check on this" date on a session or an idea.

use super::api::{open_db, ApiError, ApiResult};
use super::card::{idea_card, load_card};
use super::Shared;
use crate::ideas;
use crate::remind::{self, Target};
use crate::util::now_ms;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct RemindBody {
    /// The time in the words of `eye remind`: `tomorrow`, `mon`, `4h`, `2026-11-20 14:00`.
    when: Option<String>,
    note: Option<String>,
    /// Remove the reminder.
    clear: bool,
}

/// Same as `eye remind`: the id is a session id (or prefix) or `idea:<n>`.
pub(super) async fn api_remind(
    State(st): State<Shared>,
    Path(id): Path<String>,
    Json(b): Json<RemindBody>,
) -> ApiResult {
    let db = open_db(&st)?;
    let now = now_ms();
    let target = Target::resolve(&db, &id).map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    if b.clear {
        remind::clear(&db, &target)?;
    } else {
        let when = b
            .when
            .as_deref()
            .ok_or_else(|| ApiError::bad("say `when`, or `clear`"))?;
        let at = remind::parse_local(when, now).map_err(|e| ApiError::bad(format!("{e:#}")))?;
        remind::set(&db, &target, at, b.note.as_deref())?;
    }
    match target {
        Target::Idea(n) => {
            let idea = ideas::get(&db, n)?.ok_or_else(|| ApiError::not_found("no such idea"))?;
            Ok(Json(idea_card(&idea, &st.agents, &st.cfg, now)))
        }
        Target::Session(sid) => {
            let s = db
                .session_by_id(&sid)?
                .ok_or_else(|| ApiError::not_found("no such session"))?;
            Ok(Json(load_card(&db, &s, now, st.card_ctx())?))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ideas::{self, NewIdea};
    use crate::server::test_support::*;
    use crate::util::now_ms;
    use axum::http::StatusCode;
    use serde_json::json;

    #[tokio::test]
    async fn a_reminder_is_set_and_cleared_on_a_session() {
        let e = env();
        add_session(
            &e.db,
            "aaaaaaaa-1",
            "Waiting on Ana",
            Some("waiting_on_others"),
            4,
            now_ms(),
        );
        let (st, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa/remind",
                Some(json!({"when": "tomorrow", "note": "Ana's reply"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(c["remind_at"].as_i64().unwrap() > now_ms());
        assert_eq!(
            (c["remind_note"].as_str(), c["remind_due"].as_bool()),
            (Some("Ana's reply"), Some(false))
        );
        assert_eq!(c["column"], "waiting", "not due yet: it stays where it is");
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert!(c["remind_at"].is_i64());
        let (_, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa-1/remind",
                Some(json!({"clear": true})),
            ),
        )
        .await;
        assert!(c["remind_at"].is_null() && c["remind_note"].is_null());
        assert_eq!(c["remind_due"], false);
    }

    #[tokio::test]
    async fn a_card_with_nothing_set_says_so() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", Some("paused"), 4, now_ms());
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert!(c["remind_at"].is_null() && c["remind_note"].is_null());
        assert_eq!(c["remind_due"], false);
    }

    #[tokio::test]
    async fn a_due_reminder_brings_a_waiting_card_to_the_top_of_needs_you() {
        let e = env();
        let now = now_ms();
        add_session(&e.db, "aaaaaaaa-1", "Asked", Some("needs_you"), 4, now);
        add_session(
            &e.db,
            "bbbbbbbb-2",
            "Waiting on Ana",
            Some("waiting_on_others"),
            4,
            now - 5000,
        );
        add_session(
            &e.db,
            "cccccccc-3",
            "Not yet",
            Some("waiting_on_others"),
            4,
            now - 5000,
        );
        e.db.conn
            .execute(
                "UPDATE session SET column_manual='waiting', column_manual_at=?1 WHERE id='bbbbbbbb-2'",
                [now - 4000],
            )
            .unwrap();
        e.db.conn
            .execute(
                "UPDATE session SET remind_at=?1, remind_note='Ana''s reply' WHERE id='bbbbbbbb-2'",
                [now - 1000],
            )
            .unwrap();
        e.db.conn
            .execute(
                "UPDATE session SET remind_at=?1 WHERE id='cccccccc-3'",
                [now + 86_400_000],
            )
            .unwrap();
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(card_ids(&b, "needs_you"), ["bbbbbbbb-2", "aaaaaaaa-1"]);
        assert_eq!(card_ids(&b, "waiting"), ["cccccccc-3"]);
        let due = &b["columns"][2]["cards"][0];
        assert_eq!(
            (due["remind_due"].as_bool(), due["remind_note"].as_str()),
            (Some(true), Some("Ana's reply"))
        );
        assert_eq!(
            due["manual"], true,
            "the hand-placed column is still on record"
        );
        // putting it somewhere by hand answers it
        let (_, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/bbbbbbbb-2/column",
                Some(json!({"column": "paused"})),
            ),
        )
        .await;
        assert_eq!(
            (c["column"].as_str(), c["remind_due"].as_bool()),
            (Some("paused"), Some(false))
        );
        assert!(c["remind_at"].is_null());
        // Done ends a reminder that is not due yet too
        let (_, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/cccccccc-3/column",
                Some(json!({"column": "done"})),
            ),
        )
        .await;
        assert!(c["remind_at"].is_null());
    }

    #[tokio::test]
    async fn a_session_you_went_back_to_is_not_brought_back() {
        let e = env();
        let now = now_ms();
        add_session(&e.db, "aaaaaaaa-1", "Resumed", Some("done"), 4, now - 500);
        e.db.conn
            .execute("UPDATE session SET remind_at=?1", [now - 1000])
            .unwrap();
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(card_ids(&b, "done"), ["aaaaaaaa-1"]);
        assert!(b["columns"][5]["cards"][0]["remind_at"].is_null());
    }

    #[tokio::test]
    async fn an_idea_with_a_due_reminder_is_in_needs_you_and_stays_an_idea() {
        let e = env();
        let new = |t| NewIdea {
            title: t,
            ..Default::default()
        };
        let due = ideas::add(&e.db, &new("Look at the queue")).unwrap();
        let later = ideas::add(&e.db, &new("Next month")).unwrap();
        ideas::add(&e.db, &new("Plain")).unwrap();
        let (st, c) = call(
            &e,
            req(
                "POST",
                &format!("/api/cards/idea:{}/remind", later.id),
                Some(json!({"when": "1w"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(c["remind_at"].is_i64());
        e.db.conn
            .execute(
                "UPDATE idea SET remind_at=?2, remind_note='by Friday' WHERE id=?1",
                rusqlite::params![due.id, now_ms() - 1],
            )
            .unwrap();
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(card_ids(&b, "needs_you"), [format!("idea:{}", due.id)]);
        assert_eq!(
            card_ids(&b, "ideas"),
            [format!("idea:{}", later.id), "idea:3".to_string()]
        );
        let card = &b["columns"][2]["cards"][0];
        assert_eq!(
            (
                card["kind"].as_str(),
                card["column"].as_str(),
                card["remind_note"].as_str()
            ),
            (Some("idea"), Some("needs_you"), Some("by Friday"))
        );
        let (_, d) = call(&e, req("GET", &format!("/api/cards/idea:{}", due.id), None)).await;
        assert_eq!(
            (d["remind_due"].as_bool(), d["column"].as_str()),
            (Some(true), Some("needs_you"))
        );
        let (_, c) = call(
            &e,
            req(
                "POST",
                &format!("/api/cards/idea:{}/remind", due.id),
                Some(json!({"clear": true})),
            ),
        )
        .await;
        assert!(c["remind_at"].is_null() && c["column"] == "ideas");
        let (_, list) = call(&e, req("GET", "/api/ideas", None)).await;
        assert!(list["ideas"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["remind_at"].is_i64()));
    }

    #[tokio::test]
    async fn bad_requests_are_told_why() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", Some("paused"), 4, now_ms());
        for (uri, body, want) in [
            (
                "/api/cards/aaaaaaaa-1/remind",
                json!({"when": "soon"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/cards/aaaaaaaa-1/remind",
                json!({"when": "2020-01-01"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/cards/aaaaaaaa-1/remind",
                json!({"note": "x"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/cards/zzzzzzzz/remind",
                json!({"when": "1d"}),
                StatusCode::NOT_FOUND,
            ),
            (
                "/api/cards/idea:99/remind",
                json!({"when": "1d"}),
                StatusCode::NOT_FOUND,
            ),
        ] {
            let (st, _) = call(&e, req("POST", uri, Some(body))).await;
            assert_eq!(st, want, "{uri}");
        }
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert!(c["remind_at"].is_null());
    }
}
