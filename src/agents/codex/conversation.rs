//! A Codex rollout as a conversation (for the board and the picker) and as the tail that goes
//! into a summary excerpt.

use super::rollout::{exit_code, patch_files, read_line, Event, Tool};
use crate::agents::TailItem;
use crate::conversation::Message;
use crate::util::{iso_to_ms, tail_chars};
use anyhow::Result;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::BufRead;

const MSG_CAP: usize = 1_500;
const TAIL_TEXTS: usize = 30;
const TAIL_ITEMS_MAX: usize = 90;

fn lines<R: BufRead + ?Sized>(r: &mut R, mut each: impl FnMut(Option<i64>, Event)) -> Result<()> {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if r.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        let line = buf.strip_suffix(b"\n").unwrap_or(&buf);
        if let Some((v, e)) = read_line(line) {
            let ts = v
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(iso_to_ms);
            each(ts, e);
        }
    }
}

/// The agent's lines since the last prompt.
#[derive(Default)]
struct Turn {
    texts: Vec<String>,
    tools: u32,
    ts: Option<i64>,
}

impl Turn {
    fn flush(&mut self, out: &mut Vec<Message>) {
        let t = std::mem::take(self);
        if !t.texts.is_empty() || t.tools > 0 {
            out.push(Message::new(
                "assistant",
                &t.texts.join("\n\n"),
                t.ts,
                t.tools,
            ));
        }
    }
}

/// The user's prompts and Codex's replies (the replies of one turn joined, with the tool-call
/// count). Injected context and tool output are left out.
pub fn conversation(r: &mut dyn BufRead) -> Result<Vec<Message>> {
    let mut out = vec![];
    let mut turn = Turn::default();
    lines(r, |ts, e| match e {
        Event::Prompt(text) => {
            turn.flush(&mut out);
            out.push(Message::new("user", &text, ts, 0));
        }
        Event::Reply(text) => {
            turn.texts.push(text);
            turn.ts = ts.or(turn.ts);
        }
        Event::Call { tool, .. } | Event::Done { tool, .. } => {
            // a code mode program is as many calls as can be read from it, and at least one
            turn.tools += match tool {
                Tool::Script(calls) => calls.len().max(1) as u32,
                _ => 1,
            };
            turn.ts = ts.or(turn.ts);
        }
        _ => {}
    })?;
    turn.flush(&mut out);
    Ok(out)
}

