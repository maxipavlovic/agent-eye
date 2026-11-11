//! The JSON of a card: one session or idea as the board and the detail view show it.

use crate::agents::{self, Agents};
use crate::columns;
use crate::config::{Config, UserCfg};
use crate::db::Db;
use crate::folder::{self, FolderState};
use crate::model::{Column, Idea, Output, Session, Stream};
use crate::util::{head_chars, home_rel};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;

pub(super) const MAX_JIRA_ON_CARD: usize = 3;

pub(super) const MAX_TOPICS_ON_CARD: usize = 5;

// ---------- cards ----------

pub(super) fn status_of(s: &Stream) -> &str {
    s.status.as_deref().unwrap_or("")
}

pub(super) fn clean_branch(b: &Option<String>) -> Option<&str> {
    b.as_deref().filter(|b| !b.is_empty() && *b != "HEAD")
}

pub(super) fn output_json(o: &Output) -> Value {
    json!({"kind": o.kind, "value": o.value, "url": o.url, "title": o.title})
}

pub(super) fn stream_json(s: &Stream) -> Value {
    json!({
        "kind": s.kind, "key": s.key, "title": s.title, "status": s.status,
        "detail": s.detail, "link": s.link
    })
}

/// Outputs for a card face: PRs, artifacts, at most 3 Jira keys, one folder, in that order.
/// Jira keys that also appear in the title, branch or a PR are preferred.
pub(super) fn card_outputs(s: &Session, outputs: &[Output]) -> Vec<Value> {
    let mut hay = format!(
        "{} {}",
        s.title.as_deref().unwrap_or(""),
        clean_branch(&s.git_branch).unwrap_or("")
    );
    for o in outputs.iter().filter(|o| o.kind == "pr") {
        hay.push(' ');
        hay.push_str(&o.value);
        hay.push(' ');
        hay.push_str(o.url.as_deref().unwrap_or(""));
    }
    let hay = hay.to_uppercase();
    let (mut preferred, mut rest): (Vec<&Output>, Vec<&Output>) = outputs
        .iter()
        .filter(|o| o.kind == "jira")
        .partition(|o| hay.contains(&o.value.to_uppercase()));
    preferred.append(&mut rest);
    let jira = preferred.into_iter().take(MAX_JIRA_ON_CARD);
    let of = |k: &'static str| outputs.iter().filter(move |o| o.kind == k);
    of("pr")
        .take(3)
        .chain(of("artifact").take(3))
        .chain(jira)
        .chain(of("folder").take(1))
        .map(output_json)
        .collect()
}

pub(super) fn count_by_kind(outputs: &[Output]) -> Value {
    let mut m: HashMap<&str, usize> = HashMap::new();
    for o in outputs {
        *m.entry(o.kind.as_str()).or_default() += 1;
    }
    json!(m)
}

pub(super) fn effective_manual(s: &Session) -> Option<Column> {
    if s.archived {
        return None;
    }
    let c = s.column_manual.as_deref().and_then(Column::parse)?;
    (s.column_manual_at.unwrap_or(0) >= s.last_activity_at.unwrap_or(0)).then_some(c)
}

/// `detail` keeps every stream; board cards carry only topics (sub-agents are counted).
/// What a card needs from the config and the agents.
#[derive(Clone, Copy)]
pub struct CardCtx<'a> {
    pub user: &'a UserCfg,
    pub agents: &'a Agents,
}

