//! A deterministic, made-up eye home for the end-to-end suite (`e2e/`).
//!
//! `cargo run --example fixture_home -- <dir>` writes under `<dir>`:
//! - `eye/`    the eye home (use as `EYE_HOME`): database, config, no token yet
//! - `home/`   a fake user home (use as `HOME`); the cards name folders under `/Users/demo` instead
//! - `claude/` a fake Claude Code folder with one transcript, for the conversation and the export
//!
//! `cargo run --example fixture_home -- <dir> --proc <pid>` adds one "left behind" process item
//! for a process the caller started, with the key eye itself would store. Nothing here touches
//! the real home of whoever runs it.
//!
//! Time: every stamp is relative to `NOW_MS` (2026-11-20 12:00:00 UTC). Run the server built with
//! `--features fixed-clock` and `EYE_FIXED_NOW_MS=1795176000000`, `TZ=UTC`, and pin the browser
//! clock to the same instant, and ages, due reminders and usage windows are the same every day.

use agent_eye::db::Db;
use agent_eye::loose::{Kind, LooseEnd, Probe, System};
use agent_eye::paths::Paths;
use agent_eye::usage::{limits, LimitWindow, PlanLimits};
use anyhow::{bail, Context, Result};
use rusqlite::params;
use std::path::Path;

/// 2026-11-20 12:00:00 UTC.
const NOW_MS: i64 = 1_795_176_000_000;
const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

/// One made-up session.
struct S {
    title: &'static str,
    project: &'static str,
    branch: &'static str,
    agent: &'static str,
    /// Minutes since the last message.
    age: i64,
    /// `needs_you`, `waiting_on_others`, `paused`, `done`, or "" for a live one.
    ai: &'static str,
    state: &'static str,
    next: &'static str,
    /// `busy`, `idle`, `waiting` or "".
    live: &'static str,
    origin: &'static str,
    prompt: &'static str,
    reply: &'static str,
}

