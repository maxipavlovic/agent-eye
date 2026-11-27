use super::unproven::Unproven;
use super::*;
use crate::agents::claude::Claude;
use crate::agents::opencode::OpenCode;
use std::sync::Arc;

fn with(agent: AgentRef) -> Agents {
    Agents::from_vec(vec![agent])
}

fn claude() -> Agents {
    with(Arc::new(Claude::new("/nonexistent".into())))
}

fn why(cfg: &str, agents: &Agents) -> (Judging, String) {
    judge(&Config::parse(cfg).unwrap(), agents)
        .map(|_| ())
        .unwrap_err()
}

#[test]
fn a_proven_agent_judges_unless_the_config_says_otherwise() {
    assert!(judge(&Config::default(), &claude()).is_ok());
    assert_eq!(
        why("[clock]\nenabled = false", &claude()),
        (
            Judging::Off,
            "the steering clock is off ([clock] enabled)".into()
        )
    );
    assert_eq!(
        why("[clock]\nllm = false", &claude()),
        (Judging::Off, "[clock] llm is off".into())
    );
    let (state, reason) = why("[summary]\nenabled = false", &claude());
    assert_eq!(state, Judging::Off);
    assert!(reason.contains("[summary] enabled is off"));
}

#[test]
fn no_agent_means_unavailable() {
    let (state, reason) = why("", &Agents::default());
    assert_eq!(state, Judging::Unavailable);
    assert!(reason.contains("no enabled agent"));
}

#[test]
fn opencode_judges_now_that_its_lock_down_is_checked_on_a_real_install() {
    let opencode = with(Arc::new(OpenCode::new("/nonexistent".into())));
    assert_eq!(
        judge(&Config::default(), &opencode).unwrap().id(),
        "opencode"
    );
}

#[test]
fn an_agent_whose_run_is_unproven_is_not_asked_and_the_reason_names_it() {
    let unproven = with(Arc::new(Unproven(Arc::new(Claude::new(
        "/nonexistent".into(),
    )))));
    let (state, reason) = why("", &unproven);
    assert_eq!(state, Judging::Unavailable);
    assert!(
        reason.starts_with("Claude Code is not used for judging"),
        "{reason}"
    );
}
