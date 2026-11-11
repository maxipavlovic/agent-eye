//! The terminal picker's engine: the in-memory index, ranking, launching, and the shell widgets.
//! The screen lives in `tui.rs`.

use crate::agents::Agents;
use crate::columns;
use crate::config::Config;
use crate::db::Db;
use crate::folder::{self, FolderState};
use crate::hide;
use crate::ideas;
use crate::model::{Column, Idea, Output, Session, Stream};
use crate::paths::Paths;
use crate::remind::{self, Remind};
use crate::search::{rank_key, RankKey};
use crate::util::{head_chars, printable, shell_quote, short_id};
use anyhow::{Context, Result};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as MatcherConfig, Matcher, Utf32Str};
use std::collections::HashMap;
use std::path::PathBuf;

const FTS_LIMIT: i64 = 400;

/// A picker row is a session or an idea.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Session,
    Idea,
}

/// One row of the picker.
#[derive(Debug, Clone)]
pub struct Row {
    pub kind: Kind,
    /// Session id, or `idea:<n>`.
    pub id: String,
    pub title: String,
    pub title_lc: String,
    pub notes_lc: String,
    /// Last folder segment.
    pub project: String,
    pub folder: Option<String>,
    /// What the folder of an idea is, found once when the rows are loaded (the disk is not asked
    /// again per frame); None for a session or an idea without a folder.
    pub folder_state: Option<FolderState>,
    pub column: Column,
    pub last: i64,
    pub hidden: bool,
    /// Board position of an idea.
    pub sort: i64,
    pub session: Option<Session>,
    pub idea: Option<Idea>,
    /// A reminder to check on this, set for later or due.
    pub remind: Option<Remind>,
}

/// needs_you > running > waiting > idea > paused > done > archive
pub fn status_rank(c: Column) -> u8 {
    match c {
        Column::NeedsYou => 0,
        Column::Running => 1,
        Column::Waiting => 2,
        Column::Ideas => 3,
        Column::Paused => 4,
        Column::Done => 5,
        Column::Archive => 6,
    }
}

/// Every row, in memory, ready to rank.
pub struct Index {
    pub rows: Vec<Row>,
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn project_of(cwd: &str) -> String {
    let t = cwd.trim_end_matches('/');
    if std::env::var_os("HOME").is_some_and(|h| h.to_string_lossy().trim_end_matches('/') == t) {
        return "~".to_string();
    }
    match t.rsplit('/').next() {
        Some(x) if !x.is_empty() => x.to_string(),
        _ => "/".to_string(),
    }
}

/// The title of a session on one line.
pub fn session_title(s: &Session) -> String {
    let t = s
        .title
        .clone()
        .or_else(|| s.first_prompt.as_deref().map(|p| head_chars(p, 80)))
        .unwrap_or_else(|| "(untitled)".into());
    let t = one_line(&t);
    if t.is_empty() {
        "(untitled)".into()
    } else {
        t
    }
}

impl Index {
    /// Load every session and open idea into rows.
    pub fn load(db: &Db, cfg: &Config, now: i64) -> Result<Index> {
        let outs = db.session_ids_with_outputs()?;
        let kids = db.session_ids_with_children()?;
        let enabled = cfg.enabled_agent_ids();
        let mut rows = vec![];
        for s in db.all_sessions()? {
            let has_out = outs.contains(&s.id);
            let column = columns::compute(&columns::Input::from_session(&s, has_out, now));
            let hidden = hide::agent_off(&enabled, &s)
                || hide::is_hidden(&cfg.hide, &s, has_out, kids.contains(&s.id));
            let title = session_title(&s);
            rows.push(Row {
                kind: Kind::Session,
                id: s.id.clone(),
                title_lc: title.to_lowercase(),
                title,
                notes_lc: String::new(),
                project: s.cwd.as_deref().map(project_of).unwrap_or_default(),
                folder: s.cwd.clone(),
                folder_state: None,
                column,
                last: s.last_activity_at.unwrap_or(0),
                hidden,
                sort: 0,
                remind: remind::of_session(&s, now),
                session: Some(s),
                idea: None,
            });
        }
        let open_ideas = ideas::list_open(db)?;
        for (n, i) in open_ideas
            .into_iter()
            .filter(|i| !hide::idea_off(&enabled, i))
            .enumerate()
        {
            let title = one_line(&i.title);
            let remind = remind::of_idea(&i, now);
            // a due idea is in Needs you, as it is on the board
            let column = if remind.as_ref().is_some_and(|r| r.due) {
                Column::NeedsYou
            } else {
                Column::Ideas
            };
            rows.push(Row {
                kind: Kind::Idea,
                id: format!("idea:{}", i.id),
                title_lc: title.to_lowercase(),
                title,
                notes_lc: i
                    .notes
                    .as_deref()
                    .map(one_line)
                    .unwrap_or_default()
                    .to_lowercase(),
                project: i.folder.as_deref().map(project_of).unwrap_or_default(),
                folder_state: i
                    .folder
                    .as_deref()
                    .map(|f| folder::classify(cfg.folder_root().as_deref(), f)),
                folder: i.folder.clone(),
                column,
                last: i.created_at.unwrap_or(0),
                hidden: false,
                sort: n as i64,
                session: None,
                idea: Some(i),
                remind,
            });
        }
        Ok(Index { rows })
    }

