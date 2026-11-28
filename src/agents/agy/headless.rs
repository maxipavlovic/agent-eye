//! Summaries through `agy -p`: print mode with `--output-format json` and `--json-schema`, which
//! answers with one JSON object (`status`, `response`, `structured_output`, `usage`). agy has no
//! flag to leave no conversation behind, so the run happens in a folder named `eye-ask-<uuid>`,
//! the scanner leaves conversations in such folders out, and the conversation is deleted after the
//! call (`forget_ask`): its folder, database and row in `conversation_summaries.db`, which hold the
//! prompt. A run that goes wrong or is killed is cleaned up the same way, by the folder it ran in.

use super::files::{self, ASK_PREFIX, BRAIN};
use super::Agy;
use crate::agents::{HeadlessReply, HeadlessRequest};
use crate::util::{head_chars, new_uuid, valid_session_id};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The file in the work folder that one question at a time holds (`flock`).
const GATE: &str = "agy-ask.lock";

/// Wait for the turn to ask agy, and keep it until the returned file is dropped. A question's
/// clean-up leaves `history.jsonl` and the caches alone while any agy conversation is open
/// (`files::delete`), and a question running beside it is one: questions side by side
/// (`[summary] concurrency`) could leave the masked excerpt in agy's up-arrow history. The lock
/// is a file, so a question from another eye process waits too.
fn one_at_a_time(work_dir: &Path) -> Result<File> {
    let gate = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(work_dir.join(GATE))
        .context("opening the lock of agy's questions")?;
    gate.lock().context("waiting for another question to agy")?;
    Ok(gate)
}

/// The folder a question runs in. When it goes out of scope the question's conversation is deleted
/// from agy's folder (`agy`), then the folder itself.
struct Scratch {
    path: PathBuf,
    agy: PathBuf,
    /// The conversation the answer named, when there was one.
    conversation: Option<String>,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        forget_ask(
            &self.agy,
            name.as_deref().unwrap_or_default(),
            self.conversation.as_deref(),
        );
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Delete what a question left in agy's folder: the conversation the answer named and every
/// conversation started in the question's folder (`folder_name`), each with its database and its row
/// in `conversation_summaries.db`. Best effort, like the folder's removal: the answer is already in
/// hand. While agy is open `history.jsonl` is not edited (see `files::delete`).
fn forget_ask(agy: &Path, folder_name: &str, answered: Option<&str>) -> Vec<String> {
    let mut ids: Vec<String> = answered.map(String::from).into_iter().collect();
    if folder_name.starts_with(ASK_PREFIX) {
        ids.extend(
            files::summaries_with_folders(agy)
                .into_iter()
                .filter(|(_, s)| {
                    s.folder
                        .as_deref()
                        .and_then(|f| Path::new(f).file_name())
                        .is_some_and(|n| n.to_string_lossy() == folder_name)
                })
                .map(|(id, _)| id),
        );
    }
    ids.sort();
    ids.dedup();
    ids.iter()
        .filter(|id| valid_session_id(id))
        .flat_map(|id| files::delete(agy, &agy.join(BRAIN).join(id), id).unwrap_or_default())
        .collect()
}

/// The JSON in a reply that may be wrapped in a code fence.
fn fenced_json(text: &str) -> Result<Value> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .map_or(t, |r| r.trim_end_matches("```").trim());
    serde_json::from_str(t).with_context(|| {
        format!(
            "agy did not answer with JSON: {}",
            head_chars(text.trim(), 200)
        )
    })
}

/// What `agy -p --output-format json` printed, as the reply.
pub fn reply_of(stdout: &str, duration_ms: i64) -> Result<HeadlessReply> {
    let v: Value = serde_json::from_str(stdout.trim())
        .with_context(|| format!("agy printed no JSON: {}", head_chars(stdout.trim(), 200)))?;
    let status = v.get("status").and_then(Value::as_str).unwrap_or("");
    anyhow::ensure!(
        status == "SUCCESS",
        "agy answered with status {status:?}: {}",
        head_chars(&v.to_string(), 300)
    );
    let output = match v.get("structured_output").filter(|o| o.is_object()) {
        Some(o) => o.clone(),
        None => fenced_json(v.get("response").and_then(Value::as_str).unwrap_or(""))?,
    };
    let u = v.get("usage").cloned().unwrap_or(Value::Null);
    let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
    Ok(HeadlessReply {
        output,
        duration_ms,
        usage: json!({
            "input_tokens": n("input_tokens"),
            "output_tokens": n("output_tokens"),
            "cache_read_input_tokens": n("cache_read_tokens"),
        }),
        cost_usd: None,
    })
}

