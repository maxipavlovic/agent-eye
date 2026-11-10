//! Summaries through `codex exec`: one question, answered once, leaving nothing behind in Codex.

use super::Codex;
use crate::agents::{HeadlessReply, HeadlessRequest};
use crate::util::{head_chars, new_uuid};
use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// What a summary run needs `codex exec` to have. Older versions lack some of them.
const FLAGS: [&str; 6] = [
    "--ephemeral",
    "--ignore-user-config",
    "--ignore-rules",
    "--skip-git-repo-check",
    "--output-schema",
    "--output-last-message",
];

/// Keywords of a JSON schema that Codex's structured output does not take. eye cuts every answer
/// to length itself.
const UNSUPPORTED: [&str; 4] = ["maxLength", "minLength", "maxItems", "minItems"];

/// The programs already found to have every flag.
static CHECKED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// Does `codex exec` know the flags a summary needs? Asked once per program, with `--help`.
pub fn flags_check(exe: &str) -> Result<()> {
    let mut checked = CHECKED.lock().unwrap_or_else(|e| e.into_inner());
    if checked.contains(exe) {
        return Ok(());
    }
    let out = Command::new(exe)
        .args(["exec", "--help"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("starting `{exe}` (is it on PATH?)"))?;
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let missing: Vec<&str> = FLAGS.into_iter().filter(|f| !help.contains(f)).collect();
    anyhow::ensure!(
        missing.is_empty(),
        "this Codex CLI has no {} for `codex exec`: update Codex, or let another agent write the summaries ([summary] agent in config.toml)",
        missing.join(", ")
    );
    checked.insert(exe.to_string());
    Ok(())
}

/// A JSON schema in the strict form Codex's structured output wants: every object closed
/// (`additionalProperties: false`) with all its properties required, and no keyword it rejects.
pub fn strict(schema: &Value) -> Value {
    match schema {
        Value::Object(o) => {
            let mut out = Map::new();
            for (k, v) in o {
                if UNSUPPORTED.contains(&k.as_str()) {
                    continue;
                }
                out.insert(
                    k.clone(),
                    match (k.as_str(), v) {
                        // the names of properties are not keywords
                        ("properties", Value::Object(props)) => Value::Object(
                            props.iter().map(|(n, p)| (n.clone(), strict(p))).collect(),
                        ),
                        _ => strict(v),
                    },
                );
            }
            if let Some(Value::Object(props)) = out.get("properties") {
                let all: Vec<Value> = props.keys().map(|k| json!(k)).collect();
                out.insert("required".into(), Value::Array(all));
                out.insert("additionalProperties".into(), json!(false));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(strict).collect()),
        v => v.clone(),
    }
}

/// A folder of its own for one run, below the work folder, removed when the run is over.
struct Scratch(PathBuf);

impl Scratch {
    fn new(work: &Path) -> Result<Scratch> {
        std::fs::create_dir_all(work).with_context(|| format!("creating {}", work.display()))?;
        let dir = work.join(format!("codex-{}", new_uuid()?));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn drain<R: Read + Send + 'static>(r: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = BufReader::new(r).read_to_end(&mut v);
        v
    })
}

/// Run `exe args` in `dir` with `stdin` as its input; what it printed on stderr when it failed.
fn run(exe: &str, args: &[String], dir: &Path, stdin: String, timeout: Duration) -> Result<()> {
    let mut child = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting `{exe}` (is it on PATH?)"))?;
    let mut input = child.stdin.take().context("no stdin")?;
    // written on its own thread: a prompt larger than the pipe must not block the wait below
    let writer = std::thread::spawn(move || {
        let _ = input.write_all(stdin.as_bytes());
    });
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("codex timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = writer.join();
    let _ = out.map(|h| h.join());
    let err = err
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    anyhow::ensure!(
        status.success(),
        "codex exited with {status}: {}",
        head_chars(String::from_utf8_lossy(&err).trim(), 300)
    );
    Ok(())
}

/// The JSON in what the last message says, which may be wrapped in a code fence.
fn answer(text: &str) -> Result<Value> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .map_or(t, |r| r.trim_end_matches("```").trim());
    serde_json::from_str(t).with_context(|| {
        format!(
            "codex did not answer with JSON: {}",
            head_chars(text.trim(), 200)
        )
    })
}

/// Ask Codex one structured question: `codex exec` in a read-only sandbox, with no session kept
/// (`--ephemeral`), none of the user's config, hooks or rules (`--ignore-user-config`,
/// `--ignore-rules`) and a low reasoning effort. The prompt goes in on stdin, the schema and the
/// answer through files in a folder of their own.
pub fn ask(c: &Codex, req: &HeadlessRequest) -> Result<HeadlessReply> {
    let exe = c.executable_path();
    flags_check(&exe)?;
    let scratch = Scratch::new(req.work_dir)?;
    let (schema, reply) = (scratch.0.join("schema.json"), scratch.0.join("answer.json"));
    std::fs::write(&schema, serde_json::to_vec(&strict(req.schema))?)?;
    let mut args: Vec<String> = [
        "exec",
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--skip-git-repo-check",
        "-s",
        "read-only",
    ]
    .map(String::from)
    .to_vec();
    if !req.model.is_empty() {
        args.extend(["-m".into(), req.model.into()]);
    }
    args.extend([
        "-c".into(),
        "model_reasoning_effort=low".into(),
        "--output-schema".into(),
        schema.to_string_lossy().into_owned(),
        "-o".into(),
        reply.to_string_lossy().into_owned(),
        "-".into(),
    ]);
    let prompt = format!("{}\n\n{}", req.system.trim_end(), req.input);
    let started = Instant::now();
    run(&exe, &args, &scratch.0, prompt, req.timeout)?;
    let text = std::fs::read_to_string(&reply).context("codex wrote no answer")?;
    Ok(HeadlessReply {
        output: answer(&text)?,
        duration_ms: started.elapsed().as_millis() as i64,
        usage: Value::Null,
        cost_usd: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_schema_is_made_strict() {
        let s = strict(&crate::summarize::schema());
        let objects = [&s, &s["properties"]["topics"]["items"]];
        for o in objects {
            assert_eq!(o["additionalProperties"], false);
            let props: Vec<&str> = o["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            let required: Vec<&str> = o["required"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            assert_eq!(props, required, "every property is required");
        }
        let text = s.to_string();
        for gone in UNSUPPORTED {
            assert!(!text.contains(gone), "{gone}");
        }
        assert_eq!(s["properties"]["status"]["enum"][0], "done", "enums stay");
        assert_eq!(s["properties"]["topics"]["type"], "array");
    }

    #[test]
    fn a_property_named_like_a_keyword_is_kept() {
        let s =
            strict(&json!({"type": "object", "properties": {"maxLength": {"type": "integer"}}}));
        assert_eq!(s["properties"]["maxLength"]["type"], "integer");
        assert_eq!(s["required"][0], "maxLength");
    }

    #[test]
    fn the_answer_may_come_in_a_code_fence() {
        assert_eq!(answer("{\"a\":1}").unwrap()["a"], 1);
        assert_eq!(answer("```json\n{\"a\":2}\n```").unwrap()["a"], 2);
        assert_eq!(answer("```\n{\"a\":3}\n```\n").unwrap()["a"], 3);
        let e = answer("Sorry, I cannot.").unwrap_err().to_string();
        assert!(
            e.contains("did not answer with JSON") && e.contains("Sorry"),
            "{e}"
        );
    }
}