    /// Rows for a query, best first. `fts` is the FTS hit ids in rank order.
    pub fn rank(&self, query: &str, all: bool, fts: &[String], now: i64) -> Vec<usize> {
        let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
        let id_q = crate::search::id_query(&words);
        if words.is_empty() {
            return self.board_order(all);
        }
        let fts_pos: HashMap<&str, usize> = fts
            .iter()
            .enumerate()
            .map(|(i, s)| (s.as_str(), i))
            .collect();
        // Fuzzy only for words long enough not to match everything.
        let fuzzy = words.iter().all(|w| w.chars().count() >= 3).then(|| {
            Pattern::new(
                query,
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
            )
        });
        let mut matcher = Matcher::new(MatcherConfig::DEFAULT);
        let mut buf = vec![];
        let q_len = words.join(" ").chars().count() as i64;
        // Archive last, then the match group (title, body, fuzzy); inside a group freshness,
        // status, then relevance (extra title length, FTS position or fuzzy score), newest first.
        let mut keyed: Vec<(RankKey, usize)> = vec![];
        for (i, r) in self.rows.iter().enumerate() {
            if r.hidden && !all {
                continue;
            }
            if id_q
                .as_deref()
                .is_some_and(|p| r.kind == Kind::Session && r.id.starts_with(p))
            {
                keyed.push((crate::search::ID_HIT, i));
                continue;
            }
            let title_all = words.iter().all(|w| r.title_lc.contains(w.as_str()));
            let fpos = fts_pos.get(r.id.as_str()).copied();
            let notes_all = !r.notes_lc.is_empty()
                && words
                    .iter()
                    .all(|w| r.title_lc.contains(w.as_str()) || r.notes_lc.contains(w.as_str()));
            let (group, rel) = if title_all {
                (0, (r.title_lc.chars().count() as i64 - q_len).max(0))
            } else if let Some(p) = fpos {
                (1, p as i64)
            } else if notes_all {
                (1, i64::MAX)
            } else if let Some(sc) = fuzzy
                .as_ref()
                .and_then(|p| p.score(Utf32Str::new(&r.title, &mut buf), &mut matcher))
            {
                (2, -(sc as i64))
            } else {
                continue;
            };
            let key = rank_key(r.column, group, rel, r.last, now);
            keyed.push((key, i));
        }
        keyed.sort_by_key(|(k, _)| *k);
        keyed.into_iter().map(|(_, i)| i).collect()
    }

    /// Needs you, Running, Waiting, Ideas, Paused, Done; archive only with `all`.
    pub fn board_order(&self, all: bool) -> Vec<usize> {
        let rank = status_rank;
        let mut v: Vec<usize> = (0..self.rows.len())
            .filter(|&i| {
                let r = &self.rows[i];
                all || (!r.hidden && r.column != Column::Archive)
            })
            .collect();
        v.sort_by_key(|&i| {
            let r = &self.rows[i];
            (
                rank(r.column),
                !r.remind.as_ref().is_some_and(|r| r.due),
                if r.kind == Kind::Idea {
                    r.sort
                } else {
                    -r.last
                },
            )
        });
        v
    }
}

/// Ids of sessions matching every word as an FTS prefix, best first.
pub fn fts_ids(db: &Db, query: &str) -> Vec<String> {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.chars().filter(|c| c.is_alphanumeric()).count() >= 2)
        .map(String::from)
        .collect();
    let Some(q) = crate::search::build_query(&words, " ") else {
        return vec![];
    };
    let run = || -> rusqlite::Result<Vec<String>> {
        let mut st = db.conn.prepare_cached(
            "SELECT s.id FROM session_fts JOIN session s ON s.rowid = session_fts.rowid
             WHERE session_fts MATCH ?1
             ORDER BY bm25(session_fts, 0.0, 10.0, 3.0, 1.0, 5.0) LIMIT ?2",
        )?;
        let rows = st.query_map(rusqlite::params![q, FTS_LIMIT], |r| r.get::<_, String>(0))?;
        rows.collect()
    };
    run().unwrap_or_default()
}