/// The arguments of a summary run.
fn command_args(prompt: String, schema: &Value, model: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["-p".into(), prompt];
    args.extend(
        [
            "--output-format",
            "json",
            "--disable-slash-commands",
            // a summary needs no tools: plan mode does not edit or run commands, and the sandbox
            // confines whatever still runs (agy's changelog: `--mode plan` applies in `-p` runs;
            // `--sandbox` is a flag without a value). No `--print-timeout`: its unit is not
            // documented and `--help` was not run here; the loop below kills agy at `req.timeout`.
            "--mode",
            "plan",
            "--sandbox",
            "--json-schema",
        ]
        .map(String::from),
    );
    args.push(schema.to_string());
    if !model.is_empty() {
        args.extend(["--model".into(), model.into()]);
    }
    args
}

pub fn ask(a: &Agy, req: &HeadlessRequest) -> Result<HeadlessReply> {
    std::fs::create_dir_all(req.work_dir)?;
    // declared before the scratch folder, so it is let go after the folder's clean-up
    let _turn = one_at_a_time(req.work_dir)?;
    let dir = req.work_dir.join(format!("{ASK_PREFIX}{}", new_uuid()?));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let mut scratch = Scratch {
        path: dir,
        agy: a.dir.clone(),
        conversation: None,
    };
    let mut prompt = format!("{}\n\n{}", req.system.trim_end(), req.input);
    if prompt.starts_with('-') {
        prompt.insert_str(0, "Task: ");
    }
    let args = command_args(prompt, req.schema, req.model);
    let exe = a.executable_path();
    let started = Instant::now();
    let mut child = Command::new(&exe)
        .args(&args)
        .current_dir(&scratch.path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting `{exe}` (is it on PATH?)"))?;
    let drain = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut v);
            }
            v
        })
    };
    let out = drain(child.stdout.take().map(|s| Box::new(s) as _));
    let err = drain(child.stderr.take().map(|s| Box::new(s) as _));
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if started.elapsed() > req.timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("agy timed out after {}s", req.timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    scratch.conversation = conversation_of(&out);
    anyhow::ensure!(
        status.success(),
        "agy exited with {status}: {}",
        head_chars(err.trim(), 300)
    );
    reply_of(&out, started.elapsed().as_millis() as i64)
}