const SESSIONS: &[S] = &[
    S {
        title: "Add dark mode to the settings page",
        project: "web-shop",
        branch: "feature/dark-mode",
        agent: "claude",
        age: 4,
        ai: "",
        state: "Theme switch and tokens are in; profile and address forms converted",
        next: "Claude: convert the billing form, then check contrast",
        live: "busy",
        origin: "",
        prompt: "Add a dark theme to the shop settings page. Use the existing design tokens.",
        reply: "The tokens already have dark values; I am wiring a data-theme switch.",
    },
    S {
        title: "Port the rating engine to Rust",
        project: "customer-billing-reconciliation-service-monorepo",
        branch: "feature/rating-engine-rust-port-phase-two",
        agent: "codex",
        age: 11,
        ai: "",
        state: "Tier maths ported; 31 of 40 golden files match",
        next: "Codex: fix the rounding on the graduated tiers",
        live: "busy",
        origin: "ide",
        prompt: "Port the tiered rating maths to Rust and keep the golden files green.",
        reply: "Ported the flat and volume tiers; graduated tiers round differently.",
    },
    S {
        title: "Retry failed webhook deliveries",
        project: "payments-api",
        branch: "fix/webhook-retries",
        agent: "claude",
        age: 38,
        ai: "needs_you",
        state: "Retry queue with backoff and a dead-letter table are in; 14 tests pass",
        next: "You: choose the max attempts, 5 or 8",
        live: "",
        origin: "terminal",
        prompt: "Webhook deliveries fail for a few minutes after a deploy and are never retried.",
        reply: "How many attempts should we allow before the dead-letter table: 5 or 8?",
    },
    S {
        title: "Fix stale tokens in the account store",
        project: "auth-service",
        branch: "bugfix/stale-tokens",
        agent: "claude",
        age: 21,
        ai: "",
        state: "Soft-expire is written; one permission is waiting",
        next: "You: allow the test command",
        live: "waiting",
        origin: "terminal",
        prompt: "Stale tokens stay valid after a password change. Expire them softly.",
        reply: "I need to run the test suite to check the soft-expire path.",
    },
    S {
        title: "Write the Terraform module for queues",
        project: "infra",
        branch: "main",
        agent: "claude",
        age: 26 * 60,
        ai: "paused",
        state: "Module drafted with queue, dead-letter queue and alarms",
        next: "Claude: write the access policy",
        live: "",
        origin: "terminal",
        prompt: "We need a Terraform module for the job queues.",
        reply: "Drafted modules/queue; which service accounts may send and receive?",
    },
    S {
        title: "Move invoices to the new queue",
        project: "payments-api",
        branch: "feature/invoice-queue",
        agent: "claude",
        age: 3 * 60,
        ai: "waiting_on_others",
        state: "PR 412 opened; invoice jobs run on the queue behind a flag",
        next: "Dana: review PR 412",
        live: "",
        origin: "app",
        prompt: "Invoice generation still runs inside the request. Move it to the job queue.",
        reply: "Opened PR 412 with the queue change and a rollout note.",
    },
    S {
        title: "Review the onboarding flow copy",
        project: "web-shop",
        branch: "docs/onboarding-copy",
        agent: "claude",
        age: 55,
        ai: "waiting_on_others",
        state: "Three screens reviewed; two need shorter words",
        next: "Claude: rewrite the welcome screen",
        live: "idle",
        origin: "web",
        prompt: "Review the onboarding copy and mark what is too long.",
        reply: "Reviewed three of five screens.",
    },
    S {
        title: "Fix the flaky checkout test",
        project: "web-shop",
        branch: "bugfix/flaky-checkout",
        agent: "claude",
        age: 5 * 60,
        ai: "paused",
        state: "Cause found: the coupon test uses the real clock; no fix written yet",
        next: "Claude: freeze the clock in the checkout fixture",
        live: "",
        origin: "terminal",
        prompt: "checkout.spec fails one run in ten on CI. Find out why.",
        reply: "It fails when the day rolls over during the test.",
    },
    S {
        title: "Upgrade the Postgres client to 17",
        project: "payments-api",
        branch: "chore/pg17-client",
        agent: "claude",
        age: 30 * 60,
        ai: "paused",
        state: "Client bumped; the migration tests still use the old driver",
        next: "Claude: update the migration test driver",
        live: "",
        origin: "terminal",
        prompt: "Upgrade the Postgres client library to version 17.",
        reply: "Bumped the client; the migration tests need the new driver.",
    },
    S {
        title: "Draft the Q4 incident review",
        project: "ops-notes",
        branch: "main",
        agent: "claude",
        age: 2 * 24 * 60,
        ai: "done",
        state: "Review written and shared with the on-call team",
        next: "",
        live: "",
        origin: "terminal",
        prompt: "Draft the incident review for the October outage.",
        reply: "Done. The review is in docs/incident-q4.md.",
    },
    S {
        title: "Add a CSV export to the reports page",
        project: "web-shop",
        branch: "feature/csv-export",
        agent: "claude",
        age: 3 * 24 * 60,
        ai: "done",
        state: "Export button, endpoint and tests are merged",
        next: "",
        live: "",
        origin: "desktop",
        prompt: "Add a CSV export to the reports page.",
        reply: "Merged. The export streams rows, so large reports stay fast.",
    },
    S {
        title: "Night run: tidy the lint warnings",
        project: "web-shop",
        branch: "night/tidy-lint",
        agent: "claude",
        age: 9 * 60,
        ai: "done",
        state: "Removed 41 lint warnings in 6 files",
        next: "",
        live: "",
        origin: "night",
        prompt: "Fix the lint warnings without changing behaviour.",
        reply: "Removed 41 warnings; the tests still pass.",
    },
    S {
        title: "Summarise the audit log export",
        project: "ops-notes",
        branch: "main",
        agent: "codex",
        age: 4 * 24 * 60,
        ai: "done",
        state: "Summary of 12k audit lines written",
        next: "",
        live: "",
        origin: "headless",
        prompt: "Summarise the audit log export by actor.",
        reply: "Wrote the summary: three actors account for 80 percent of events.",
    },
];

