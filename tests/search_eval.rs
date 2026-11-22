//! Quality of the picker's ranking on a private, labelled copy of a real index. Not run by default:
//! `EYE_EVAL_DIR=<dir with eye.db, config.toml, labels.json> cargo test --release --test search_eval -- --ignored --nocapture`
//! Each label is `{id, kind, q, ranks: {<8-char session id>: "1"|"2"|"3"|"ok"|"no"}}`.
//! Set `EYE_EVAL_MIN_FIRST` to fail when fewer queries than that put the first choice on top.
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::pick::{search, Index};
use agent_eye::util::now_ms;
use serde_json::Value;
use std::path::PathBuf;

/// Gain of a label for nDCG.
fn gain(label: &str) -> f64 {
    match label {
        "1" => 4.0,
        "2" => 3.0,
        "3" => 2.0,
        "ok" => 1.0,
        _ => 0.0,
    }
}

/// Discounted gain of a list of gains.
fn dcg(gains: &[f64]) -> f64 {
    gains
        .iter()
        .enumerate()
        .map(|(i, g)| g / (i as f64 + 2.0).log2())
        .sum()
}

#[test]
#[ignore]
fn ranking_quality() {
    let dir = PathBuf::from(std::env::var("EYE_EVAL_DIR").expect("EYE_EVAL_DIR"));
    let db = Db::open(&dir.join("eye.db")).unwrap();
    let cfg = Config::load_or_create(&dir.join("config.toml")).unwrap();
    let idx = Index::load(&db, &cfg, now_ms()).unwrap();
    let labels: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("labels.json")).unwrap()).unwrap();
    let labels = labels.as_array().expect("labels.json is a list");
    let (mut first, mut top3, mut missing, mut ndcg) = (0, 0, 0, 0.0);
    let mut misses = vec![];
    for l in labels {
        let id = l["id"].as_str().unwrap_or("?");
        let q = l["q"].as_str().unwrap_or("");
        let ranks = l["ranks"].as_object().expect("ranks");
        let label_of = |sid: &str| {
            ranks
                .iter()
                .find(|(p, _)| sid.starts_with(p.as_str()))
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("")
        };
        let got: Vec<String> = search(&db, &idx, q, false)
            .into_iter()
            .map(|i| idx.rows[i].id.clone())
            .collect();
        let pos = got.iter().position(|s| label_of(s) == "1");
        match pos {
            Some(0) => first += 1,
            _ => misses.push(format!("{id} (first choice at {:?})", pos.map(|p| p + 1))),
        }
        if pos.is_some_and(|p| p < 3) {
            top3 += 1;
        }
        if pos.is_none() {
            missing += 1;
        }
        let top: Vec<f64> = got.iter().take(5).map(|s| gain(label_of(s))).collect();
        let mut ideal: Vec<f64> = ranks
            .values()
            .filter_map(|v| v.as_str())
            .map(gain)
            .collect();
        ideal.sort_by(|a, b| b.total_cmp(a));
        ideal.truncate(5);
        let best = dcg(&ideal);
        if best > 0.0 {
            ndcg += dcg(&top) / best;
        }
    }
    let n = labels.len();
    println!(
        "first {first}/{n}  top3 {top3}/{n}  missing {missing}  nDCG@5 {:.3}",
        ndcg / n as f64
    );
    for m in &misses {
        println!("miss: {m}");
    }
    if let Some(min) = std::env::var("EYE_EVAL_MIN_FIRST")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        assert!(first >= min, "first {first} is below {min}");
    }
}
