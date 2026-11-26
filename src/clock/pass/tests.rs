use super::*;
use crate::agents::claude::Claude;
use crate::platform::test_support::script;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

const NOW: i64 = 1_793_600_000_000;

/// An answer for each exchange: a correction when the user's line says "wrong".
const ANSWER: &str = r#"echo call >> "$(dirname "$0")/calls.log"
for last; do :; done
printf '%s\n' "$last" | awk 'BEGIN{printf "{\"structured_output\":{"}
/^\[[0-9]+\]$/{id=substr($0,2,length($0)-2)}
/^USER:/{if(n++)printf ","; w=($0 ~ /wrong/); printf "\"%s\":{\"c\":\"%s\",\"what\":\"%s\"}", id, w?"yes":"no", w?"was wrong":""}
END{print "}}"}'"#;

struct Env {
    tmp: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("claude/projects/-tmp-demo")).unwrap();
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        script(&tmp.path().join("bin"), "claude", ANSWER);
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::load_or_create(&paths.config_path()).unwrap();
        cfg.exclude.temp = false;
        cfg.override_agent_dir("claude", tmp.path().join("claude"));
        let db = Db::open(&paths.db_path()).unwrap();
        Env { tmp, paths, cfg, db }
    }

    /// A transcript of `prompts`, each answered, one a minute from 10:00 on 2 Nov 2026 (the
    /// scan reads it and stores the turns).
    fn session(&mut self, sid: &str, prompts: &[String]) {
        let line = |role: &str, i: usize, content: serde_json::Value| {
            let (h, m) = (10 + i / 60, i % 60);
            json!({"type": role, "timestamp": format!("2026-11-02T{h:02}:{m:02}:00.000Z"),
                   "cwd": "/tmp/demo", "sessionId": sid, "message": {"content": content}})
            .to_string()
        };
        let mut text = String::new();
        for (i, p) in prompts.iter().enumerate() {
            text += &line("user", i * 2, json!(p));
            text += "\n";
            text += &line("assistant", i * 2 + 1, json!([{"type": "text", "text": format!("did {i}")}]));
            text += "\n";
        }
        let path = self.tmp.path().join(format!("claude/projects/-tmp-demo/{sid}.jsonl"));
        std::fs::write(path, text).unwrap();
        crate::scan::scan(&mut self.db, &self.cfg, false).unwrap();
    }

    fn agents(&self) -> Agents {
        let exe = self.tmp.path().join("bin/claude");
        Agents::from_vec(vec![Arc::new(
            Claude::new(self.tmp.path().join("claude")).with_executable(exe.to_str().unwrap()),
        )])
    }

    fn run(&self) -> Outcome {
        run(&self.db, &self.paths, &self.cfg, &self.agents(), NOW, &mut |_| {}).unwrap()
    }

    fn calls(&self) -> usize {
        let log: PathBuf = self.tmp.path().join("bin/calls.log");
        std::fs::read_to_string(log).map_or(0, |t| t.lines().count())
    }

    /// (prompt time as a minute of the session, verdict, phrase, model) of every stored verdict.
    fn rows(&self, sid: &str) -> Vec<(i64, String, String, Option<String>)> {
        self.db
            .conn
            .prepare("SELECT prompt_ts, verdict, phrase, model FROM correction WHERE session_id=?1 ORDER BY prompt_ts")
            .unwrap()
            .query_map([sid], |r| {
                let ts: i64 = r.get(0)?;
                Ok(((ts - 1_793_613_600_000) / 60_000, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default(), r.get(3)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
}

fn say(prompts: &[&str]) -> Vec<String> {
    prompts.iter().map(|p| p.to_string()).collect()
}

#[test]
fn only_prompts_with_a_cue_word_go_to_the_model() {
    let mut e = Env::new();
    e.session("s1", &say(&["fix the build", "no, that is wrong", "ok thanks", "why is it wrong again"]));
    let out = e.run();
    assert_eq!((out.sent, out.corrections, out.pending, out.stopped), (2, 2, 0, None));
    assert_eq!(e.calls(), 1);
    let rows = e.rows("s1");
    // the first prompt follows no turn, so it is no candidate and has no row
    assert_eq!(
        rows,
        [
            (2, "yes".to_string(), "was wrong".to_string(), Some("haiku".to_string())),
            (4, "no".to_string(), String::new(), None),
            (6, "yes".to_string(), "was wrong".to_string(), Some("haiku".to_string())),
        ]
    );
}

#[test]
fn nothing_is_judged_twice() {
    let mut e = Env::new();
    e.session("s1", &say(&["fix it", "no, wrong", "carry on"]));
    e.run();
    let again = e.run();
    assert_eq!((again.sent, again.pending), (0, 0));
    assert_eq!(e.calls(), 1);
    // a new prompt is picked up on its own
    e.session("s1", &say(&["fix it", "no, wrong", "carry on", "stop, wrong again"]));
    let next = e.run();
    assert_eq!((next.sent, next.corrections), (1, 1));
    assert_eq!(e.calls(), 2);
}

#[test]
fn a_forked_session_reuses_the_verdicts_of_its_parent() {
    let mut e = Env::new();
    let history = say(&["fix it", "no, that is wrong"]);
    e.session("parent", &history);
    e.session("fork", &history);
    let out = e.run();
    assert_eq!(out.sent, 1);
    assert_eq!(e.calls(), 1);
    assert_eq!(e.rows("fork")[0].1, "yes");
    assert_eq!(out.pending, 0);
}

#[test]
fn a_failed_call_stops_the_pass_and_leaves_those_prompts_pending() {
    let mut e = Env::new();
    script(&e.tmp.path().join("bin"), "claude", "echo 'usage limit reached' >&2; exit 1");
    e.session("s1", &say(&["fix it", "no, wrong", "carry on", "ok"]));
    let out = e.run();
    assert_eq!((out.sent, out.corrections), (0, 0));
    assert!(out.stopped.is_some());
    // the prompts the filter dropped are settled; the one that needed the model waits
    assert_eq!(out.pending, 1);
    assert_eq!(e.rows("s1").len(), 2);
}

#[test]
fn judging_off_asks_nobody() {
    let mut e = Env::new();
    e.cfg.clock.llm = Some(false);
    e.session("s1", &say(&["fix it", "no, wrong"]));
    let out = e.run();
    assert!(out.stopped.unwrap().contains("off"));
    assert_eq!((e.calls(), e.rows("s1").len()), (0, 0));
}

#[test]
fn a_session_in_an_excluded_folder_is_never_sent() {
    let mut e = Env::new();
    e.session("s1", &say(&["fix it", "no, wrong"]));
    e.cfg.exclude.paths = vec!["/tmp/demo".into()];
    let out = e.run();
    assert_eq!((out.sent, out.pending), (0, 0));
    assert_eq!(e.calls(), 0);
    assert_eq!(e.rows("s1")[0].1, "no");
}

#[test]
fn the_newest_prompts_are_judged_first_up_to_the_nightly_limit_in_batches_of_a_hundred() {
    let mut e = Env::new();
    let mut prompts = say(&["start"]);
    prompts.extend((0..310).map(|i| format!("no, wrong {i}")));
    e.session("s1", &prompts);
    let out = e.run();
    assert_eq!((out.sent, out.pending), (NIGHTLY_MAX, 10));
    assert_eq!(e.calls(), 3);
    // the ten oldest wait
    let judged: Vec<i64> = e.rows("s1").iter().map(|r| r.0).collect();
    assert_eq!(judged.len(), 300);
    assert_eq!(judged.iter().min(), Some(&22));
    let tonight = e.run();
    assert_eq!((tonight.sent, tonight.pending), (10, 0));
}