/// OpenCode's session, kept apart from `SESSIONS` (its ids start at 100). It waits on a permission
/// prompt, which only OpenCode's plugin can tell eye about.
const OPENCODE_SESSIONS: &[S] = &[S {
    title: "Draft the release notes",
    project: "ops-notes",
    branch: "main",
    agent: "opencode",
    age: 14,
    ai: "",
    state: "Notes drafted from the merged pull requests; the changelog script waits",
    next: "You: allow the changelog script",
    live: "waiting",
    origin: "",
    prompt: "Draft the release notes from the pull requests merged this week.",
    reply: "Drafted them; I need to run the changelog script to fill in the dates.",
}];

fn sid(n: usize) -> String {
    format!("e2e00000-0000-4000-8000-{:012}", n + 1)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first() else {
        bail!("usage: fixture_home <dir> [--proc <pid>]");
    };
    let dir = Path::new(dir).canonicalize().or_else(|_| {
        std::fs::create_dir_all(dir)?;
        Path::new(dir).canonicalize()
    })?;
    let paths = Paths {
        home: dir.join("eye"),
    };
    paths.ensure_home()?;
    let db = Db::open(&paths.db_path())?;
    if args.get(1).map(String::as_str) == Some("--proc") {
        let pid: u32 = args.get(2).context("--proc needs a pid")?.parse()?;
        return add_process(&db, pid);
    }
    let home = dir.join("home");
    // The only folder a test really uses: an idea made on the board points at it.
    std::fs::create_dir_all(home.join("code/notes-app"))?;
    // Folders the cards name are plain text under a fixed home, so no run's temporary path shows
    // in a card, in the usage panel or in a screenshot.
    let demo = Path::new("/Users/demo");
    std::fs::write(
        paths.config_path(),
        "[agents]\nenabled = [\"claude\", \"codex\"]\n\n[board]\ndone_days = 0\nresume = \"copy\"\n\n[summary]\nenabled = false\n\n[night]\nenabled = false\n\n[remind]\nnotify = false\n",
    )?;
    let claude = dir.join("claude");
    for (n, s) in SESSIONS.iter().enumerate() {
        insert_session(&db, demo, &claude, n, s)?;
    }
    // OpenCode's session and its entry in `[agents] enabled`, added after the others
    let conf = std::fs::read_to_string(paths.config_path())?;
    let conf = conf.replacen("\"]\n", "\", \"opencode\"]\n", 1);
    std::fs::write(paths.config_path(), conf)?;
    for (n, s) in OPENCODE_SESSIONS.iter().enumerate() {
        insert_session(&db, demo, &claude, 100 + n, s)?;
    }
    ideas(&db, demo)?;
    reminders(&db)?;
    outputs(&db)?;
    usage(&db)?;
    loose(&db)?;
    night(&db, demo)?;
    Ok(())
}

fn insert_session(db: &Db, home: &Path, claude: &Path, n: usize, s: &S) -> Result<()> {
    let id = sid(n);
    let cwd = home.join("code").join(s.project);
    let last = NOW_MS - s.age * MIN;
    let live = (!s.live.is_empty()).then_some(s.live);
    let wait = (s.live == "waiting").then_some("permission prompt");
    // Only the stale-tokens session has a transcript: the conversation and the export read it.
    let transcript = (n == 3).then(|| write_transcript(claude, &cwd, &id, s, last));
    db.conn.execute(
        "INSERT INTO session(id, agent, project_dir, cwd, repo_root, title, title_source, first_prompt, started_at,
           last_activity_at, kind, transcript_path, transcript_state, git_branch, origin, origin_raw, prompt_count,
           reply_count, last_user_text, last_assistant_text, live_status, live_updated_at, waiting_since, waiting_note,
           waiting_prompt, live_wait, remote_url, ai_status, state_line, next_step, summary_at, tok_in, tok_out, tok_cw, tok_cr)
         VALUES(?1,?2,?3,?4,?4,?5,'ai',?6,?7,?8,'interactive',?9,?10,?11,?12,'',?13,?13,?6,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?8,?25,?26,?27,?28)",
        params![
            id,
            s.agent,
            cwd.display().to_string(),
            cwd.display().to_string(),
            s.title,
            s.prompt,
            last - 2 * HOUR,
            last,
            transcript.as_deref(),
            transcript.as_ref().map(|_| "present").unwrap_or("gone"),
            s.branch,
            (!s.origin.is_empty() && s.origin != "terminal").then_some(s.origin),
            6,
            s.reply,
            live,
            live.map(|_| last),
            wait.map(|_| last),
            wait.map(|_| "allow Bash: npm test"),
            wait.map(|_| 1),
            wait,
            (n == 6).then_some("https://claude.ai/code/session_01E2E0000000000000000000"),
            (!s.ai.is_empty()).then_some(s.ai),
            s.state,
            s.next,
            41_000 + n as i64 * 3_100,
            9_000 + n as i64 * 700,
            180_000 + n as i64 * 21_000,
            2_400_000 + n as i64 * 310_000,
        ],
    )?;
    agent_eye::db::fts_write(&db.conn, &id, s.prompt, s.reply, true, 8_000, 8_000)?;
    Ok(())
}

