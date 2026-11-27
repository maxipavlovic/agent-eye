//! Questions asked of a stand-in `opencode` that keeps its sessions in a file and writes down every
//! call: what eye leaves behind, and what every command it runs is given.

use super::{headless, lockdown, OpenCode};
use crate::agents::HeadlessRequest;
use crate::platform::test_support::script;
use serde_json::{json, Value};
use std::time::Duration;

/// An `opencode` that answers like 2.3.2 does. A `run` makes a session (a line in `sessions`) and
/// then does what the file `mode` says: `ok` answers `{"a":1}`, `bad` answers prose, `slow` hangs.
/// `session delete` removes the session; every call is a line of `calls.log`: the arguments, the
/// switches and the inline config.
const OPENCODE: &str = r#"d="$(dirname "$0")"
printf '%s\t%s%s%s\t%s\n' "$*" "$OPENCODE_DISABLE_AUTOUPDATE" "$OPENCODE_DISABLE_CLAUDE_CODE" "$OPENCODE_DISABLE_PROJECT_CONFIG" "$OPENCODE_CONFIG_CONTENT" >> "$d/calls.log"
touch "$d/sessions"
case "$1 $2" in
"run --help") echo "--format --archived --title --agent --model --standalone" ;;
"models "*) printf 'opencode/big-free\nopencode/x-flash-free\n' ;;
"debug config") echo '[{"type":"document","info":{"mcp":{"servers":{"jira":{"type":"local","command":["j"]}}}}}]' ;;
"session list") while read -r id; do printf '%s\teye-ask\t11/27/2026\n' "$id"; done < "$d/sessions" ;;
"session delete") grep -v -x "$3" "$d/sessions" > "$d/left"; mv "$d/left" "$d/sessions" ;;
"session export") echo '{"info":{},"messages":[]}' ;;
"session import") ;;
"--version "*) echo "2.3.2" ;;
run\ *)
  cat > /dev/null
  n=$(($(cat "$d/n" 2>/dev/null || echo 0) + 1)); echo $n > "$d/n"
  echo "ses_$n" >> "$d/sessions"
  case "$(cat "$d/mode")" in
  slow) exec sleep 5 ;;
  bad) t='Sorry, no JSON today.' ;;
  *) t='{\"a\":1}' ;;
  esac
  echo "{\"type\":\"step_start\",\"sessionID\":\"ses_$n\",\"part\":{}}"
  echo "{\"type\":\"text\",\"sessionID\":\"ses_$n\",\"part\":{\"messageID\":\"m\",\"text\":\"$t\"}}" ;;
esac"#;

struct Stub {
    tmp: tempfile::TempDir,
    c: OpenCode,
}

impl Stub {
    fn new() -> Stub {
        let tmp = tempfile::tempdir().unwrap();
        script(tmp.path(), "opencode", OPENCODE);
        let exe = tmp.path().join("opencode");
        let c = OpenCode::new(tmp.path().join("data/opencode"))
            .with_config_dir(tmp.path().join("config/opencode"))
            .with_executable(exe.to_str().unwrap());
        Stub { tmp, c }
    }

    fn ask(&self, mode: &str, timeout: Duration) -> anyhow::Result<Value> {
        std::fs::write(self.tmp.path().join("mode"), mode).unwrap();
        let schema = json!({"type": "object", "required": ["a"]});
        let work = self.tmp.path().join("work");
        let req = HeadlessRequest {
            model: "",
            system: "sys",
            input: "in",
            schema: &schema,
            work_dir: &work,
            timeout,
        };
        headless::ask(&self.c, &req).map(|r| r.output)
    }

    fn sessions_left(&self) -> String {
        std::fs::read_to_string(self.tmp.path().join("sessions")).unwrap_or_default()
    }

    /// Each call: its arguments, its switches and its inline config.
    fn calls(&self) -> Vec<(String, String, Value)> {
        std::fs::read_to_string(self.tmp.path().join("calls.log"))
            .unwrap()
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.splitn(3, '\t').collect();
                let config = serde_json::from_str(f[2]).unwrap_or(Value::Null);
                (f[0].to_string(), f[1].to_string(), config)
            })
            .collect()
    }

    fn work_is_empty(&self) -> bool {
        std::fs::read_dir(self.tmp.path().join("work"))
            .unwrap()
            .next()
            .is_none()
    }
}

/// OpenCode keeps every session a run makes (`--archived` only keeps it out of lists), so every
/// way a question ends deletes what it made: by the ids the event stream names, or, when the run
/// was stopped before it said, by the `eye-ask` sessions of the question's folder.
#[test]
fn a_question_leaves_no_session_after_success_failure_and_timeout() {
    let s = Stub::new();
    assert_eq!(s.ask("ok", Duration::from_secs(30)).unwrap()["a"], 1);
    assert_eq!(s.sessions_left(), "");

    // an invalid answer is asked again; both sessions go
    let e = s.ask("bad", Duration::from_secs(30)).unwrap_err();
    assert!(format!("{e:#}").contains("twice"), "{e:#}");
    assert_eq!(s.sessions_left(), "");
    let deleted = |id: &str| {
        s.calls()
            .iter()
            .any(|(args, _, _)| args.starts_with(&format!("session delete {id} ")))
    };
    assert!(deleted("ses_2") && deleted("ses_3"));

    let e = s.ask("slow", Duration::from_millis(1500)).unwrap_err();
    assert!(e.to_string().contains("timed out"), "{e:#}");
    assert_eq!(s.sessions_left(), "");
    assert!(deleted("ses_4"));
    assert!(s.work_is_empty());
}

/// So that eye never acts through OpenCode, updates it or starts its background service as a side
/// effect: the checks, the model list, the config read, the runs, the session commands and the
/// doctor's version all get the locked-down config, the switches and a private server, and a run
/// also its own agent and the user's MCP servers switched off.
#[test]
fn every_opencode_command_eye_runs_is_locked_down_and_never_updates() {
    let s = Stub::new();
    s.ask("ok", Duration::from_secs(30)).unwrap();
    s.ask("slow", Duration::from_millis(1500)).unwrap_err();
    let _ = s.c.export_native("ses_9");
    let tmp = s.tmp.path().join("import.json");
    s.c.import_native(&json!({"info": {}, "messages": []}), &tmp)
        .unwrap();
    s.c.output(&["--version"], None).unwrap();
    let calls = s.calls();
    for first in [
        "run --help",
        "models",
        "debug config",
        "run --format",
        "session delete",
        "session list",
        "session export",
        "session import",
        "--version",
    ] {
        assert!(
            calls.iter().any(|(a, _, _)| a.starts_with(first)),
            "{first}"
        );
    }
    for (args, switches, config) in &calls {
        assert!(args.split(' ').any(|a| a == "--standalone"), "{args}");
        assert_eq!(switches, "111", "{args}");
        assert_eq!(config["autoupdate"], false, "{args}");
        assert_eq!(config["permission"]["*"], "deny", "{args}");
        assert_eq!(config["share"], "disabled", "{args}");
        if args.starts_with("run --format") {
            assert!(args.contains(&format!("--agent {}", lockdown::AGENT)));
            assert_eq!(config["mcp"]["jira"]["enabled"], false);
        }
    }
    // the free model is the small one the list shows
    assert!(calls
        .iter()
        .any(|(a, _, _)| a.starts_with("run --format") && a.ends_with("-m opencode/x-flash-free")));
    assert!(!tmp.exists());
}
