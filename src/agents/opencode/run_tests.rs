//! Questions asked of a stand-in `opencode` that keeps its sessions in a file and writes down every
//! call: what eye leaves behind, and what every command it runs is given.

use super::{headless, lockdown, OpenCode};
use crate::agents::HeadlessRequest;
use crate::platform::test_support::script;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

/// An `opencode` that answers like 2.3.2 does. A `run` makes a session (a line of `sessions`: id
/// and title) and then does what its prompt says: `SLOW` hangs, `WAIT` answers after 5 s (while
/// `running-<id>` exists), `BAD` answers prose, `NOID` answers without naming its session,
/// anything else `{"a":1}`. `session list` prints the shape of
/// `tests/fixtures/opencode-real/session-list.txt` and notes in `saw_running` how many sessions
/// were running; `session delete` removes the session and notes in `violations` when it was still
/// running. `debug config` fails when the file `debug` says `fail` and prints prose when it says
/// `prose`. Every call is a line of `calls.log`: the arguments, the switches, the inline config
/// and the folder.
const OPENCODE: &str = r#"d="$(dirname "$0")"
lock() { until mkdir "$d/lock" 2>/dev/null; do sleep 0.01; done; }
unlock() { rmdir "$d/lock"; }
printf '%s\t%s%s%s\t%s\t%s\n' "$*" "$OPENCODE_DISABLE_AUTOUPDATE" "$OPENCODE_DISABLE_CLAUDE_CODE" "$OPENCODE_DISABLE_PROJECT_CONFIG" "$OPENCODE_CONFIG_CONTENT" "$PWD" >> "$d/calls.log"
touch "$d/sessions"
case "$1 $2" in
"run --help") echo "--format --archived --title --agent --model --standalone" ;;
"models "*) printf 'opencode/big-free\nopencode/x-flash-free\n' ;;
"debug config")
  case "$(cat "$d/debug" 2>/dev/null)" in
  fail) echo "Error: config is broken" >&2; exit 1 ;;
  prose) echo "Error: config is broken" ;;
  *) echo '[{"type":"document","info":{"mcp":{"servers":{"jira":{"type":"local","command":["j"]}}}}}]' ;;
  esac ;;
"session list")
  ls "$d" | grep -c '^running-' >> "$d/saw_running"
  while IFS="$(printf '\t')" read -r id title; do printf '%s\t%s\t11/27/2026, 9:02:59 AM\n' "$id" "$title"; done < "$d/sessions" ;;
"session delete")
  [ -e "$d/running-$3" ] && echo "$3" >> "$d/violations"
  lock; awk -F '\t' -v id="$3" '$1 != id' "$d/sessions" > "$d/left"; mv "$d/left" "$d/sessions"; unlock ;;
"session export") echo '{"info":{},"messages":[]}' ;;
"session import") ;;
"--version "*) echo "2.3.2" ;;
run\ *)
  title=""; prev=""
  for a in "$@"; do [ "$prev" = --title ] && title="$a"; prev="$a"; done
  input="$(cat)"
  lock; n=$(($(cat "$d/n" 2>/dev/null || echo 0) + 1)); echo $n > "$d/n"
  printf 'ses_%s\t%s\n' "$n" "$title" >> "$d/sessions"; unlock
  case "$input" in *SLOW*) exec sleep 5 ;; esac
  case "$input" in *WAIT*) touch "$d/running-ses_$n"; sleep 5; rm "$d/running-ses_$n" ;; esac
  case "$input" in *BAD*) t='Sorry, no JSON today.' ;; *) t='{\"a\":1}' ;; esac
  case "$input" in *NOID*) echo "{\"type\":\"text\",\"part\":{\"messageID\":\"m\",\"text\":\"$t\"}}"; exit 0 ;; esac
  echo "{\"type\":\"step_start\",\"sessionID\":\"ses_$n\",\"part\":{}}"
  echo "{\"type\":\"text\",\"sessionID\":\"ses_$n\",\"part\":{\"messageID\":\"m\",\"text\":\"$t\"}}" ;;
esac"#;

struct Stub {
    tmp: tempfile::TempDir,
    c: OpenCode,
}

/// One call of the stand-in: its arguments, switches, inline config and folder.
struct Call {
    args: String,
    switches: String,
    config: Value,
    cwd: String,
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