/// Rows for a query, best first (empty query: board order).
pub fn search(db: &Db, idx: &Index, query: &str, all: bool, now: i64) -> Vec<usize> {
    let fts = if query.trim().is_empty() {
        vec![]
    } else {
        fts_ids(db, query)
    };
    idx.rank(query, all, &fts, now)
}

/// What the preview needs beyond the row.
#[derive(Debug, Clone, Default)]
pub struct Detail {
    pub streams: Vec<Stream>,
    pub outputs: Vec<Output>,
}

/// The streams and outputs the preview shows for a row.
pub fn detail(db: &Db, row: &Row) -> Detail {
    if row.kind != Kind::Session {
        return Detail::default();
    }
    Detail {
        streams: db.streams_for(&row.id).unwrap_or_default(),
        outputs: db.outputs_for(&row.id).unwrap_or_default(),
    }
}

/// True when the row can be resumed or started.
pub fn actionable(row: &Row) -> bool {
    match row.kind {
        Kind::Session => row
            .session
            .as_ref()
            .is_some_and(|s| s.kind.as_deref() != Some("history_only") && s.cwd.is_some()),
        // Ideas come from the open list, which drops them once their session shows up.
        Kind::Idea => row.idea.is_some(),
    }
}

/// Everything needed to land in an agent.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Shell line for `--print`.
    pub command: String,
    pub cwd: PathBuf,
    /// The executable to run and its arguments.
    pub program: String,
    pub args: Vec<String>,
    /// The idea whose start was recorded; undone when the launch fails.
    pub idea: Option<i64>,
}

/// How a row is launched.
#[derive(Debug, Clone, Copy, Default)]
pub struct LaunchOpts {
    /// Resume a session as a new session that branches from it.
    pub fork: bool,
    /// Make the folder of an idea first when it does not exist.
    pub create_folder: bool,
}

/// Restore the transcript from the archive when it is gone, and record an idea start.
/// An idea whose folder does not exist is started only with `create_folder`.
pub fn prepare(
    db: &Db,
    paths: &Paths,
    agents: &Agents,
    cfg: &Config,
    row: &Row,
    how: LaunchOpts,
) -> Result<Launch> {
    let LaunchOpts {
        fork,
        create_folder,
    } = how;
    anyhow::ensure!(
        !fork || row.kind == Kind::Session,
        "only a session can be forked"
    );
    match row.kind {
        Kind::Session => {
            let s = row.session.as_ref().context("no session")?;
            anyhow::ensure!(
                s.kind.as_deref() != Some("history_only"),
                "this session has no transcript any more; it cannot be resumed"
            );
            let (agent, cwd) = agents.runnable(s).map_err(anyhow::Error::msg)?;
            crate::archive::restore_if_missing(db, paths, agents, s)?;
            anyhow::ensure!(
                std::path::Path::new(cwd).is_dir(),
                "the folder {cwd} no longer exists"
            );
            let inv = if fork {
                agent
                    .fork(&s.id)
                    .with_context(|| format!("{} cannot fork a session", agent.name()))?
            } else {
                agent.resume(&s.id)
            };
            Ok(Launch {
                command: inv.shell_line(cwd),
                cwd: PathBuf::from(cwd),
                program: agent.executable(),
                args: inv.args,
                idea: None,
            })
        }
        Kind::Idea => {
            let idea = row.idea.as_ref().context("no idea")?;
            let cur = ideas::get(db, idea.id)?.context("the idea is gone")?;
            if let Some(s) = ideas::started_session(db, &cur)? {
                anyhow::bail!("this idea was already started (session {s})");
            }
            let plan = ideas::plan_start(agents, &cur, false)?;
            if ideas::check_folder(cfg, &plan.dir, create_folder)? {
                crate::folder::create(cfg.folder_root().as_deref(), &plan.dir)?;
            }
            ideas::record_start(db, cur.id, plan.session_id.as_deref())?;
            Ok(Launch {
                command: plan.command(),
                cwd: PathBuf::from(&plan.dir),
                program: plan.agent.executable(),
                args: plan.invocation.args,
                idea: Some(cur.id),
            })
        }
    }
}

