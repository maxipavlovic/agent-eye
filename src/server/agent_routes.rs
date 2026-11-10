//! The enabled agents, and the check that a request names one of them.

use super::api::{ApiError, ApiResult};
use super::Shared;
use crate::agents::Agents;
use anyhow::Result;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

/// The enabled agents: `{agents: [{id, name, short, default}]}`.
pub(super) async fn api_agents(State(st): State<Shared>) -> ApiResult {
    let list: Vec<Value> = st
        .agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            json!({"id": a.id(), "name": a.name(), "short": a.short_name(), "default": i == 0})
        })
        .collect();
    Ok(Json(json!({"agents": list})))
}

/// An agent id from a request: empty means the default agent, anything else must be enabled.
pub(super) fn checked_agent<'a>(
    agents: &Agents,
    id: Option<&'a str>,
) -> Result<Option<&'a str>, ApiError> {
    match id.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) if agents.get(a).is_none() => {
            Err(ApiError::bad(format!("the agent '{a}' is not enabled")))
        }
        other => Ok(other),
    }
}
