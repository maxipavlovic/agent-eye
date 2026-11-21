//! What agy's own process logs (`log/cli-<time>.log`, one per `agy` process) say about a
//! conversation. Two things are nowhere else on disk:
//!
//! * a `-p` run: `Print mode: ... conversation=<id>, sending message`. The summaries row of such a
//!   conversation looks like any other (its `source` is empty), so this line is how eye knows it
//!   was headless.
//! * the web link of remote control: `[remote-control-<uuid>-v2] ... Connection status: Connected`
//!   in the log of the process that created the conversation, which is shown to the person as
//!   `https://antigravity.google.com/r/<uuid>-v2`. It is the process's link, not the conversation's:
//!   it belongs to every conversation that process opened while the connection is up.
//!
//! Logs are read whole (at most 16 MiB each) and the result is kept until the file changes.

use crate::util::valid_session_id;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MAX_LOG: u64 = 16 << 20;
const MAX_FILES: usize = 600;
const LINK: &str = "https://antigravity.google.com/r/";

/// What one log says.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Conversations this process created or streamed.
    pub conversations: Vec<String>,
    /// Conversations this process ran in print mode.
    pub print: Vec<String>,
    /// The remote-control id (`<uuid>-v2`) and whether its connection is up at the end of the log.
    pub remote: Option<(String, bool)>,
}

type Cache = HashMap<PathBuf, (u64, i64, Arc<Entry>)>;
static CACHE: std::sync::LazyLock<Mutex<Cache>> =
    std::sync::LazyLock::new(|| Mutex::new(Cache::new()));

/// The id that follows `marker` in `line` (36 characters of a uuid).
fn id_after(line: &str, marker: &str) -> Option<String> {
    let rest = &line[line.find(marker)? + marker.len()..];
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
        .collect();
    (id.len() == 36 && valid_session_id(&id)).then_some(id)
}

/// Read one log's text.
pub fn parse(text: &str) -> Entry {
    let mut e = Entry::default();
    let push = |v: &mut Vec<String>, id: String| {
        if !v.contains(&id) {
            v.push(id);
        }
    };
    for line in text.lines() {
        if let Some(id) = id_after(line, "Created conversation ")
            .or_else(|| id_after(line, "Streaming conversation "))
        {
            push(&mut e.conversations, id);
        }
        if let Some(id) = id_after(line, "Print mode: conversation=") {
            push(&mut e.print, id);
        }
        if let Some(i) = line.find("[remote-control-") {
            let rest = &line[i + "[remote-control-".len()..];
            if let Some(rc) = rest.split(']').next().filter(|r| r.ends_with("-v2")) {
                let rc = rc.to_string();
                let up = if line.contains("Connection status: Connected") {
                    Some(true)
                } else if line.contains("Connection ended")
                    || line.contains("Connection status: Disconnected")
                {
                    Some(false)
                } else {
                    None
                };
                match (&mut e.remote, up) {
                    (Some((id, state)), Some(u)) if *id == rc => *state = u,
                    (slot, u) => *slot = Some((rc, u.unwrap_or(false))),
                }
            }
        }
        if line.contains("Remote control disabled") {
            if let Some((_, up)) = &mut e.remote {
                *up = false;
            }
        }
    }
    e
}

fn read(path: &Path) -> Option<Arc<Entry>> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_LOG {
        return None;
    }
    let mtime = crate::util::mtime_ms(&meta);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((len, m, e)) = cache.get(path) {
        if *len == meta.len() && *m == mtime {
            return Some(e.clone());
        }
    }
    let text = String::from_utf8_lossy(&std::fs::read(path).ok()?).into_owned();
    let e = Arc::new(parse(&text));
    cache.insert(path.to_path_buf(), (meta.len(), mtime, e.clone()));
    Some(e)
}

/// The logs of `dir`, newest first.
pub fn entries(dir: &Path) -> Vec<Arc<Entry>> {
    let Ok(rd) = std::fs::read_dir(dir.join("log")) else {
        return vec![];
    };
    let mut names: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with("cli-") && n.ends_with(".log")
        })
        .map(|e| e.path())
        .collect();
    names.sort_by(|a, b| b.cmp(a));
    names.truncate(MAX_FILES);
    names.iter().filter_map(|p| read(p)).collect()
}

/// Did agy run this conversation in print mode (`-p`)?
pub fn headless(dir: &Path, id: &str) -> bool {
    entries(dir).iter().any(|e| e.print.iter().any(|p| p == id))
}

/// The remote-control link of the newest process that opened conversation `id`, while its
/// connection is up.
pub fn remote_url(dir: &Path, id: &str) -> Option<String> {
    let e = entries(dir)
        .into_iter()
        .find(|e| e.conversations.iter().any(|c| c == id))?;
    match &e.remote {
        Some((rc, true)) => Some(format!("{LINK}{rc}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
I1121 10:00:01.022774 447 remote_control_v2.go:2108] [remote-control-11111111-2222-3333-4444-555555555555-v2] Starting V2 remote control connection to: https://x:443
I1121 10:00:01.027804 297 server.go:1269] Created conversation aaaaaaaa-1111-2222-3333-444444444444 (backend=BACKEND_TYPE_UNSPECIFIED)
I1121 10:00:01.409304 447 remote_control_v2.go:2235] [remote-control-11111111-2222-3333-4444-555555555555-v2] Connection status: Connected
I1121 10:00:02.000000 1 printmode.go:209] Print mode: starting (promptLength=3, model=\"m\", conversationID=\"\")
I1121 10:00:03.000000 1 session.go:201] Print mode: conversation=bbbbbbbb-1111-2222-3333-444444444444, sending message
";

    #[test]
    fn a_log_names_its_conversations_print_runs_and_link() {
        let e = parse(LOG);
        assert_eq!(e.conversations, ["aaaaaaaa-1111-2222-3333-444444444444"]);
        assert_eq!(e.print, ["bbbbbbbb-1111-2222-3333-444444444444"]);
        assert_eq!(
            e.remote,
            Some(("11111111-2222-3333-4444-555555555555-v2".into(), true))
        );
    }

    #[test]
    fn a_connection_that_ended_has_no_link() {
        let more = format!("{LOG}I1121 10:09:00.0 447 x.go:1] [remote-control-11111111-2222-3333-4444-555555555555-v2] Connection ended. Retrying immediately...\n");
        assert!(!parse(&more).remote.unwrap().1);
        let off = format!("{LOG}I1121 10:09:00.0 1 server.go:1] Remote control disabled, s\n");
        assert!(!parse(&off).remote.unwrap().1);
    }

    #[test]
    fn a_log_without_remote_control_has_none() {
        let e = parse("Created conversation aaaaaaaa-1111-2222-3333-444444444444 (x)\n");
        assert_eq!(e.remote, None);
        assert!(parse("garbage\n\u{0}\n").conversations.is_empty());
    }
}
