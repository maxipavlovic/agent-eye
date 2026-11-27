//! What every `opencode` command eye runs is given, so that none of them can act, update OpenCode
//! or reach OpenCode's background service. Checked on a real OpenCode 2.3.2 with a user config that
//! allows everything and declares an MCP server (`docs/security.md`):
//!
//! - the config is inline (`OPENCODE_CONFIG_CONTENT`), which OpenCode applies after every config of
//!   the user's, so its rules come last and win; it is never written to a file;
//! - a question runs as an agent of that config (`eye-ask`), never the user's default one: OpenCode
//!   applies an agent's own permissions after the global ones, so a user's `agent.build`
//!   permissions beat a global deny (a real run did every planted step that way);
//! - `--standalone` gives each command a private server: OpenCode's background service runs with
//!   the config it started with, and one started by eye would keep eye's;
//! - the user's MCP servers are named in the config and switched off: OpenCode starts every
//!   configured server for a run, even with every tool denied;
//! - no update, no sharing, no `~/.claude/CLAUDE.md` read as instructions, and no project config or
//!   `AGENTS.md` from the folders above the run's folder.

use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// The agent a question runs as. Only the run's own config defines it.
pub const AGENT: &str = "eye-ask";

/// The variable OpenCode reads its inline config from.
pub const CONFIG_VAR: &str = "OPENCODE_CONFIG_CONTENT";

/// The switches every command gets besides the config.
pub const ENV: [(&str, &str); 3] = [
    ("OPENCODE_DISABLE_AUTOUPDATE", "1"),
    ("OPENCODE_DISABLE_CLAUDE_CODE", "1"),
    ("OPENCODE_DISABLE_PROJECT_CONFIG", "1"),
];

/// Everything denied. OpenCode starts every agent from a rule that allows everything, so a list of
/// denied tools would leave the rest open (an MCP server's tools, code mode); the `*` rule denies
/// all of them, and the named keys say it again for the ones OpenCode has always had.
fn deny() -> Value {
    let rules: Map<String, Value> = [
        "*",
        "bash",
        "edit",
        "webfetch",
        "websearch",
        "external_directory",
        "task",
        "skill",
    ]
    .into_iter()
    .map(|k| (k.to_string(), json!("deny")))
    .collect();
    Value::Object(rules)
}

/// The inline config: every permission denied by default and for [`AGENT`] itself, every tool off,
/// the MCP servers `mcp` names switched off (a server is switched off by a whole entry; the
/// command is never run), no sharing and no update. Nothing in it allows or asks anything, and the
/// run is not given `--auto`, so nothing is approved on its behalf.
pub fn config(mcp: &[String]) -> String {
    let mut c = json!({
        "permission": deny(),
        "tools": {"*": false},
        "agent": {AGENT: {"mode": "primary", "permission": deny(), "tools": {"*": false}}},
        "share": "disabled",
        "autoupdate": false,
    });
    if !mcp.is_empty() {
        let off: Map<String, Value> = mcp
            .iter()
            .map(|n| {
                let entry = json!({"type": "local", "command": ["true"], "enabled": false});
                (n.clone(), entry)
            })
            .collect();
        c["mcp"] = Value::Object(off);
    }
    c.to_string()
}

/// The names of the MCP servers in what `opencode debug config` prints: `mcp.servers.<name>` in
/// each config source of OpenCode 2, `mcp.<name>` in the one merged config of 1.x.
pub fn mcp_names(debug_config: &str) -> Vec<String> {
    fn walk(v: &Value, names: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    match (k.as_str(), x.as_object()) {
                        ("mcp", Some(o)) => {
                            let servers = o.get("servers").and_then(Value::as_object).unwrap_or(o);
                            names.extend(servers.keys().cloned());
                        }
                        _ => walk(x, names),
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, names)),
            _ => {}
        }
    }
    let mut names = BTreeSet::new();
    if let Ok(v) = serde_json::from_str::<Value>(debug_config.trim()) {
        walk(&v, &mut names);
    }
    names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_names_are_read_from_both_shapes_of_debug_config() {
        // OpenCode 2.3.2: a list of sources, each with its servers under `mcp.servers`
        let v2 = r#"[{"type":"document","path":"/x/opencode.json","info":{"mcp":{"timeout":5,
            "servers":{"jira":{"type":"local","command":["j"],"disabled":false}}}}},
            {"type":"directory","path":"/x","info":{}},
            {"type":"document","info":{"mcp":{"servers":{"docs":{"type":"remote","url":"u"}}}}}]"#;
        assert_eq!(mcp_names(v2), ["docs", "jira"]);
        // 1.x: the merged config, servers right under `mcp`
        let v1 = r#"{"mcp":{"probe":{"type":"local","command":["p"]}},"agent":{}}"#;
        assert_eq!(mcp_names(v1), ["probe"]);
        assert!(mcp_names("[]").is_empty());
        assert!(mcp_names("Error: unknown command").is_empty());
    }

    #[test]
    fn a_named_server_is_switched_off_by_a_whole_entry() {
        let c: Value = serde_json::from_str(&config(&["jira".into()])).unwrap();
        assert_eq!(c["mcp"]["jira"]["enabled"], false);
        // OpenCode 2.3.2 drops an entry without a type and a command, and the server then starts
        assert_eq!(c["mcp"]["jira"]["type"], "local");
        assert!(c["mcp"]["jira"]["command"].is_array());
        let none: Value = serde_json::from_str(&config(&[])).unwrap();
        assert!(none.get("mcp").is_none());
    }
}
