//! How well the corrections pass tells pushback from everything else, on private, labelled prompts.
//! Not run by default:
//! `EYE_EVAL_DIR=<dir with clock_labels.json> EYE_EVAL_MODEL=haiku cargo test --release --test clock_eval -- --ignored --nocapture`
//! Each label is `{id, session_id, prompt_ts, correction}`. The exchanges are read, read-only, from
//! the transcripts under `~/.claude` (or `EYE_CLAUDE_DIR`), through the Claude adapter, and sent
//! through the same call the nightly pass makes. Set `EYE_EVAL_MIN` (percent) to fail when
//! precision or recall is lower.
use agent_eye::agents::claude::Claude;
use agent_eye::agents::Agent;
use agent_eye::clock::detect::{self, Exchange};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The transcript of a session: `<dir>/projects/<project>/<session id>.jsonl`.
fn transcript(dir: &Path, sid: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir.join("projects"))
        .ok()?
        .flatten()
        .map(|p| p.path().join(format!("{sid}.jsonl")))
        .find(|p| p.exists())
}

#[test]
#[ignore]
fn corrections_are_told_from_the_rest() {
    let labels_dir = PathBuf::from(std::env::var("EYE_EVAL_DIR").expect("EYE_EVAL_DIR"));
    let model = std::env::var("EYE_EVAL_MODEL").unwrap_or_else(|_| "haiku".into());
    let claude_dir = std::env::var("EYE_CLAUDE_DIR").map_or_else(
        |_| PathBuf::from(std::env::var("HOME").expect("HOME")).join(".claude"),
        PathBuf::from,
    );
    let labels: Value = serde_json::from_str(
        &std::fs::read_to_string(labels_dir.join("clock_labels.json")).unwrap(),
    )
    .unwrap();
    let claude = Claude::new(claude_dir.clone());
    let mut items: Vec<((String, bool), Exchange)> = vec![];
    let mut missing = vec![];
    for l in labels.as_array().expect("clock_labels.json is a list") {
        let (id, sid) = (
            l["id"].as_str().unwrap_or("?"),
            l["session_id"].as_str().unwrap_or(""),
        );
        let ts = l["prompt_ts"].as_i64().unwrap_or(0);
        let found = transcript(&claude_dir, sid)
            .and_then(|p| claude.read_conversation(&p).ok())
            .and_then(|m| {
                // the labels carry the time the prompt history gave, a few ms off the transcript's
                let near = m
                    .iter()
                    .filter(|x| x.role == "user")
                    .filter_map(|x| x.ts)
                    .filter(|t| (t - ts).abs() < 2_000)
                    .min_by_key(|t| (t - ts).abs())?;
                detect::exchange(&m, near)
            });
        match found {
            Some(ex) => items.push(((id.to_string(), l["correction"].as_bool().unwrap()), ex)),
            None => missing.push(id.to_string()),
        }
    }
    let through_filter = items
        .iter()
        .filter(|(_, e)| detect::is_candidate(&e.prompt))
        .count();
    let work = tempfile::tempdir().unwrap();
    let (mut tp, mut fp, mut fneg, mut tn) = (0, 0, 0, 0);
    let mut wrong = vec![];
    for batch in detect::batches(items) {
        let refs: Vec<&Exchange> = batch.iter().map(|(_, e)| e).collect();
        let judged = detect::judge(&claude, &model, work.path(), &refs).expect("the model answers");
        for (((id, truth), _), j) in batch.iter().zip(judged) {
            let said = j.is_some_and(|j| j.correction);
            match (said, *truth) {
                (true, true) => tp += 1,
                (true, false) => fp += 1,
                (false, true) => fneg += 1,
                (false, false) => tn += 1,
            }
            if said != *truth {
                wrong.push(format!("{id} (label {truth}, model {said})"));
            }
        }
    }
    let pct = |a: i32, b: i32| {
        if b == 0 {
            0.0
        } else {
            100.0 * f64::from(a) / f64::from(b)
        }
    };
    let (precision, recall) = (pct(tp, tp + fp), pct(tp, tp + fneg));
    println!(
        "model {model}: {} labelled, {} not found in the transcripts {missing:?}",
        tp + fp + fneg + tn,
        missing.len()
    );
    println!("cue filter lets through {through_filter} of the found exchanges");
    println!(
        "true corrections found {tp}/{}, false alarms {fp}, missed {fneg}, correct no {tn}",
        tp + fneg
    );
    println!(
        "precision {precision:.0}% ({tp}/{}), recall {recall:.0}% ({tp}/{})",
        tp + fp,
        tp + fneg
    );
    println!("wrong: {wrong:?}");
    if let Some(min) = std::env::var("EYE_EVAL_MIN")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        assert!(
            precision >= min && recall >= min,
            "precision {precision:.0}% recall {recall:.0}% under {min}%"
        );
    }
}
