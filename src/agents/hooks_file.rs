//! Editing the JSON file an agent keeps its hooks in (`settings.json`, `hooks.json`). eye adds one
//! entry per event and removes only its own; every other entry stays as it was. The old file is
//! backed up before it is written, and a file that is not valid JSON is never touched.

use crate::files::{backup_name, line_diff, pretty_json, write_file};
use crate::util::{shell_quote, show_path};
use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// What one agent's hooks file looks like to eye.
pub struct Spec {
    /// The file's name, for messages.
    pub file: &'static str,
    /// The events eye listens to.
    pub events: &'static [&'static str],
    /// The agent named on the hook command (`hook <Event> --agent codex`); none is the form that
    /// means Claude Code.
    pub agent: Option<&'static str>,
    /// The group eye adds for an event, given the hook command and the event.
    pub group: fn(command: &str, event: &str) -> Value,
}

/// Is this the command of an eye hook (`<bin> hook <Event>`)?
pub fn is_eye_hook_command(cmd: &str) -> bool {
    let toks: Vec<String> = cmd
        .split_whitespace()
        .map(|t| t.trim_matches(|c| c == '\'' || c == '"').to_string())
        .collect();
    toks.windows(2).any(|w| {
        [crate::install::ALIAS_NAME, crate::install::BIN_NAME]
            .iter()
            .any(|n| w[0] == *n || w[0].ends_with(&format!("/{n}")))
            && w[1] == "hook"
    })
}

/// The hook command line: `<bin> hook <Event>`, and `--agent <id>` when the spec names one.
pub fn hook_command(bin: &Path, event: &str, agent: Option<&str>) -> String {
    let b = bin.to_string_lossy();
    let b = if b.contains(|c: char| c.is_whitespace() || c == '\'') {
        shell_quote(&b)
    } else {
        b.into_owned()
    };
    match agent {
        Some(a) => format!("{b} hook {event} --agent {a}"),
        None => format!("{b} hook {event}"),
    }
}

/// Does this group of an event hold a hook of eye's?
pub fn group_has_eye(g: &Value) -> bool {
    g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
        hs.iter().any(|h| {
            h.get("command")
                .and_then(Value::as_str)
                .is_some_and(is_eye_hook_command)
        })
    })
}

/// What a merge did: events that got an eye entry, and events whose eye entry was brought up to
/// date (a new binary path, a new field). Other people's hooks are never touched.
#[derive(Debug, Default, PartialEq)]
pub struct Merged {
    pub added: Vec<&'static str>,
    pub updated: Vec<&'static str>,
}

impl Merged {
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.updated.is_empty()
    }
}

/// Add one eye entry per event; an event that already has one keeps its place, and its eye
/// handler is replaced when it differs from what this version writes.
pub fn merge(settings: &mut Value, bin: &Path, spec: &Spec) -> Result<Merged> {
    let file = spec.file;
    let root = settings
        .as_object_mut()
        .with_context(|| format!("{file} is not a JSON object"))?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .with_context(|| format!("{file} \"hooks\" is not an object"))?;
    let mut done = Merged::default();
    for ev in spec.events {
        let arr = hooks
            .entry(*ev)
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
            .with_context(|| format!("{file} hooks.{ev} is not an array"))?;
        let want = (spec.group)(&hook_command(bin, ev, spec.agent), ev);
        let Some(g) = arr.iter_mut().find(|g| group_has_eye(g)) else {
            arr.push(want);
            done.added.push(*ev);
            continue;
        };
        let handler = g
            .get_mut("hooks")
            .and_then(Value::as_array_mut)
            .and_then(|hs| {
                hs.iter_mut().find(|h| {
                    h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(is_eye_hook_command)
                })
            });
        if let (Some(h), Some(w)) = (handler, want.pointer("/hooks/0")) {
            if h != w {
                *h = w.clone();
                done.updated.push(*ev);
            }
        }
    }
    Ok(done)
}

/// Remove eye's hook entries and whatever became empty because of it.
pub fn remove(settings: &mut Value) -> usize {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
    let mut removed = 0;
    let keys: Vec<String> = hooks.keys().cloned().collect();
    for k in keys {
        let Some(arr) = hooks.get_mut(&k).and_then(Value::as_array_mut) else {
            continue;
        };
        let mut i = 0;
        while i < arr.len() {
            if !group_has_eye(&arr[i]) {
                i += 1;
                continue;
            }
            if let Some(hs) = arr[i].get_mut("hooks").and_then(Value::as_array_mut) {
                let before = hs.len();
                hs.retain(|h| {
                    !h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(is_eye_hook_command)
                });
                removed += before - hs.len();
                if hs.is_empty() {
                    arr.remove(i);
                    continue;
                }
            }
            i += 1;
        }
        if arr.is_empty() {
            hooks.remove(&k);
        }
    }
    if hooks.is_empty() {
        if let Some(o) = settings.as_object_mut() {
            o.remove("hooks");
        }
    }
    removed
}

