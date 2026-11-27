//! `[board] quiet`: sessions a program started, kept quiet. A program that drives an agent (an app
//! on the agent's SDK, a script's headless run) has a user of its own, so the agent's questions are
//! not yours. A quiet session never moves itself to Needs you (`columns`), gets no summary from the
//! daemon (`daemon::quiet_round`) and stays out of the steering clock (`clock::signals::left_out`).
//! It is still indexed, found, listed and resumable. Each part reads the setting when it runs, so a
//! switch on the board counts without a restart.

use crate::columns::{self, Input};
use crate::config::Config;
use crate::db::Db;
use crate::model::Column;
use anyhow::Result;

/// The origins a program starts, the only ones that can be quiet: `app` (an app that drives the
/// agent through its SDK, or the agent as an MCP server) and `headless` (`claude -p`, `codex exec`,
/// a CI action).
pub const ORIGINS: [&str; 2] = ["app", "headless"];

/// Which of `ORIGINS` are quiet. Small and `Copy`, so whatever works out a column can carry it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Quiet([bool; ORIGINS.len()]);

impl Quiet {
    /// Nothing is quiet: the column a session gets without the setting.
    pub const NONE: Quiet = Quiet([false; ORIGINS.len()]);

    /// The origins `[board] quiet` lists.
    pub fn of(cfg: &Config) -> Quiet {
        Quiet(ORIGINS.map(|o| cfg.board.quiet.iter().any(|q| q == o)))
    }

    /// Is a session of this origin quiet?
    pub fn holds(self, origin: Option<&str>) -> bool {
        origin.is_some_and(|o| self.origins().any(|q| q == o))
    }

    /// The quiet origins, in the order of `ORIGINS`.
    pub fn origins(self) -> impl Iterator<Item = &'static str> {
        ORIGINS
            .into_iter()
            .zip(self.0)
            .filter_map(|(o, on)| on.then_some(o))
    }
}

/// Why a list cannot be `[board] quiet`: it names an origin that is not a program's.
pub fn check(list: &[String]) -> Result<(), String> {
    match list.iter().find(|o| !ORIGINS.contains(&o.as_str())) {
        Some(o) => Err(format!("'{o}' cannot be quiet: give app or headless")),
        None => Ok(()),
    }
}

/// One program origin as the settings panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub origin: &'static str,
    /// Its sessions, but for hidden ones and ones archived by hand: what quiet covers. A throw-away
    /// one the rules file in the archive counts, since its summary and its prompts are quiet too.
    pub sessions: usize,
    /// Those in Needs you now, as if nothing were quiet: what turning quiet on would clear.
    pub needs_you: usize,
}