/// The card of a session; `detail` adds all streams and outputs.
pub fn card_json(
    s: &Session,
    streams: &[Stream],
    outputs: &[Output],
    now: i64,
    detail: bool,
    cx: CardCtx,
) -> Value {
    let has_outputs = !outputs.is_empty();
    let input = columns::Input::from_session(s, has_outputs, now);
    let column = columns::compute(&input);
    let mut auto_in = input.clone();
    auto_in.column_manual = None;
    let auto_column = columns::compute(&auto_in);
    let children: Vec<&Stream> = streams.iter().filter(|x| x.kind == "child").collect();
    let child_done = children.iter().filter(|c| status_of(c) == "done").count();
    let topics: Vec<&Stream> = streams.iter().filter(|x| x.kind == "topic").collect();
    let topics_json: Vec<Value> = topics
        .iter()
        .take(MAX_TOPICS_ON_CARD)
        .map(|t| {
            json!({"key": t.key, "title": t.title.as_deref().unwrap_or(&t.key),
                   "status": t.status, "next": t.detail})
        })
        .collect();
    let (state, state_source) = match (&s.state_line, &s.recap) {
        (Some(v), _) if !v.trim().is_empty() => (Some(v.clone()), "summary"),
        (_, Some(r)) if !r.trim().is_empty() => (Some(head_chars(r.trim(), 220)), "recap"),
        _ => (None, "none"),
    };
    let title = s
        .title
        .clone()
        .or_else(|| s.first_prompt.as_deref().map(|p| head_chars(p, 80)))
        .unwrap_or_else(|| "(untitled)".into());
    let resume_cmd = cx.agents.resume_line(s);
    let resumable = s.kind.as_deref() != Some("history_only") && resume_cmd.is_some();
    let agent_id = if s.agent.is_empty() {
        agents::LEGACY_ID
    } else {
        &s.agent
    };
    json!({
        "id": s.id,
        "agent": agent_id,
        "agent_name": cx.agents.short_name(agent_id),
        "title": title,
        "folder": s.cwd.as_deref().map(home_rel),
        "repo": s.repo_root.as_deref().map(home_rel),
        "branch": clean_branch(&s.git_branch),
        "column": column.as_str(),
        "auto_column": auto_column.as_str(),
        "manual": effective_manual(s).is_some(),
        "last_activity_at": s.last_activity_at,
        "started_at": s.started_at,
        "state": state,
        "state_source": state_source,
        "next": s.next_line().map(|n| cx.user.as_you(&n)),
        "blocked": s.blocked(),
        "ai_status": s.ai_status,
        "recap": s.recap,
        "live_status": s.live_status,
        "kind": s.kind,
        "streams": streams.iter().filter(|x| detail || x.kind == "topic").map(stream_json).collect::<Vec<_>>(),
        "outputs": card_outputs(s, outputs),
        "output_counts": count_by_kind(outputs),
        "topics": topics_json,
        "topics_more": topics.len().saturating_sub(MAX_TOPICS_ON_CARD),
        "resume_cmd": if resumable { json!(resume_cmd) } else { Value::Null },
        "resumable": resumable,
        "parent_id": s.parent_session_id,
        "child_count": children.len(),
        "child_done": child_done,
        "prompt_count": s.prompt_count,
        "tokens": s.tokens.as_ref().map(|t| t.to_json()),
    })
}

pub(super) fn idea_card(i: &Idea, agents: &Agents, cfg: &Config) -> Value {
    let folder = i
        .folder
        .as_deref()
        .map(|f| folder::classify(cfg.folder_root().as_deref(), f));
    let mut card = json!({
        "id": format!("idea:{}", i.id),
        "idea_id": i.id,
        "agent": i.agent,
        "agent_name": agents.resolve(i.agent.as_deref()).map(|a| a.short_name()),
        "title": i.title,
        "folder": i.folder.as_deref().map(home_rel),
        "column": "ideas", "auto_column": "ideas", "manual": false,
        "last_activity_at": i.created_at, "started_at": i.created_at,
        "state": i.notes, "state_source": "idea", "next": null, "ai_status": null, "recap": null,
        "has_prompt": i.prompt.is_some(),
        "kind": "idea", "streams": [], "outputs": [], "output_counts": {}, "topics": [],
        "topics_more": 0, "child_count": 0, "child_done": 0, "prompt_count": 0,
        "resumable": false, "resume_cmd": null, "branch": null, "repo": null,
        "live_status": null, "parent_id": null, "from_session_id": i.from_session_id, "tokens": null,
    });
    if let Some(f) = folder {
        card["folder_state"] = json!(f.code());
        card["folder_creatable"] = json!(f == FolderState::Missing { creatable: true });
        card["folder_note"] = json!(f.note());
    }
    card
}