/// The events of the spec that have no eye entry in `settings`.
pub fn missing(settings: &Value, spec: &Spec) -> Vec<&'static str> {
    spec.events
        .iter()
        .copied()
        .filter(|ev| {
            !settings
                .pointer(&format!("/hooks/{ev}"))
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(group_has_eye))
        })
        .collect()
}

/// The file's JSON and its text; an empty object and no text when the file is not there.
pub fn read(p: &Path) -> Result<(Value, String)> {
    match std::fs::read_to_string(p) {
        Ok(t) => {
            let v: Value = serde_json::from_str(&t)
                .with_context(|| format!("{} is not valid JSON; not touching it", p.display()))?;
            Ok((v, t))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((Value::Object(Map::new()), String::new()))
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
    }
}

/// Is the file already in the form eye writes (2-space JSON)? Then a rewrite changes nothing but
/// the hooks.
fn is_canonical(v: &Value, text: &str) -> bool {
    text.is_empty() || pretty_json(v).trim_end() == text.trim_end()
}

fn backup_and_write(
    p: &Path,
    old_text: &str,
    new_text: &str,
    file: &str,
) -> Result<Option<PathBuf>> {
    let mut bak = None;
    if !old_text.is_empty() {
        let b = backup_name(p);
        std::fs::copy(p, &b).with_context(|| format!("backing up to {}", b.display()))?;
        bak = Some(b);
    }
    let target = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mode = std::fs::metadata(&target)
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(0o600);
    write_file(&target, new_text.as_bytes(), mode)?;
    let back: Value = serde_json::from_str(&std::fs::read_to_string(p)?)
        .with_context(|| format!("the written {file} does not parse"))?;
    anyhow::ensure!(back.is_object(), "the written {file} is not an object");
    Ok(bak)
}

/// What changing a hooks file did, or in a dry run would do.
#[derive(Debug, Default)]
pub struct Change {
    /// The diff, a line for each line added or removed.
    pub diff: Vec<String>,
    /// The copy of the old file, when one was made.
    pub backup: Option<PathBuf>,
    /// The file was not 2-space JSON and is rewritten in that form: every entry is kept, the
    /// layout is not.
    pub reformatted: bool,
}

/// Change the file from `before` to `after`, after a backup of the old one. Nothing is written in
/// a dry run.
pub fn change(
    p: &Path,
    spec: &Spec,
    before: &Value,
    old_text: &str,
    after: &Value,
    dry_run: bool,
) -> Result<Change> {
    let file = spec.file;
    let new_text = pretty_json(after);
    let reformatted = !is_canonical(before, old_text);
    let base = if reformatted {
        pretty_json(before)
    } else {
        old_text.to_string()
    };
    let diff = line_diff(&base, &new_text);
    let backup = if dry_run {
        None
    } else {
        backup_and_write(p, old_text, &new_text, file)?
    };
    Ok(Change {
        diff,
        backup,
        reformatted,
    })
}

impl Change {
    /// What happened to the file besides the hooks: the backup, and a layout that changed. Said in
    /// a dry run with "would".
    pub fn notes(&self, dry_run: bool) -> Vec<String> {
        let mut notes = vec![];
        if let Some(b) = &self.backup {
            notes.push(format!("backup {}", show_path(b)));
        }
        if self.reformatted {
            notes.push(if dry_run {
                "would be rewritten as 2-space JSON, after a backup".to_string()
            } else {
                "rewritten as 2-space JSON".to_string()
            });
        }
        notes
    }
}

/// `text`, then the notes in brackets when there are any.
fn with_notes(text: String, notes: Vec<String>) -> String {
    if notes.is_empty() {
        text
    } else {
        format!("{text} ({})", notes.join(", "))
    }
}

/// The summary of adding eye's hooks: `hooks ok (8 events)`, or what was added and where.
pub fn added_text(spec: &Spec, path: &Path, merged: &Merged, change: &Change, dry: bool) -> String {
    if !merged.changed() {
        return format!("hooks ok ({} events)", spec.events.len());
    }
    let what = |evs: &[&str]| {
        if evs.len() == spec.events.len() {
            format!("{} events", evs.len())
        } else {
            evs.join(", ")
        }
    };
    let mut parts = vec![];
    if !merged.added.is_empty() {
        let w = what(&merged.added);
        parts.push(if dry {
            format!("would add hooks ({w})")
        } else {
            format!("hooks added ({w})")
        });
    }
    if !merged.updated.is_empty() {
        let w = what(&merged.updated);
        parts.push(if dry {
            format!("would update hooks ({w})")
        } else {
            format!("hooks updated ({w})")
        });
    }
    // "hooks added (...) to ~/x", "hooks updated (...) in ~/x", or both
    let place = if merged.updated.is_empty() {
        "to"
    } else {
        "in"
    };
    let text = format!("{} {place} {}", parts.join(", "), show_path(path));
    with_notes(text, change.notes(dry))
}

/// The summary of taking eye's hooks out: `hooks removed (8 entries) from ~/...`.
pub fn removed_text(path: &Path, removed: usize, change: &Change, dry: bool) -> String {
    if removed == 0 {
        return "no hooks found".to_string();
    }
    let text = if dry {
        format!(
            "would remove hooks ({removed} entries) from {}",
            show_path(path)
        )
    } else {
        format!("hooks removed ({removed} entries) from {}", show_path(path))
    };
    with_notes(text, change.notes(dry))
}

/// A one-entry hook group in the form both agents read: `{"hooks": [<handler>]}` with `extra`
/// keys first (Claude Code's `matcher`).
pub fn group(extra: &[(&str, Value)], handler: Value) -> Value {
    let mut m = Map::new();
    for (k, v) in extra {
        m.insert((*k).into(), v.clone());
    }
    m.insert("hooks".into(), json!([handler]));
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: Spec = Spec {
        file: "settings.json",
        events: &["Stop", "SessionEnd"],
        agent: None,
        group: |command, _| group(&[], json!({"type": "command", "command": command})),
    };

    #[test]
    fn detects_eye_entries_only() {
        assert!(is_eye_hook_command("/Users/x/.local/bin/eye hook Stop"));
        assert!(is_eye_hook_command(
            "/Users/x/.local/bin/agent-eye hook Stop"
        ));
        assert!(is_eye_hook_command("'/a b/eye' hook Stop"));
        assert!(is_eye_hook_command("/b/agent-eye hook Stop --agent codex"));
        assert!(!is_eye_hook_command("/opt/rtk hook claude"));
        assert!(!is_eye_hook_command("/x/eyes hook Stop"));
    }

    #[test]
    fn the_hook_command_names_the_agent_only_when_asked() {
        let bin = Path::new("/Users/you/.local/bin/agent-eye");
        assert_eq!(
            hook_command(bin, "Stop", None),
            "/Users/you/.local/bin/agent-eye hook Stop"
        );
        assert_eq!(
            hook_command(bin, "Stop", Some("codex")),
            "/Users/you/.local/bin/agent-eye hook Stop --agent codex"
        );
        assert_eq!(
            hook_command(Path::new("/a b/agent-eye"), "Stop", None),
            "'/a b/agent-eye' hook Stop"
        );
    }

    #[test]
    fn a_settings_file_that_is_a_link_stays_a_link() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("dotfiles/settings.json");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "{}\n").unwrap();
        let link = t.path().join("settings.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        backup_and_write(&link, "{}\n", "{\n  \"a\": 1\n}\n", "settings.json").unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "{\n  \"a\": 1\n}\n"
        );
    }

    #[test]
    fn an_outdated_eye_entry_is_updated_in_place_and_the_rest_is_left_alone() {
        let bin = Path::new("/b/agent-eye");
        let mut v = json!({"hooks": {}});
        merge(&mut v, bin, &SPEC).unwrap();
        // an older version wrote the Stop handler differently; a user hook sits beside it
        let stop = v
            .pointer_mut("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array_mut()
            .unwrap();
        stop[0]["timeout"] = json!(30);
        stop.push(json!({"type": "command", "command": "/opt/x/notify stop"}));
        let m = merge(&mut v, bin, &SPEC).unwrap();
        assert_eq!(
            (m.added.len(), m.updated.as_slice()),
            (0, ["Stop"].as_slice())
        );
        let hs = v
            .pointer("/hooks/Stop/0/hooks")
            .unwrap()
            .as_array()
            .unwrap();
        let want = (SPEC.group)(&hook_command(bin, "Stop", SPEC.agent), "Stop");
        assert_eq!(hs[0], want["hooks"][0]);
        assert_eq!(
            hs[1]["command"], "/opt/x/notify stop",
            "the user's hook stays"
        );
        assert!(!merge(&mut v, bin, &SPEC).unwrap().changed());
    }

    #[test]
    fn merge_adds_one_entry_per_event_and_remove_takes_them_out() {
        let mut v = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "/opt/x/notify stop"}]}]}});
        let before = v.clone();
        let bin = Path::new("/b/agent-eye");
        assert_eq!(
            merge(&mut v, bin, &SPEC).unwrap().added,
            ["Stop", "SessionEnd"]
        );
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(missing(&v, &SPEC).is_empty());
        assert!(
            !merge(&mut v, bin, &SPEC).unwrap().changed(),
            "a second merge changes nothing"
        );
        assert_eq!(remove(&mut v), 2);
        assert_eq!(v, before);
        assert_eq!(missing(&v, &SPEC), ["Stop", "SessionEnd"]);
    }

    #[test]
    fn a_wrong_shape_is_named_after_the_file() {
        for (bad, says) in [
            (json!([]), "settings.json is not a JSON object"),
            (
                json!({"hooks": []}),
                "settings.json \"hooks\" is not an object",
            ),
            (
                json!({"hooks": {"Stop": "x"}}),
                "settings.json hooks.Stop is not an array",
            ),
        ] {
            let mut v = bad;
            let e = merge(&mut v, Path::new("/b/agent-eye"), &SPEC).unwrap_err();
            assert_eq!(e.to_string(), says);
        }
    }
}