    fn ask(&self, input: &str, timeout: Duration) -> anyhow::Result<Value> {
        let schema = json!({"type": "object", "required": ["a"]});
        let work = self.tmp.path().join("work");
        let req = HeadlessRequest {
            model: "",
            system: "sys",
            input,
            schema: &schema,
            work_dir: &work,
            timeout,
        };
        headless::ask(&self.c, &req).map(|r| r.output)
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.tmp.path().join(name)).unwrap_or_default()
    }

    fn calls(&self) -> Vec<Call> {
        self.file("calls.log")
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.splitn(4, '\t').collect();
                Call {
                    args: f[0].to_string(),
                    switches: f[1].to_string(),
                    config: serde_json::from_str(f[2]).unwrap_or(Value::Null),
                    cwd: f[3].to_string(),
                }
            })
            .collect()
    }

    fn deleted(&self, id: &str) -> bool {
        self.calls()
            .iter()
            .any(|c| c.args.starts_with(&format!("session delete {id} ")))
    }

    /// The one folder every question runs in.
    fn ask_dir(&self) -> PathBuf {
        self.tmp.path().join("work/opencode-ask")
    }
}

/// OpenCode keeps every session a run makes (`--archived` only keeps it out of lists), so every
/// way a question ends deletes what it made: by the ids the event stream names, or, when the run
/// was stopped before it said, by the question's own title in the folder's session list.
#[test]
fn a_question_leaves_no_session_after_success_failure_and_timeout() {
    let s = Stub::new();
    assert_eq!(s.ask("ok", Duration::from_secs(30)).unwrap()["a"], 1);
    assert_eq!(s.file("sessions"), "");

    // an invalid answer is asked again; both sessions go
    let e = s.ask("BAD", Duration::from_secs(30)).unwrap_err();
    assert!(format!("{e:#}").contains("twice"), "{e:#}");
    assert_eq!(s.file("sessions"), "");
    assert!(s.deleted("ses_2") && s.deleted("ses_3"));

    let e = s.ask("SLOW", Duration::from_millis(1500)).unwrap_err();
    assert!(e.to_string().contains("timed out"), "{e:#}");
    assert_eq!(s.file("sessions"), "");
    assert!(s.deleted("ses_4"));
    // the folder stays, private and empty
    let meta = std::fs::metadata(s.ask_dir()).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    assert!(std::fs::read_dir(s.ask_dir()).unwrap().next().is_none());
}

/// Questions run side by side in the same folder (summaries do), so the clean-up of one that timed
/// out must leave alone the session of another that is still running.
#[test]
fn a_timed_out_question_cleans_up_only_its_own_session_in_the_shared_folder() {
    let s = std::sync::Arc::new(Stub::new());
    let other = {
        let s = s.clone();
        std::thread::spawn(move || s.ask("WAIT", Duration::from_secs(30)))
    };
    std::thread::sleep(Duration::from_millis(300));
    let e = s.ask("SLOW", Duration::from_millis(1500)).unwrap_err();
    assert!(e.to_string().contains("timed out"), "{e:#}");
    assert_eq!(other.join().unwrap().unwrap()["a"], 1);
    // the clean-up listed the sessions while the other question's run was going on
    assert!(s.file("saw_running").lines().any(|n| n != "0"));
    assert_eq!(s.file("violations"), "", "a running session was deleted");
    assert_eq!(s.file("sessions"), "");
    let runs: Vec<Call> = s
        .calls()
        .into_iter()
        .filter(|c| c.args.starts_with("run --format"))
        .collect();
    assert_eq!(runs.len(), 2);
    for r in &runs {
        assert_eq!(
            PathBuf::from(&r.cwd).canonicalize().unwrap(),
            s.ask_dir().canonicalize().unwrap()
        );
    }
    let title = |c: &Call| {
        c.args
            .split(' ')
            .skip_while(|a| *a != "--title")
            .nth(1)
            .map(String::from)
    };
    let (a, b) = (title(&runs[0]).unwrap(), title(&runs[1]).unwrap());
    assert!(a.starts_with("eye-ask-") && b.starts_with("eye-ask-") && a != b);
}