/// Two turns, one with a secret that must be masked in the export.
fn write_transcript(claude: &Path, cwd: &Path, id: &str, s: &S, last: i64) -> String {
    let dir = claude.join("projects").join("-fixture-auth-service");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.jsonl"));
    let ts = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .unwrap()
            .to_string()
            .replace("Z", ".000Z")
    };
    let line = |kind: &str, at: i64, msg: serde_json::Value| {
        serde_json::json!({"type": kind, "timestamp": ts(at), "cwd": cwd.display().to_string(), "sessionId": id, "message": msg})
            .to_string()
    };
    let lines = [
        line(
            "user",
            last - 90 * MIN,
            serde_json::json!({"content": s.prompt}),
        ),
        line(
            "assistant",
            last - 89 * MIN,
            serde_json::json!({"content": [{"type": "text", "text": "Found it: tokens keep their expiry after a password change. Config used token ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA for the test."}]}),
        ),
        line(
            "user",
            last - 30 * MIN,
            serde_json::json!({"content": "Good, expire them softly and keep old sessions alive for a day."}),
        ),
        line(
            "assistant",
            last - 21 * MIN,
            serde_json::json!({"content": [{"type": "text", "text": s.reply}]}),
        ),
    ];
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    path.display().to_string()
}

fn ideas(db: &Db, home: &Path) -> Result<()> {
    let rows: [(&str, &str, &str, Option<&str>, i64); 3] = [
        (
            "Spike: replace polling with webhooks",
            "Check how much the nightly poll costs first.",
            "Sketch a webhook receiver for the order updates and list the open questions.",
            Some("code/payments-api"),
            6 * HOUR,
        ),
        (
            "Write a changelog generator",
            "Group commits by type; look at how the other repos do it.",
            "",
            None,
            2 * DAY,
        ),
        (
            "Evaluate pgvector for the search box",
            "",
            "Compare pgvector with the current full-text search on 10k rows.",
            Some("code/search-lab"),
            5 * DAY,
        ),
    ];
    for (i, (title, notes, prompt, folder, age)) in rows.iter().enumerate() {
        db.conn.execute(
            "INSERT INTO idea(title, notes, prompt, folder, created_at, sort, agent) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                title,
                (!notes.is_empty()).then_some(notes),
                (!prompt.is_empty()).then_some(prompt),
                folder.map(|f| home.join(f).display().to_string()),
                NOW_MS - age,
                i as i64 + 1,
                (i == 2).then_some("codex"),
            ],
        )?;
    }
    Ok(())
}

