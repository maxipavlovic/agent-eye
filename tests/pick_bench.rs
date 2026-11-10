//! Per-keystroke timing on a copy of the real index:
//! `EYE_BENCH_HOME=<dir with eye.db> cargo test --release --test pick_bench -- --ignored --nocapture`
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::pick::{search, Index};
use agent_eye::util::now_ms;
use std::time::Instant;

#[test]
#[ignore]
fn keystroke_timing() {
    let home = std::env::var("EYE_BENCH_HOME").expect("EYE_BENCH_HOME");
    let home = std::path::PathBuf::from(home);
    let db = Db::open(&home.join("eye.db")).unwrap();
    let cfg = Config::load_or_create(&home.join("config.toml")).unwrap();
    let t = Instant::now();
    let idx = Index::load(&db, &cfg, now_ms()).unwrap();
    println!(
        "index load: {:.1} ms ({} rows)",
        t.elapsed().as_secs_f64() * 1e3,
        idx.rows.len()
    );
    let mut worst: f64 = 0.0;
    for q in [
        "catalog migration",
        "seat licenses",
        "PROJ-35",
        "pr",
        "a",
        "https",
        "terminal picker",
        "web page simplification",
        "proj 123 cert",
        "weekly rep",
    ] {
        let mut line = String::new();
        let mut prev = vec![];
        for c in q.chars() {
            line.push(c);
            let t = Instant::now();
            let r = search(&db, &idx, &line, false, now_ms());
            let ms = t.elapsed().as_secs_f64() * 1e3;
            worst = worst.max(ms);
            prev.push(format!("{ms:.1}"));
            let _ = r;
        }
        println!("{q:<30} ms per key: {}", prev.join(" "));
    }
    println!("worst keystroke: {worst:.1} ms");
}