fn one_line(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// The lines a tool call is in the tail: `$ <command>`, `edit <files>` or its name. A code mode
/// program is the calls read from it, or just `exec` when none can be.
fn tool_lines(tool: &Tool) -> Vec<String> {
    let edit = |files: &[&str]| {
        (!files.is_empty()).then(|| format!("edit {}", one_line(&files.join(", "), 120)))
    };
    let line = match tool {
        Tool::Shell(cmd) => Some(format!("$ {}", one_line(cmd, 120))),
        Tool::Patch(patch) => edit(&patch_files(patch)),
        Tool::Files(files) => edit(&files.iter().map(String::as_str).collect::<Vec<_>>()),
        Tool::Script(calls) if calls.is_empty() => Some("exec".into()),
        Tool::Script(calls) => return calls.iter().flat_map(tool_lines).collect(),
        Tool::Named(name) => Some(one_line(name, 120)),
        Tool::Other => None,
    };
    line.into_iter().collect()
}

/// What a failed command said.
fn failure(code: i64, said: &str) -> String {
    format!("error: exit {code} {}", one_line(said, 160))
}

/// The last ~30 messages with the commands, edits and failures between them.
pub fn tail(r: &mut dyn BufRead) -> Result<Vec<TailItem>> {
    let mut items: VecDeque<TailItem> = VecDeque::new();
    let mut texts = 0usize;
    lines(r, |_, e| {
        let tools = |l: Vec<String>| l.into_iter().map(TailItem::Tool).collect::<Vec<_>>();
        let new = match e {
            Event::Prompt(t) => vec![TailItem::Text {
                role: "user",
                text: tail_chars(&t, MSG_CAP),
            }],
            Event::Reply(t) => vec![TailItem::Text {
                role: "assistant",
                text: tail_chars(&t, MSG_CAP),
            }],
            Event::Call { tool, .. } => tools(tool_lines(&tool)),
            Event::Done { tool, output, exit } => {
                let mut v = tool_lines(&tool);
                v.extend(exit.filter(|c| *c != 0).map(|c| failure(c, &output)));
                tools(v)
            }
            Event::Output { text, .. } => match exit_code(&text) {
                Some(code) if code != 0 => {
                    let said = text.split_once("Output:").map_or("", |(_, o)| o);
                    tools(vec![failure(code, said)])
                }
                _ => vec![],
            },
            Event::Compacted => tools(vec!["context compacted".into()]),
            _ => vec![],
        };
        for it in new {
            if matches!(it, TailItem::Text { .. }) {
                texts += 1;
            }
            items.push_back(it);
        }
        while texts > TAIL_TEXTS || items.len() > TAIL_ITEMS_MAX {
            match items.pop_front() {
                Some(TailItem::Text { .. }) => texts -= 1,
                Some(_) => {}
                None => break,
            }
        }
    })?;
    Ok(items.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::TEXT_CAP;
    use serde_json::json;
    use std::io::Cursor;

    fn ev(kind: &str, payload: Value) -> String {
        json!({"timestamp": "2026-11-09T14:00:00.000Z", "type": kind, "payload": payload})
            .to_string()
    }
    fn user(t: &str) -> String {
        ev("event_msg", json!({"type": "user_message", "message": t}))
    }
    fn reply(t: &str) -> String {
        ev("event_msg", json!({"type": "agent_message", "message": t}))
    }
    fn call(cmd: &str) -> String {
        ev(
            "response_item",
            json!({"type": "function_call", "name": "exec_command", "arguments": json!({"cmd": cmd}).to_string(), "call_id": "c"}),
        )
    }
    fn input(lines: &[String]) -> Cursor<Vec<u8>> {
        Cursor::new(lines.join("\n").into_bytes())
    }

    #[test]
    fn turns_join_replies_and_count_tool_calls() {
        let m = conversation(&mut input(&[
            user("first question"),
            reply("Let me look."),
            call("ls"),
            call("cat a"),
            reply("Done."),
            user("second"),
            reply("Sure."),
        ]))
        .unwrap();
        let got: Vec<(&str, &str, u32)> = m
            .iter()
            .map(|x| (x.role, x.text.as_str(), x.tools))
            .collect();
        assert_eq!(
            got,
            [
                ("user", "first question", 0),
                ("assistant", "Let me look.\n\nDone.", 2),
                ("user", "second", 0),
                ("assistant", "Sure.", 0),
            ]
        );
        assert!(m[0].ts.is_some() && m[1].ts.is_some());
    }

    #[test]
    fn injected_context_and_the_models_own_copy_are_not_in_the_conversation() {
        let m = conversation(&mut input(&[
            ev(
                "response_item",
                json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "# AGENTS.md instructions for /x"}]}),
            ),
            user("<environment_context>\n</environment_context>"),
            user("real"),
            ev(
                "response_item",
                json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "model copy"}]}),
            ),
            reply("answer"),
            ev("compacted", json!({"message": "SECRET-SUMMARY"})),
        ]))
        .unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].text.as_str(), m[1].text.as_str()), ("real", "answer"));
    }

    #[test]
    fn masks_secrets_and_caps_text() {
        let long = "x".repeat(TEXT_CAP + 10);
        let m = conversation(&mut input(&[
            user("use password: hunter2hunter2 please"),
            reply(&long),
        ]))
        .unwrap();
        assert!(m[0].text.contains("[masked]") && !m[0].text.contains("hunter2"));
        assert!(m[1].truncated);
    }

    #[test]
    fn the_tail_has_commands_edits_failures_and_compaction_between_the_messages() {
        let patch = "*** Begin Patch\n*** Update File: a.py\n*** Add File: b.py\n*** End Patch";
        let t = tail(&mut input(&[
            user("go"),
            call("cargo test"),
            ev(
                "response_item",
                json!({"type": "function_call_output", "call_id": "c", "output": "Process exited with code 101\nOutput:\nthread 'x' panicked\n"}),
            ),
            ev(
                "response_item",
                json!({"type": "custom_tool_call", "call_id": "p", "name": "apply_patch", "input": patch}),
            ),
            ev("compacted", json!({"message": "s"})),
            ev(
                "response_item",
                json!({"type": "function_call_output", "call_id": "c", "output": "Process exited with code 0\nOutput:\nok\n"}),
            ),
            reply("fixed"),
        ]))
        .unwrap();
        assert_eq!(
            t,
            [
                TailItem::Text {
                    role: "user",
                    text: "go".into()
                },
                TailItem::Tool("$ cargo test".into()),
                TailItem::Tool("error: exit 101 thread 'x' panicked".into()),
                TailItem::Tool("edit a.py, b.py".into()),
                TailItem::Tool("context compacted".into()),
                TailItem::Text {
                    role: "assistant",
                    text: "fixed".into()
                },
            ]
        );
    }

    /// A line of a paginated rollout: the envelope has an `ordinal`.
    fn pg(kind: &str, payload: Value) -> String {
        json!({"timestamp": "2026-11-09T14:00:00.000Z", "ordinal": 7, "type": kind, "payload": payload})
            .to_string()
    }
    fn done(item: Value) -> String {
        pg(
            "event_msg",
            json!({"type": "item_completed", "turn_id": "t", "item": item}),
        )
    }

    /// What Codex 0.167 writes for a tool call: the model's raw call (a JavaScript program of code
    /// mode) and its output, and the item that says what ran.
    fn paginated_exchange() -> Vec<String> {
        let js = "const r = await tools.exec_command({cmd:\"cargo test\"}); text(r.output);";
        vec![
            pg(
                "event_msg",
                json!({"type": "item_completed", "item": {"type": "UserMessage", "content": [{"type": "text", "text": "go"}]}}),
            ),
            pg(
                "response_item",
                json!({"type": "custom_tool_call", "call_id": "c", "name": "exec", "input": js}),
            ),
            done(
                json!({"type": "CommandExecution", "command": ["/bin/zsh", "-lc", "cargo test"],
                        "cwd": "file:///w/app", "status": "failed", "aggregated_output": "thread 'x' panicked\n", "exit_code": 101}),
            ),
            pg(
                "response_item",
                json!({"type": "custom_tool_call_output", "call_id": "c", "output": [{"type": "input_text", "text": "Script completed\nOutput:\nthread 'x' panicked"}]}),
            ),
            done(json!({"type": "FileChange", "status": "completed",
                        "changes": {"/w/app/a.py": {"type": "update", "unified_diff": "@@", "move_path": null},
                                    "/w/app/b.py": {"type": "add", "content": "x"}}})),
            done(
                json!({"type": "McpToolCall", "server": "jira", "tool": "get_issue", "arguments": {}, "status": "completed"}),
            ),
            done(
                json!({"type": "WebSearch", "id": "w", "query": "rust flock", "action": {"type": "search"}}),
            ),
            pg(
                "event_msg",
                json!({"type": "item_completed", "item": {"type": "AgentMessage", "content": [{"type": "Text", "text": "fixed"}]}}),
            ),
        ]
    }

    #[test]
    fn a_paginated_rollouts_tools_come_from_its_items_and_are_counted_once() {
        let lines = paginated_exchange();
        let m = conversation(&mut input(&lines)).unwrap();
        let got: Vec<(&str, u32)> = m.iter().map(|x| (x.role, x.tools)).collect();
        assert_eq!(
            got,
            [("user", 0), ("assistant", 4)],
            "the command, the files, the MCP tool and the search, not the raw call and its output"
        );
        let t = tail(&mut input(&lines)).unwrap();
        assert_eq!(
            t,
            [
                TailItem::Text {
                    role: "user",
                    text: "go".into()
                },
                TailItem::Tool("$ cargo test".into()),
                TailItem::Tool("error: exit 101 thread 'x' panicked".into()),
                TailItem::Tool("edit /w/app/a.py, /w/app/b.py".into()),
                TailItem::Tool("mcp jira.get_issue".into()),
                TailItem::Tool("web search rust flock".into()),
                TailItem::Text {
                    role: "assistant",
                    text: "fixed".into()
                },
            ]
        );
        for item in &t {
            if let TailItem::Tool(l) = item {
                assert!(!l.contains("await") && !l.contains("tools."), "{l}");
            }
        }
    }

    #[test]
    fn a_code_mode_program_is_shown_as_what_it_runs_and_never_as_source() {
        let exec = |js: &str| {
            ev(
                "response_item",
                json!({"type": "custom_tool_call", "call_id": "c", "name": "exec", "input": js}),
            )
        };
        let lines = [
            user("go"),
            exec(
                r#"const [a, b] = await Promise.all([tools.exec_command({cmd:"cargo test"}), tools.exec_command({cmd:"git status"})]);"#,
            ),
            exec(
                r#"text(await tools.apply_patch("*** Begin Patch\n*** Update File: src/a.rs\n*** End Patch"))"#,
            ),
            exec("const cmd = pick(); await tools.exec_command({cmd});"),
            reply("done"),
        ];
        let m = conversation(&mut input(&lines)).unwrap();
        assert_eq!(
            m[1].tools, 4,
            "two commands, a patch and a program that cannot be read"
        );
        let t = tail(&mut input(&lines)).unwrap();
        let tools: Vec<&str> = t
            .iter()
            .filter_map(|i| match i {
                TailItem::Tool(l) => Some(l.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            tools,
            ["$ cargo test", "$ git status", "edit src/a.rs", "exec"]
        );
    }

    #[test]
    fn the_tail_keeps_the_newest_thirty_messages() {
        let mut lines = vec![];
        for i in 0..40 {
            lines.push(user(&format!("q{i}")));
            lines.push(reply(&format!("a{i}")));
        }
        let t = tail(&mut input(&lines)).unwrap();
        let n = t
            .iter()
            .filter(|i| matches!(i, TailItem::Text { .. }))
            .count();
        assert_eq!(n, 30);
        assert_eq!(
            t.last(),
            Some(&TailItem::Text {
                role: "assistant",
                text: "a39".into()
            })
        );
    }
}