/// One due (the session has not moved since), one later on a session, one later on an idea.
fn reminders(db: &Db) -> Result<()> {
    let set = |table: &str, key: &str, at: i64, note: &str| -> Result<()> {
        db.conn.execute(
            &format!("UPDATE {table} SET remind_at=?1, remind_note=?2 WHERE {key}"),
            params![at, note],
        )?;
        Ok(())
    };
    set(
        "session",
        &format!("id='{}'", sid(4)),
        NOW_MS - HOUR,
        "Ask Ana whether the access policy is ready",
    )?;
    set(
        "session",
        &format!("id='{}'", sid(8)),
        NOW_MS + 3 * DAY,
        "Check that the new driver is released",
    )?;
    set("idea", "id=2", NOW_MS + 7 * DAY, "")?;
    Ok(())
}

fn outputs(db: &Db) -> Result<()> {
    for (n, kind, value, url) in [
        (
            5,
            "pr",
            "https://github.com/acme/payments-api/pull/412",
            "https://github.com/acme/payments-api/pull/412",
        ),
        (
            10,
            "pr",
            "https://github.com/acme/web-shop/pull/388",
            "https://github.com/acme/web-shop/pull/388",
        ),
    ] {
        db.conn.execute(
            "INSERT INTO output(session_id, kind, value, url, first_seen_at) VALUES(?1,?2,?3,?4,?5)",
            params![sid(n), kind, value, url, NOW_MS - DAY],
        )?;
        // the search index holds a session's outputs, so it is written again
        agent_eye::db::fts_write(&db.conn, &sid(n), "", "", false, 8_000, 8_000)?;
    }
    Ok(())
}

/// Ten days of use, five sessions, four models, two agents.
fn usage(db: &Db) -> Result<()> {
    let models = [
        (0usize, "claude-opus-5-5"),
        (2, "claude-sonnet-5-5"),
        (5, "claude-sonnet-5-5"),
        (7, "claude-haiku-5-5"),
        (1, "gpt-6-luna"),
        (12, "gpt-6-luna"),
    ];
    for back in 0..10i64 {
        let day = jiff::Timestamp::from_millisecond(NOW_MS - back * DAY)?
            .strftime("%Y-%m-%d")
            .to_string();
        for (i, (n, model)) in models.iter().enumerate() {
            if (back + i as i64) % 3 == 2 {
                continue;
            }
            let k = 1 + ((back * 7 + i as i64 * 5) % 6);
            db.conn.execute(
                "INSERT INTO usage_day(session_id, day, model, tok_in, tok_out, tok_cw, tok_cr) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![sid(*n), day, model, k * 40_000, k * 9_000, k * 110_000, k * 1_700_000],
            )?;
        }
    }
    for (n, _) in &models {
        db.conn.execute(
            "INSERT OR IGNORE INTO usage_state(session_id, unreadable) VALUES(?1,0)",
            [sid(*n)],
        )?;
    }
    let w = |name: &str, used: f64, minutes: i64, resets: i64| LimitWindow {
        name: name.into(),
        used_percent: used,
        window_minutes: Some(minutes),
        resets_at: Some(NOW_MS + resets),
    };
    limits::record(
        &db.conn,
        "claude",
        &PlanLimits {
            seen_at: NOW_MS - 20 * MIN,
            plan: Some("Max".into()),
            windows: vec![
                w("five_hour", 62.0, 300, 2 * HOUR),
                w("seven_day", 31.0, 10_080, 3 * DAY),
            ],
        },
    )?;
    Ok(())
}

/// What sessions left behind: git items on the stale-tokens session, a server on another. A
/// process item that can be stopped is added by `--proc`.
fn loose(db: &Db) -> Result<()> {
    let items = [
        (
            3,
            Kind::Unpushed,
            "unpushed:/work/auth-service:abc1234",
            "2 commits in auth-service are on no remote",
            "2 commits not pushed\nrepo /work/auth-service",
        ),
        (
            3,
            Kind::Uncommitted,
            "uncommitted:/work/auth-service",
            "3 modified files in auth-service are not committed",
            "3 modified files\nrepo /work/auth-service",
        ),
        (
            3,
            Kind::Process,
            "proc:999999:1795000000",
            "node server on port 4321 is still running",
            "node (pid 999999, port 4321)",
        ),
        (
            0,
            Kind::Unpushed,
            "unpushed:/work/web-shop:def5678",
            "1 commit in web-shop is on no remote",
            "1 commit not pushed\nrepo /work/web-shop",
        ),
    ];
    for (n, kind, key, line, detail) in items {
        db.conn.execute(
            "INSERT INTO loose(session_id, key, kind, line, detail, since_ms, found_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![sid(n), key, kind.as_str(), line, detail, NOW_MS - 3 * HOUR, NOW_MS - 5 * MIN],
        )?;
    }
    Ok(())
}

