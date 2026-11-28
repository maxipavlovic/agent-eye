//! A home of each agent in a temporary folder: the fixture Claude and Codex homes, and an Agy and an
//! OpenCode home made here, each with two small sessions. Nothing real is read.

use agent_eye::config::Config;
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};

pub const AGY_A: &str = "a1111111-1111-4111-8111-111111111111";
pub const AGY_B: &str = "b2222222-2222-4222-8222-222222222222";
pub const OC_A: &str = "ses_v1old";
pub const OC_B: &str = "ses_v1new";

pub struct Homes {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub agy: PathBuf,
    pub opencode: PathBuf,
    /// All four agents on, each looking in its folder here.
    pub cfg: Config,
}

fn sql(c: &Connection, q: &str, p: impl rusqlite::Params) {
    c.execute(q, p).unwrap();
}

pub fn homes(tmp: &Path) -> Homes {
    let claude = super::fixtures::claude_home(tmp);
    let fix = |n: &str| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(n)
    };
    let codex = tmp.join("codex");
    super::fixtures::copy_dir(&fix("codex-home"), &codex);
    let agy = tmp.join("agy");
    super::fixtures::copy_dir(&fix("agy-home"), &agy);
    std::fs::remove_file(agy.join("databases.json")).unwrap();
    std::fs::create_dir_all(agy.join("conversations")).unwrap();
    for id in [AGY_A, AGY_B] {
        let c = Connection::open(agy.join(format!("conversations/{id}.db"))).unwrap();
        c.execute_batch(
            "CREATE TABLE gen_metadata (idx integer PRIMARY KEY, data blob, size integer NOT NULL DEFAULT 0);",
        )
        .unwrap();
    }
    let c = Connection::open(agy.join("conversation_summaries.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE conversation_summaries (conversation_id text PRIMARY KEY, title text NOT NULL DEFAULT '',
         last_modified_time datetime NOT NULL, workspace_uris text NOT NULL, parent_conversation_id text NOT NULL DEFAULT '');",
    )
    .unwrap();
    for (id, t) in [(AGY_A, "Quokka census"), (AGY_B, "Other chat")] {
        sql(
            &c,
            "INSERT INTO conversation_summaries VALUES(?1,?2,'2026-11-05 14:05:10.123456+00:00','[\"file:///work/zoo\"]','')",
            (id, t),
        );
    }
    drop(c);
    let opencode = tmp.join("opencode");
    std::fs::create_dir_all(&opencode).unwrap();
    let c = Connection::open(opencode.join("opencode.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE session (id text PRIMARY KEY, project_id text, directory text NOT NULL, parent_id text, title text, time_created integer, time_updated integer);
         CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer, data text NOT NULL);
         CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer, data text NOT NULL);",
    )
    .unwrap();
    for (id, mid, text) in [
        (OC_A, "m_a", "Ping the old server"),
        (OC_B, "m_b", "Ping the new one"),
    ] {
        sql(
            &c,
            "INSERT INTO session VALUES(?1,'p','/work/oc',NULL,'t',1792000000000,1792000001000)",
            [id],
        );
        sql(
            &c,
            "INSERT INTO message VALUES(?1,?2,1792000000000,?3)",
            (
                mid,
                id,
                json!({"role":"user","time":{"created":1792000000000i64}}).to_string(),
            ),
        );
        sql(
            &c,
            "INSERT INTO part VALUES(?1,?2,?3,1,?4)",
            (
                format!("p_{mid}"),
                mid,
                id,
                json!({"type":"text","text":text}).to_string(),
            ),
        );
    }
    drop(c);
    let mut cfg = Config::parse(&format!(
        "[agents]\nenabled = [\"claude\", \"codex\", \"agy\", \"opencode\"]\n[agents.codex]\ndir = \"{}\"\n[agents.agy]\ndir = \"{}\"\n[agents.opencode]\ndir = \"{}\"\n",
        codex.display(),
        agy.display(),
        opencode.display()
    ))
    .unwrap();
    cfg.override_agent_dir("claude", claude.clone());
    Homes {
        claude,
        codex,
        agy,
        opencode,
        cfg,
    }
}