/// The id of the conversation `agy -p` says it made.
fn conversation_of(stdout: &str) -> Option<String> {
    serde_json::from_str::<Value>(stdout.trim())
        .ok()?
        .get("conversation_id")?
        .as_str()
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_that_stop_tools_are_in_the_command() {
        let args = command_args("p".into(), &json!({"type":"object"}), "m");
        let at = |f: &str| args.iter().position(|a| a == f).unwrap();
        assert_eq!(args[at("--mode") + 1], "plan");
        assert!(args.contains(&"--sandbox".to_string()));
        assert_eq!(args[at("--model") + 1], "m");
        // the flag value of --json-schema is the next argument, not --sandbox
        assert_eq!(args[at("--json-schema") + 1], "{\"type\":\"object\"}");
    }

    #[test]
    fn the_structured_output_is_the_answer() {
        let out = json!({"conversation_id":"c","status":"SUCCESS","response":"{\"a\":1}\n",
            "structured_output":{"a":1},"usage":{"input_tokens":9,"output_tokens":2,"cache_read_tokens":4}});
        let r = reply_of(&out.to_string(), 5).unwrap();
        assert_eq!(r.output["a"], 1);
        assert_eq!(r.usage["input_tokens"], 9);
        assert_eq!(r.usage["cache_read_input_tokens"], 4);
        assert!(r.cost_usd.is_none());
    }

    #[test]
    fn a_fenced_response_is_read_and_a_failure_is_an_error() {
        let out = json!({"status":"SUCCESS","response":"```json\n{\"b\":2}\n```"});
        assert_eq!(reply_of(&out.to_string(), 1).unwrap().output["b"], 2);
        let bad = json!({"status":"ERROR","response":""});
        assert!(reply_of(&bad.to_string(), 1)
            .unwrap_err()
            .to_string()
            .contains("status"));
        assert!(reply_of("not json", 1).is_err());
        let prose = json!({"status":"SUCCESS","response":"Sorry."});
        assert!(reply_of(&prose.to_string(), 1)
            .unwrap_err()
            .to_string()
            .contains("did not answer"));
    }

    fn conversation(agy: &Path, id: &str) {
        std::fs::create_dir_all(agy.join(BRAIN).join(id)).unwrap();
        std::fs::create_dir_all(agy.join("conversations")).unwrap();
        std::fs::write(agy.join("conversations").join(format!("{id}.db")), "x").unwrap();
    }

    fn summaries(agy: &Path, rows: &[(&str, &Path)]) {
        let c = rusqlite::Connection::open(agy.join("conversation_summaries.db")).unwrap();
        c.execute(
            "CREATE TABLE conversation_summaries(conversation_id TEXT, title TEXT, workspace_uris TEXT,
             parent_conversation_id TEXT, last_modified_time TEXT, source TEXT, agent_name TEXT, status TEXT)",
            [],
        )
        .unwrap();
        for (id, folder) in rows {
            let uris = json!([format!("file://{}", folder.display())]).to_string();
            c.execute(
                "INSERT INTO conversation_summaries VALUES(?1, 'masked prompt', ?2, '', '', '', '', '')",
                rusqlite::params![id, uris],
            )
            .unwrap();
        }
    }

    fn left(agy: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(agy.join(BRAIN))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn rows(agy: &Path) -> Vec<String> {
        let c = rusqlite::Connection::open(agy.join("conversation_summaries.db")).unwrap();
        let mut st = c
            .prepare("SELECT conversation_id FROM conversation_summaries")
            .unwrap();
        st.query_map([], |r| r.get(0)).unwrap().flatten().collect()
    }

    #[test]
    fn the_conversation_a_question_made_is_deleted_with_its_summary_row() {
        let t = tempfile::tempdir().unwrap();
        let agy = t.path().join("agy");
        let ask = t.path().join("work").join(format!("{ASK_PREFIX}1111"));
        let mine = t.path().join("project");
        conversation(&agy, "asked-1");
        conversation(&agy, "mine-1");
        summaries(&agy, &[("asked-1", &ask), ("mine-1", &mine)]);
        let gone = forget_ask(&agy, &format!("{ASK_PREFIX}1111"), None);
        assert!(
            gone.iter().any(|g| g.contains("conversation_summaries.db")),
            "{gone:?}"
        );
        // a person's own conversation stays, a question's goes: folder, database and row
        assert_eq!(left(&agy), ["mine-1"]);
        assert!(!agy.join("conversations/asked-1.db").exists());
        assert!(agy.join("conversations/mine-1.db").exists());
        assert_eq!(rows(&agy), ["mine-1"]);
    }

    #[test]
    fn the_conversation_the_answer_names_is_deleted_even_when_no_row_says_where_it_ran() {
        let t = tempfile::tempdir().unwrap();
        let agy = t.path().join("agy");
        conversation(&agy, "asked-2");
        conversation(&agy, "mine-2");
        summaries(&agy, &[("mine-2", &t.path().join("project"))]);
        forget_ask(&agy, &format!("{ASK_PREFIX}2222"), Some("asked-2"));
        assert_eq!(left(&agy), ["mine-2"]);
        // a name that is not a conversation id never reaches the file system
        assert!(forget_ask(&agy, "x", Some("../mine-2")).is_empty());
        assert_eq!(left(&agy), ["mine-2"]);
    }

    #[test]
    fn a_folder_that_is_not_a_question_deletes_only_what_the_answer_names() {
        let t = tempfile::tempdir().unwrap();
        let agy = t.path().join("agy");
        let mine = t.path().join("project");
        conversation(&agy, "mine-3");
        summaries(&agy, &[("mine-3", &mine)]);
        assert!(forget_ask(&agy, "project", None).is_empty());
        assert_eq!(left(&agy), ["mine-3"]);
    }

    #[test]
    fn the_scratch_folder_cleans_up_when_a_run_fails() {
        let t = tempfile::tempdir().unwrap();
        let agy = t.path().join("agy");
        let work = t.path().join("work");
        let path = work.join(format!("{ASK_PREFIX}3333"));
        std::fs::create_dir_all(&path).unwrap();
        conversation(&agy, "asked-4");
        summaries(&agy, &[("asked-4", &path)]);
        drop(Scratch {
            path: path.clone(),
            agy: agy.clone(),
            conversation: None,
        });
        assert!(!path.exists() && left(&agy).is_empty() && rows(&agy).is_empty());
    }

    /// An `agy` that makes the conversation `asked-run` (as the real one would, in agy's folder),
    /// then does `body`.
    fn stub(t: &Path, agy: &Path, body: &str) -> String {
        let p = t.join("agy-stub");
        let (brain, db) = (agy.join(BRAIN).join("asked-run"), agy.join("conversations"));
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nmkdir -p '{}' '{}'\ntouch '{}/asked-run.db'\n{body}\n",
                brain.display(),
                db.display(),
                db.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn run_stub(body: &str, timeout: Duration) -> (Result<HeadlessReply>, tempfile::TempDir) {
        run_stub_in(tempfile::tempdir().unwrap(), body, timeout)
    }

    fn run_stub_in(
        t: tempfile::TempDir,
        body: &str,
        timeout: Duration,
    ) -> (Result<HeadlessReply>, tempfile::TempDir) {
        let (agy, work) = (t.path().join("agy"), t.path().join("work"));
        let a = Agy::new(agy.clone()).with_executable(&stub(t.path(), &agy, body));
        let schema = json!({"type":"object"});
        let req = HeadlessRequest {
            model: "",
            system: "sys",
            input: "in",
            schema: &schema,
            work_dir: &work,
            timeout,
        };
        (ask(&a, &req), t)
    }

    /// Neither the run's folder nor the conversation it made is left (the lock file stays: it is
    /// one empty file in eye's own work folder).
    fn assert_clean(t: &tempfile::TempDir) {
        let agy = t.path().join("agy");
        assert!(left(&agy).is_empty(), "{:?}", left(&agy));
        assert!(!agy.join("conversations/asked-run.db").exists());
        let work: Vec<_> = std::fs::read_dir(t.path().join("work"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(work, [GATE]);
    }

    /// `[summary] concurrency` asks side by side. A clean-up leaves `history.jsonl` and the caches
    /// alone while any conversation is open (`files::delete`), and a question that is still running
    /// is one: with two at once, the masked excerpt of the first could stay in agy's history.
    #[test]
    fn two_questions_at_once_run_one_after_the_other() {
        let t = tempfile::tempdir().unwrap();
        let (agy, work) = (t.path().join("agy"), t.path().join("work"));
        let marks = t.path().join("marks");
        std::fs::create_dir_all(&marks).unwrap();
        let said = r#"{"conversation_id":"asked-run","status":"SUCCESS","response":"","structured_output":{"a":1}}"#;
        let body = format!(
            "touch '{m}/busy.'$$\nsleep 0.3\nset -- '{m}'/busy.*\n[ $# -gt 1 ] && touch '{m}/overlap'\nrm '{m}/busy.'$$\nprintf '%s' '{said}'",
            m = marks.display()
        );
        let a = Agy::new(agy.clone()).with_executable(&stub(t.path(), &agy, &body));
        let schema = json!({"type":"object"});
        let req = HeadlessRequest {
            model: "",
            system: "sys",
            input: "in",
            schema: &schema,
            work_dir: &work,
            timeout: Duration::from_secs(30),
        };
        std::thread::scope(|s| {
            let runs: Vec<_> = (0..2).map(|_| s.spawn(|| ask(&a, &req))).collect();
            for run in runs {
                assert_eq!(run.join().unwrap().unwrap().output["a"], 1);
            }
        });
        assert!(!marks.join("overlap").exists(), "two questions ran at once");
    }

    /// agy has no flag to keep a run from making a conversation, so every way a question can end
    /// removes what it made: its folder always, its conversation when the answer names it.
    #[test]
    fn a_question_leaves_nothing_after_success_failure_and_timeout() {
        let said = r#"printf '%s' '{"conversation_id":"asked-run","status":"SUCCESS","response":"","structured_output":{"a":1}}'"#;
        let (reply, t) = run_stub(said, Duration::from_secs(30));
        assert_eq!(reply.unwrap().output["a"], 1);
        assert_clean(&t);

        let failed = format!("{said}; echo oops >&2; exit 3");
        let (reply, t) = run_stub(&failed, Duration::from_secs(30));
        assert!(reply.is_err());
        assert_clean(&t);

        // a run that times out never says which conversation it made: the row agy wrote for the
        // question's folder is how it is found
        let t = tempfile::tempdir().unwrap();
        let (agy, work) = (t.path().join("agy"), t.path().join("work"));
        let writer = std::thread::spawn(move || {
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(10));
                // the question's folder, not the lock file that sits beside it
                let folder = std::fs::read_dir(&work).ok().and_then(|d| {
                    d.flatten().map(|e| e.path()).find(|p| {
                        p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with(ASK_PREFIX))
                    })
                });
                if let Some(folder) = folder {
                    std::fs::create_dir_all(&agy).unwrap();
                    summaries(&agy, &[("asked-run", &folder)]);
                    return;
                }
            }
        });
        let (reply, t) = run_stub_in(t, "exec sleep 5", Duration::from_millis(1500));
        writer.join().unwrap();
        assert!(reply.unwrap_err().to_string().contains("timed out"));
        assert_clean(&t);
        assert!(rows(&t.path().join("agy")).is_empty());
    }

    #[test]
    fn the_answers_conversation_id_is_read() {
        assert_eq!(
            conversation_of(r#"{"conversation_id":"abc-1","status":"SUCCESS"}"#).as_deref(),
            Some("abc-1")
        );
        assert_eq!(conversation_of("not json"), None);
        assert_eq!(conversation_of(r#"{"status":"ERROR"}"#), None);
    }
}