/// So that eye never acts through OpenCode, updates it or starts its background service as a side
/// effect: the checks, the model list, the config read, the runs, the session commands and the
/// doctor's version all get the locked-down config, the switches and a private server, and a run
/// also its own agent and the user's MCP servers switched off.
#[test]
fn every_opencode_command_eye_runs_is_locked_down_and_never_updates() {
    let s = Stub::new();
    s.ask("ok", Duration::from_secs(30)).unwrap();
    s.ask("SLOW", Duration::from_millis(1500)).unwrap_err();
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
        assert!(calls.iter().any(|c| c.args.starts_with(first)), "{first}");
    }
    for c in &calls {
        let args = &c.args;
        assert!(args.split(' ').any(|a| a == "--standalone"), "{args}");
        assert_eq!(c.switches, "111", "{args}");
        assert_eq!(c.config["autoupdate"], false, "{args}");
        assert_eq!(c.config["permission"]["*"], "deny", "{args}");
        assert_eq!(c.config["share"], "disabled", "{args}");
        if args.starts_with("run --format") {
            assert!(args.contains(&format!("--agent {}", lockdown::AGENT)));
            assert_eq!(c.config["mcp"]["jira"]["enabled"], false);
        }
        // a question's commands start in its folder, so OpenCode makes no project anywhere else
        if [
            "models",
            "debug",
            "run --format",
            "session delete",
            "session list",
        ]
        .iter()
        .any(|p| args.starts_with(p))
        {
            let cwd = PathBuf::from(&c.cwd).canonicalize().unwrap();
            assert_eq!(cwd, s.ask_dir().canonicalize().unwrap(), "{args}");
        }
    }
    // the free model is the small one the list shows
    assert!(calls.iter().any(
        |c| c.args.starts_with("run --format") && c.args.ends_with("-m opencode/x-flash-free")
    ));
    assert!(!tmp.exists());
}

/// A night run keeps the project's config (its instructions, formatters and MCP servers), so the
/// names of the servers to switch off are read in the run's folder with only updates off.
#[test]
fn a_night_runs_mcp_servers_are_read_in_its_folder_with_the_projects_config() {
    let s = Stub::new();
    let exe = s.tmp.path().join("opencode");
    let folder = s.tmp.path().join("wt");
    std::fs::create_dir_all(&folder).unwrap();
    let names = super::mcp_servers_for_run(exe.to_str().unwrap(), &folder).unwrap();
    assert_eq!(names, ["jira"]);
    let calls = s.calls();
    assert_eq!(calls.len(), 1);
    let c = &calls[0];
    assert_eq!(c.args, "debug config --standalone");
    assert_eq!(
        c.switches, "1",
        "updates off, the project's config and instructions kept"
    );
    assert_eq!(c.config["autoupdate"], false);
    let cwd = PathBuf::from(&c.cwd).canonicalize().unwrap();
    assert_eq!(cwd, folder.canonicalize().unwrap());
}

/// Without the names eye cannot switch the servers off: a failed or unreadable `debug config` is an
/// error, never "no servers".
#[test]
fn mcp_servers_that_cannot_be_read_are_an_error_not_none() {
    let s = Stub::new();
    let exe = s.tmp.path().join("opencode");
    for how in ["fail", "prose"] {
        std::fs::write(s.tmp.path().join("debug"), how).unwrap();
        let e = super::mcp_servers_for_run(exe.to_str().unwrap(), s.tmp.path()).unwrap_err();
        assert!(
            format!("{e:#}").contains("config is broken"),
            "{how}: {e:#}"
        );
    }
}

/// The servers would start without their names (their tools stay denied by `*`), so a question
/// whose servers cannot be read is not asked.
#[test]
fn a_question_is_not_asked_when_its_mcp_servers_cannot_be_read() {
    let s = Stub::new();
    std::fs::write(s.tmp.path().join("debug"), "fail").unwrap();
    let e = s.ask("ok", Duration::from_secs(30)).unwrap_err();
    assert!(format!("{e:#}").contains("nothing was asked"), "{e:#}");
    assert!(!s.calls().iter().any(|c| c.args.starts_with("run --format")));
    assert_eq!(s.file("sessions"), "");
}

/// A run that answers without naming its session is found by its title and deleted all the same.
#[test]
fn a_run_that_does_not_name_its_session_is_cleaned_up_by_its_title() {
    let s = Stub::new();
    assert_eq!(s.ask("NOID", Duration::from_secs(30)).unwrap()["a"], 1);
    assert_eq!(s.file("sessions"), "");
    assert!(s.deleted("ses_1"));
}

/// The clean-up reads `opencode session list` as OpenCode 2.3.2 prints it.
#[test]
fn the_clean_up_finds_a_title_in_the_real_session_list() {
    let list = include_str!("../../../tests/fixtures/opencode-real/session-list.txt");
    let title = "eye-ask-7f1d2e3c-4b5a-4968-8776-655443322110";
    assert_eq!(
        super::cli::titled(list, title),
        ["ses_0aa11b21cffeH8i9J0k1L2m3N4"]
    );
    assert!(super::cli::titled(list, "eye-ask-").is_empty());
    assert!(super::cli::titled(list, "Fix the login").is_empty());
}