/// A process the caller started, stored the way a scan would.
fn add_process(db: &Db, pid: u32) -> Result<()> {
    let info = System::new()
        .proc_info(pid)
        .with_context(|| format!("pid {pid} is not running"))?;
    let it = LooseEnd {
        key: format!("proc:{pid}:{}", info.start),
        session_id: sid(3),
        kind: Kind::Process,
        line: format!("sleep (pid {pid}) is still running"),
        detail: format!("sleep (pid {pid})"),
        since_ms: NOW_MS - HOUR,
    };
    db.conn.execute(
        "INSERT OR REPLACE INTO loose(session_id, key, kind, line, detail, since_ms, found_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![it.session_id, it.key, it.kind.as_str(), it.line, it.detail, it.since_ms, NOW_MS],
    )?;
    println!("{}", it.key);
    Ok(())
}

/// A night run in every status. Two are planned (an idea and a session); night is switched off in
/// the config, so nothing starts.
fn night(db: &Db, home: &Path) -> Result<()> {
    let folder = home.join("code/payments-api").display().to_string();
    // (kind, idea, session, status, why, run session, started, ended, report, diff)
    type Row<'a> = (
        &'a str,
        Option<i64>,
        Option<usize>,
        &'a str,
        Option<&'a str>,
        Option<usize>,
        Option<i64>,
        Option<i64>,
        Option<&'a str>,
        Option<&'a str>,
    );
    let rows: [Row; 6] = [
        (
            "idea",
            Some(1),
            None,
            "planned",
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        (
            "session",
            None,
            Some(7),
            "planned",
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        (
            "session",
            None,
            Some(1),
            "running",
            None,
            None,
            Some(NOW_MS - 20 * MIN),
            None,
            None,
            None,
        ),
        (
            "idea",
            Some(2),
            None,
            "done",
            None,
            Some(11),
            Some(NOW_MS - 11 * HOUR),
            Some(NOW_MS - 9 * HOUR - 30 * MIN),
            Some("Removed 41 lint warnings; tests pass."),
            Some(" 6 files changed, 22 insertions(+), 63 deletions(-)"),
        ),
        (
            "idea",
            Some(3),
            None,
            "failed",
            Some("the agent exited with an error"),
            None,
            Some(NOW_MS - 12 * HOUR),
            Some(NOW_MS - 11 * HOUR - 50 * MIN),
            Some("Error: could not resolve the dependency graph."),
            None,
        ),
        (
            "session",
            None,
            Some(9),
            "skipped",
            Some("a session you work in was busy"),
            None,
            None,
            Some(NOW_MS - 10 * HOUR),
            None,
            None,
        ),
    ];
    for (i, (kind, idea, session, status, why, run, started, ended, report, diff)) in
        rows.iter().enumerate()
    {
        db.conn.execute(
            "INSERT INTO night_run(kind, idea_id, session_id, agent, folder, prompt, night, status, why, run_session_id,
               started_at, ended_at, report, diff_stat, created_at, isolation, worktree, branch)
             VALUES(?1,?2,?3,'claude',?4,?5,'2026-11-20',?6,?7,?8,?9,?10,?11,?12,?13,'worktree',?14,?15)",
            params![
                kind,
                idea,
                session.map(sid),
                folder,
                "Continue with the failing tests, then stop.",
                status,
                why,
                run.map(sid),
                started,
                ended,
                report,
                diff,
                NOW_MS - 14 * HOUR + i as i64,
                (*status == "done").then_some("/tmp/eye-night/run-4"),
                (*status == "done").then_some("night/tidy-lint"),
            ],
        )?;
    }
    Ok(())
}