/// Replace this process with the agent. Returns only on failure.
pub fn exec(db: &Db, l: &Launch) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(&l.program)
        .args(&l.args)
        .current_dir(&l.cwd)
        .exec();
    if let Some(i) = l.idea {
        let _ = ideas::undo_start(db, i);
    }
    anyhow::Error::new(err).context(format!("starting {}", l.program))
}

/// "5m", "3h", "2d", "4mo".
pub fn short_age(now: i64, ms: i64) -> String {
    let s = (now - ms).max(0) / 1000;
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        86400..=5_183_999 => format!("{}d", s / 86400),
        _ => format!("{}mo", s / 2_592_000),
    }
}

/// The status mark of a column.
pub fn mark(c: Column) -> &'static str {
    match c {
        Column::NeedsYou => "●",
        Column::Running => "▶",
        Column::Waiting => "◷",
        Column::Ideas => "✦",
        Column::Paused => "‖",
        Column::Done => "✓",
        Column::Archive => "·",
    }
}

/// The column's name for people.
pub fn column_label(c: Column) -> &'static str {
    match c {
        Column::NeedsYou => "Needs you",
        Column::Running => "Running",
        Column::Waiting => "Waiting on others",
        Column::Ideas => "Idea",
        Column::Paused => "Paused",
        Column::Done => "Done",
        Column::Archive => "Archive",
    }
}

/// The plain-text list used without a terminal and by `--filter`.
pub fn plain_line(r: &Row, now: i64) -> String {
    format!(
        "{}  {} {:<9}  {:>4}  {}  [{}]",
        short_id(&r.id),
        mark(r.column),
        r.column.as_str(),
        short_age(now, r.last),
        printable(&r.title),
        printable(
            &r.folder
                .as_deref()
                .map(crate::util::home_rel)
                .unwrap_or_default()
        )
    )
}

// ---- shell integration ----

/// `ctrl-g`, `^G`, `\C-g` and `c-g` all name Ctrl-G.
pub fn parse_key(k: &str) -> Result<char> {
    let t = k.trim().to_lowercase();
    let letter = t
        .strip_prefix("ctrl-")
        .or_else(|| t.strip_prefix("c-"))
        .or_else(|| t.strip_prefix("\\c-"))
        .or_else(|| t.strip_prefix('^'))
        .unwrap_or("");
    let mut cs = letter.chars();
    match (cs.next(), cs.next()) {
        (Some(c), None) if c.is_ascii_lowercase() => Ok(c),
        _ => anyhow::bail!("unknown key '{k}': use something like ctrl-g"),
    }
}

/// The name this program was invoked under (`eye`, or an alias); widgets and hints use it.
pub fn bin_name() -> String {
    std::env::args_os()
        .next()
        .and_then(|a| {
            std::path::Path::new(&a)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
        })
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "agent-eye".into())
}