pub(super) fn load_card(db: &Db, s: &Session, now: i64, cx: CardCtx) -> Result<Value> {
    Ok(card_json(
        s,
        &db.streams_for(&s.id)?,
        &db.outputs_for(&s.id)?,
        now,
        false,
        cx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::ideas;
    use crate::server::test_support::*;
    use crate::util::now_ms;

    #[tokio::test]
    async fn state_source_names_where_the_state_comes_from() {
        let e = env();
        add_session(&e.db, "cccccccc-1", "Written", None, 3, now_ms());
        add_session(&e.db, "cccccccc-2", "Recap only", None, 3, now_ms());
        e.db.conn
            .execute(
                "UPDATE session SET state_line=NULL, recap='Goal: x. Next: y.' WHERE id='cccccccc-2'",
                [],
            )
            .unwrap();
        add_session(&e.db, "cccccccc-3", "Nothing", None, 3, now_ms());
        e.db.conn
            .execute(
                "UPDATE session SET state_line=NULL WHERE id='cccccccc-3'",
                [],
            )
            .unwrap();
        for (id, source) in [
            ("cccccccc-1", "summary"),
            ("cccccccc-2", "recap"),
            ("cccccccc-3", "none"),
        ] {
            let (_, c) = call(&e, req("GET", &format!("/api/cards/{id}"), None)).await;
            assert_eq!(c["state_source"], source, "{id}");
        }
    }

    #[tokio::test]
    async fn a_stopped_session_says_what_it_asks() {
        let e = env();
        add_session(&e.db, "bbbbbbbb-1", "Blocked", None, 3, now_ms());
        e.db.conn
            .execute(
                "UPDATE session SET live_status='waiting', waiting_since=1, waiting_note='answer: Which repo?',
                 waiting_prompt=1, next_step='Sam: later' WHERE id='bbbbbbbb-1'",
                [],
            )
            .unwrap();
        let (_, c) = call(&e, req("GET", "/api/cards/bbbbbbbb-1", None)).await;
        assert_eq!(
            (c["next"].as_str(), c["blocked"].as_bool()),
            (Some("You: answer: Which repo?"), Some(true))
        );
        // a finished reply that asks something waits on you without blocking
        e.db.conn
            .execute(
                "UPDATE session SET waiting_prompt=0 WHERE id='bbbbbbbb-1'",
                [],
            )
            .unwrap();
        let (_, c) = call(&e, req("GET", "/api/cards/bbbbbbbb-1", None)).await;
        assert_eq!(
            (c["next"].as_str(), c["blocked"].as_bool()),
            (Some("You: answer: Which repo?"), Some(false))
        );
        e.db.conn
            .execute(
                "UPDATE session SET live_status='busy' WHERE id='bbbbbbbb-1'",
                [],
            )
            .unwrap();
        let (_, c) = call(&e, req("GET", "/api/cards/bbbbbbbb-1", None)).await;
        assert_eq!(
            (c["next"].as_str(), c["blocked"].as_bool()),
            (Some("Sam: later"), Some(false))
        );
    }

    #[tokio::test]
    async fn the_configured_user_name_shows_as_you() {
        let cfg = Config::parse("[user]\nname = \"Sam\"").unwrap();
        let e = env_cfg(cfg);
        add_session(&e.db, "cccccccc-1", "Named", Some("paused"), 3, now_ms());
        e.db.conn
            .execute(
                "UPDATE session SET next_step='Sam: review the PR' WHERE id='cccccccc-1'",
                [],
            )
            .unwrap();
        let (_, c) = call(&e, req("GET", "/api/cards/cccccccc-1", None)).await;
        assert_eq!(c["next"], "You: review the PR");
        e.db.conn
            .execute(
                "UPDATE session SET next_step='Claude: fix it' WHERE id='cccccccc-1'",
                [],
            )
            .unwrap();
        let (_, c) = call(&e, req("GET", "/api/cards/cccccccc-1", None)).await;
        assert_eq!(c["next"], "Claude: fix it");
    }

    #[tokio::test]
    async fn cards_carry_tokens_or_null() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", Some("paused"), 1, now_ms());
        add_session(&e.db, "bbbbbbbb-2", "u", Some("paused"), 1, now_ms());
        ideas::add(
            &e.db,
            &ideas::NewIdea {
                title: "i",
                notes: Some("n"),
                folder: Some("/tmp"),
                ..Default::default()
            },
        )
        .unwrap();
        e.db.conn
            .execute(
                "UPDATE session SET tok_in=1000, tok_out=200, tok_cw=30, tok_cr=4, tok_sub=500 WHERE id='aaaaaaaa-1'",
                [],
            )
            .unwrap();
        let want = json!({"input":1000,"output":200,"cache_write":30,"cache_read":4,"total":1734,"subagents_total":500});
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(c["tokens"], want);
        let (_, c) = call(&e, req("GET", "/api/cards/bbbbbbbb-2", None)).await;
        assert!(c["tokens"].is_null());
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        let cards: Vec<&Value> = b["columns"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["cards"].as_array().unwrap())
            .collect();
        assert_eq!(cards.len(), 3);
        for c in cards {
            assert!(c.get("tokens").is_some(), "{c}");
            assert_eq!(c["tokens"] == want, c["id"] == "aaaaaaaa-1");
        }
    }
}