/// The program origins that have sessions on this machine, in the order of `ORIGINS`. An origin
/// set to quiet stays in the list when none of its sessions are left, so it can be turned off.
pub fn found(db: &Db, cfg: &Config, now: i64) -> Result<Vec<Found>> {
    let hidden = crate::hide::hidden_ids(db, cfg)?;
    let outputs = db.session_ids_with_outputs()?;
    let mut out: Vec<Found> = ORIGINS
        .iter()
        .map(|&origin| Found {
            origin,
            sessions: 0,
            needs_you: 0,
        })
        .collect();
    for s in db.all_sessions()? {
        let Some(f) = out
            .iter_mut()
            .find(|f| s.origin.as_deref() == Some(f.origin))
        else {
            continue;
        };
        if s.archived || hidden.contains(&s.id) {
            continue;
        }
        let input = Input::from_session(&s, outputs.contains(&s.id), now, Quiet::NONE);
        f.sessions += 1;
        f.needs_you += usize::from(columns::compute(&input) == Column::NeedsYou);
    }
    let set = Quiet::of(cfg);
    out.retain(|f| f.sessions > 0 || set.holds(Some(f.origin)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    const NOW: i64 = 10_000_000;

    fn quiet(list: &[&str]) -> Config {
        let mut c = Config::default();
        c.board.quiet = list.iter().map(|o| o.to_string()).collect();
        c
    }

    /// A session of `origin` with three prompts, active a minute ago, that the summary put in `ai`.
    fn add(db: &Db, id: &str, origin: Option<&str>, ai: Option<&str>) {
        db.conn
            .execute(
                "INSERT INTO session(id, cwd, kind, prompt_count, last_activity_at, ai_status, origin, transcript_state)
                 VALUES(?1, '/work/p', 'interactive', 3, ?2, ?3, ?4, 'present')",
                params![id, NOW - 60_000, ai, origin],
            )
            .unwrap();
    }

    #[test]
    fn nothing_is_quiet_by_default_and_an_old_config_without_the_key_works() {
        for text in ["[board]\nport = 7500\n", &crate::config::default_toml()] {
            let (c, unknown) = Config::parse_noting(text).unwrap();
            assert!(c.board.quiet.is_empty(), "{text}");
            assert_eq!(Quiet::of(&c), Quiet::NONE);
            assert!(unknown.is_empty(), "{unknown:?}");
        }
    }

    #[test]
    fn only_the_origins_a_program_starts_can_be_quiet() {
        let c = Config::parse("[board]\nquiet = [\"app\", \"headless\"]\n").unwrap();
        assert_eq!(Quiet::of(&c).origins().collect::<Vec<_>>(), ORIGINS);
        for word in ["terminal", "night", "desktop", "App", ""] {
            let e = Config::parse(&format!("[board]\nquiet = [\"app\", \"{word}\"]\n"))
                .unwrap_err()
                .to_string();
            assert!(
                e.contains(&format!(
                    "[board] quiet: '{word}' cannot be quiet: give app or headless"
                )),
                "{e}"
            );
        }
        assert!(Config::parse("[board]\nquiet = \"app\"\n").is_err());
    }

    #[test]
    fn a_session_is_quiet_when_its_origin_is_listed() {
        let q = Quiet::of(&quiet(&["app"]));
        assert!(q.holds(Some("app")));
        assert!(!q.holds(Some("headless")) && !q.holds(Some("terminal")) && !q.holds(None));
        assert!(!Quiet::NONE.holds(Some("app")));
    }

    #[test]
    fn the_panel_counts_sessions_and_needs_you_as_if_nothing_were_quiet() {
        let db = Db::open_memory().unwrap();
        add(&db, "a1", Some("app"), Some("needs_you"));
        add(&db, "a2", Some("app"), Some("needs_you"));
        add(&db, "a3", Some("app"), Some("paused"));
        // a one-shot run the rules file in the archive still counts: quiet covers it too
        add(&db, "a4", Some("app"), None);
        db.conn
            .execute("UPDATE session SET prompt_count=1 WHERE id='a4'", [])
            .unwrap();
        // left out: hidden, and archived by hand
        add(&db, "a5", Some("app"), Some("needs_you"));
        db.set_hidden("a5", Some(true)).unwrap();
        add(&db, "a6", Some("app"), Some("needs_you"));
        db.conn
            .execute("UPDATE session SET archived=1 WHERE id='a6'", [])
            .unwrap();
        // not a program's
        add(&db, "t1", Some("terminal"), Some("needs_you"));
        add(&db, "u1", None, Some("needs_you"));
        let want = vec![Found {
            origin: "app",
            sessions: 4,
            needs_you: 2,
        }];
        assert_eq!(found(&db, &Config::default(), NOW).unwrap(), want);
        // turning it on changes the columns, not the counts that say what it clears
        assert_eq!(found(&db, &quiet(&["app"]), NOW).unwrap(), want);
    }

    #[test]
    fn an_origin_set_to_quiet_stays_listed_with_none_left_and_none_found_is_an_empty_list() {
        let db = Db::open_memory().unwrap();
        assert!(found(&db, &Config::default(), NOW).unwrap().is_empty());
        add(&db, "h1", Some("headless"), Some("done"));
        db.set_hidden("h1", Some(true)).unwrap();
        assert!(found(&db, &Config::default(), NOW).unwrap().is_empty());
        let got = found(&db, &quiet(&["headless"]), NOW).unwrap();
        assert_eq!(
            got,
            [Found {
                origin: "headless",
                sessions: 0,
                needs_you: 0,
            }]
        );
    }
}