/// The shell widget for `eye init zsh|bash`.
pub fn init_script(shell: &str, key: &str, bin: &str) -> Result<String> {
    let k = parse_key(key)?;
    match shell {
        "zsh" => Ok(format!(
            r#"# {bin}: Ctrl-{up} opens the session picker; the chosen command lands on the command line
_eye_pick() {{
  local sel
  zle -I
  sel="$(command {bin} pick --print -- "$BUFFER" </dev/tty)" || {{ zle reset-prompt; return 0; }}
  if [[ -n $sel ]]; then
    LBUFFER="$sel"
    RBUFFER=""
  fi
  zle reset-prompt
}}
zle -N _eye_pick
bindkey '^{up}' _eye_pick
"#,
            up = k.to_ascii_uppercase(),
            bin = shell_quote(bin).trim_matches('\'').to_string()
        )),
        "bash" => Ok(format!(
            r#"# {bin}: Ctrl-{up} opens the session picker; the chosen command lands on the command line
_eye_pick() {{
  local sel
  sel="$(command {bin} pick --print -- "$READLINE_LINE" </dev/tty)" || return 0
  if [[ -n $sel ]]; then
    READLINE_LINE="$sel"
    READLINE_POINT=${{#READLINE_LINE}}
  fi
}}
bind -x '"\C-{k}": _eye_pick'
"#,
            up = k.to_ascii_uppercase(),
            bin = shell_quote(bin).trim_matches('\'').to_string()
        )),
        other => anyhow::bail!("unsupported shell '{other}' (zsh or bash)"),
    }
}

/// The line to put in the rc file.
pub fn rc_line(shell: &str, bin: &str) -> String {
    format!("eval \"$({bin} init {shell})\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idea_row_knows_its_folder_from_the_load_and_a_session_row_asks_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = Config::with_claude_dir(&tmp.path().join("c"));
        cfg.folder_root = Some(tmp.path().to_path_buf());
        let db = Db::open_memory().unwrap();
        let folder = tmp.path().join("new").to_string_lossy().into_owned();
        for f in [Some(folder.as_str()), None] {
            ideas::add(
                &db,
                &ideas::NewIdea {
                    title: "Try it",
                    folder: f,
                    ..ideas::NewIdea::default()
                },
            )
            .unwrap();
        }
        let idx = Index::load(&db, &cfg, 1_000).unwrap();
        let states: Vec<_> = idx.rows.iter().map(|r| r.folder_state.clone()).collect();
        assert!(states.contains(&Some(FolderState::Missing { creatable: true })));
        assert!(states.contains(&None), "an idea without a folder");
        std::fs::create_dir(tmp.path().join("new")).unwrap();
        let again = Index::load(&db, &cfg, 1_000).unwrap();
        assert!(again
            .rows
            .iter()
            .any(|r| r.folder_state == Some(FolderState::Exists)));
    }

    fn row(id: &str, title: &str, col: Column, last: i64) -> Row {
        Row {
            kind: Kind::Session,
            id: id.into(),
            title: title.into(),
            title_lc: title.to_lowercase(),
            notes_lc: String::new(),
            project: "p".into(),
            folder: Some("/tmp/p".into()),
            folder_state: None,
            column: col,
            last,
            hidden: false,
            sort: 0,
            session: None,
            idea: None,
            remind: None,
        }
    }

    #[test]
    fn a_session_folder_with_control_characters_is_not_launched() {
        let db = Db::open_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: tmp.path().to_path_buf(),
        };
        let agents = Agents::from_vec(vec![std::sync::Arc::new(
            crate::agents::claude::Claude::new("/nonexistent".into()),
        )]);
        let mut r = row("abcd-1234", "t", Column::Paused, 1);
        r.session = Some(Session {
            id: "abcd-1234".into(),
            cwd: Some(tmp.path().join("a\nb").to_string_lossy().into_owned()),
            ..Default::default()
        });
        let err = prepare(
            &db,
            &paths,
            &agents,
            &Config::default(),
            &r,
            LaunchOpts::default(),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("control characters"), "{err}");
    }

    const NOW: i64 = 100 * DAY;
    const DAY: i64 = 24 * 3600 * 1000;

    fn idx() -> Index {
        Index {
            rows: vec![
                row("a", "Catalog migration", Column::Paused, 10),
                row("b", "Billing fixes", Column::NeedsYou, 5),
                row("c", "Weekly report", Column::Running, 20),
                row("d", "Old thing", Column::Done, 30),
                row("e", "Archived chat", Column::Archive, 40),
                row("f", "Catalog sync", Column::Paused, 50),
            ],
        }
    }

    fn ids(i: &Index, v: Vec<usize>) -> Vec<String> {
        v.into_iter().map(|n| i.rows[n].id.clone()).collect()
    }

    #[test]
    fn empty_query_is_board_order() {
        let i = idx();
        assert_eq!(
            ids(&i, i.rank("", false, &[], NOW)),
            ["b", "c", "f", "a", "d"]
        );
        assert_eq!(
            ids(&i, i.rank("  ", true, &[], NOW)),
            ["b", "c", "f", "a", "d", "e"]
        );
    }

    #[test]
    fn title_hits_first_then_fts_then_fuzzy() {
        let i = idx();
        // "d" only matches through FTS (prompt text), "a" by title
        let r = ids(&i, i.rank("catalog", false, &["d".into(), "a".into()], NOW));
        assert_eq!(r, ["f", "a", "d"]); // closer title first
                                        // title AND with out-of-order words
        assert_eq!(ids(&i, i.rank("migration cat", false, &[], NOW)), ["a"]);
        // fuzzy catches a typo-ish subsequence
        assert_eq!(ids(&i, i.rank("blng", false, &[], NOW)), ["b"]);
        assert_eq!(ids(&i, i.rank("bilfix", false, &[], NOW)), ["b"]);
        // nothing matches
        assert!(i.rank("zzzz", false, &[], NOW).is_empty());
    }

    #[test]
    fn closest_title_then_status_then_recency() {
        let i = Index {
            rows: vec![
                row("long", "catalog migration session search", Column::Done, 99),
                row("real", "Catalog Migration", Column::NeedsYou, 1),
                row("same1", "Catalog Migration", Column::Paused, 50),
                row("same2", "Catalog Migration", Column::Paused, 60),
            ],
        };
        // FTS even prefers the long one; the title closeness still wins
        let r = ids(
            &i,
            i.rank(
                "catalog migration",
                false,
                &["long".into(), "real".into()],
                NOW,
            ),
        );
        assert_eq!(r, ["real", "same2", "same1", "long"]);
    }

    #[test]
    fn recent_body_hit_beats_older_one_with_better_fts_position() {
        let i = Index {
            rows: vec![
                row("old", "Alpha", Column::Paused, NOW - 40 * DAY),
                row("new", "Beta", Column::Paused, NOW - DAY),
            ],
        };
        let r = ids(
            &i,
            i.rank("zebra", false, &["old".into(), "new".into()], NOW),
        );
        assert_eq!(r, ["new", "old"]);
    }

    #[test]
    fn fresh_paused_beats_stale_done_with_closer_title() {
        let i = Index {
            rows: vec![
                row("closer", "Catalog", Column::Done, NOW - 60 * DAY),
                row(
                    "fresh",
                    "Catalog migration plan",
                    Column::Paused,
                    NOW - 2 * DAY,
                ),
                row(
                    "month",
                    "Catalog migration plan",
                    Column::Paused,
                    NOW - 20 * DAY,
                ),
                row("body", "Alpha", Column::NeedsYou, NOW),
            ],
        };
        let r = ids(&i, i.rank("catalog", false, &["body".into()], NOW));
        assert_eq!(r, ["fresh", "month", "closer", "body"]);
    }

    #[test]
    fn archive_last_and_hidden_filtered() {
        let mut i = idx();
        i.rows[2].hidden = true;
        assert!(!ids(&i, i.rank("weekly", false, &[], NOW)).contains(&"c".to_string()));
        assert_eq!(ids(&i, i.rank("weekly", true, &[], NOW)), ["c"]);
        // an archive session with a title hit still ranks after the others
        i.rows[4].title = "Catalog archived".into();
        i.rows[4].title_lc = "catalog archived".into();
        let r = ids(&i, i.rank("catalog", false, &[], NOW));
        assert_eq!(r.last().unwrap(), "e");
    }

    #[test]
    fn keys_and_widgets() {
        assert_eq!(parse_key("ctrl-g").unwrap(), 'g');
        assert_eq!(parse_key("^R").unwrap(), 'r');
        assert_eq!(parse_key("\\C-t").unwrap(), 't');
        assert!(parse_key("f5").is_err());
        let z = init_script("zsh", "ctrl-g", "eye").unwrap();
        assert!(z.contains("bindkey '^G' _eye_pick"));
        assert!(z.contains("eye pick --print -- \"$BUFFER\""));
        assert!(z.contains("LBUFFER=\"$sel\""));
        let b = init_script("bash", "ctrl-o", "eye").unwrap();
        assert!(b.contains("bind -x '\"\\C-o\": _eye_pick'"));
        assert!(b.contains("READLINE_LINE=\"$sel\""));
        assert!(init_script("fish", "ctrl-g", "eye").is_err());
        let a = init_script("zsh", "ctrl-g", "agent-eye").unwrap();
        assert!(a.contains("command agent-eye pick --print") && !a.contains("command eye"));
        assert_eq!(rc_line("zsh", "eye"), "eval \"$(eye init zsh)\"");
    }

    #[test]
    fn home_folder_is_tilde() {
        let h = std::env::var("HOME").unwrap();
        assert_eq!(project_of(&h), "~");
        assert_eq!(project_of(&format!("{h}/code/webapp")), "webapp");
    }

    #[test]
    fn ages() {
        assert_eq!(short_age(1_000_000, 1_000_000), "now");
        assert_eq!(short_age(10_000_000, 10_000_000 - 3 * 3_600_000), "3h");
        assert_eq!(short_age(10_000_000_000, 0), "3mo");
    }
}
