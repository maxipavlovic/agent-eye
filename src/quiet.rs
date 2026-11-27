//! `[board] quiet`: sessions a program started, kept quiet. A program that drives an agent (an app
//! on the agent's SDK, a script's headless run) has a user of its own, so the agent's questions are
//! not yours. A quiet session is Running while it works and in the archive otherwise, never in Needs
//! you by itself (`columns`), gets no summary from the daemon (`daemon::quiet_round`) or a bulk
//! `eye summarize --all`, and stays out of the steering clock (`clock::signals::left_out`). It is
//! still indexed, found and resumable. Each part
//! reads the setting when it runs, so a switch on the board counts without a restart; the picker
//! and `eye ls` read it when they start.

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

    /// The origins `[board] quiet` lists, in any case.
    pub fn of(cfg: &Config) -> Quiet {
        Quiet(ORIGINS.map(|o| cfg.board.quiet.iter().any(|q| origin_of(q) == Some(o))))
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

/// The origin a word of `[board] quiet` names, in any case (`App` is `app`); None for a word this
/// version does not know, which a newer eye may.
pub fn origin_of(word: &str) -> Option<&'static str> {
    ORIGINS
        .into_iter()
        .find(|o| o.eq_ignore_ascii_case(word.trim()))
}

/// The list as the file should hold it, each word as its origin, or why it cannot be: a word that
/// names no program origin, or an origin named twice.
pub fn canonical(words: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = vec![];
    for w in words {
        let o =
            origin_of(w).ok_or_else(|| format!("'{w}' cannot be quiet: give app or headless"))?;
        if out.iter().any(|k| k == o) {
            return Err(format!("'{w}' is listed twice"));
        }
        out.push(o.into());
    }
    Ok(out)
}

/// Split the list the file holds into the origins it names, once each and in lower case, and the
/// words this version does not know (left out, and reported as unknown settings are).
pub(crate) fn sort_out(words: &[String]) -> (Vec<String>, Vec<String>) {
    let mut known: Vec<String> = vec![];
    let mut unknown = vec![];
    for w in words {
        match origin_of(w) {
            Some(o) if !known.iter().any(|k| k == o) => known.push(o.into()),
            Some(_) => {}
            None => unknown.push(w.clone()),
        }
    }
    (known, unknown)
}

/// One program origin as the settings panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub origin: &'static str,
    /// Its sessions, but for hidden ones and ones archived by hand: what quiet covers. A throw-away
    /// one the rules file in the archive counts, since its summary and its prompts are quiet too.
    pub sessions: usize,
    /// Those quiet takes out of Needs you: in Needs you without it and not with it (a due reminder
    /// or a move by hand keeps one there). The same whether the origin is quiet now or not.
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
        let mut input = Input::from_session(&s, outputs.contains(&s.id), now, Quiet::NONE);
        let loud = columns::compute(&input);
        input.quiet = true;
        let hushed = columns::compute(&input);
        f.sessions += 1;
        f.needs_you += usize::from(loud == Column::NeedsYou && hushed != Column::NeedsYou);
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
    fn the_words_match_in_any_case_and_one_this_version_does_not_know_is_left_out_with_a_warning() {
        let (c, unknown) =
            Config::parse_noting("[board]\nquiet = [\"App\", \" HEADLESS \", \"app\"]\n").unwrap();
        assert_eq!(c.board.quiet, ORIGINS);
        assert_eq!(Quiet::of(&c).origins().collect::<Vec<_>>(), ORIGINS);
        assert!(unknown.is_empty(), "{unknown:?}");
        // a newer eye may know more origins: this one still loads the file
        let (c, unknown) =
            Config::parse_noting("[board]\nquiet = [\"app\", \"terminal\", \"robots\"]\n").unwrap();
        assert_eq!(c.board.quiet, ["app"]);
        assert_eq!(
            unknown,
            ["board.quiet \"robots\"", "board.quiet \"terminal\""]
        );
        // a value of the wrong type is still an error, as for any known key
        assert!(Config::parse("[board]\nquiet = \"app\"\n").is_err());
        // the board's POST is strict: a word it cannot place, or one origin twice, is refused
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            canonical(&words(&["Headless", "app"])).unwrap(),
            ["headless", "app"]
        );
        assert_eq!(
            canonical(&words(&["app", "terminal"])).unwrap_err(),
            "'terminal' cannot be quiet: give app or headless"
        );
        assert_eq!(
            canonical(&words(&["app", "App"])).unwrap_err(),
            "'App' is listed twice"
        );
    }

    #[test]
    fn a_session_is_quiet_when_its_origin_is_listed() {
        let q = Quiet::of(&quiet(&["app"]));
        assert!(q.holds(Some("app")));
        assert!(!q.holds(Some("headless")) && !q.holds(Some("terminal")) && !q.holds(None));
        assert!(!Quiet::NONE.holds(Some("app")));
    }

    #[test]
    fn the_panel_counts_sessions_and_the_ones_quiet_takes_out_of_needs_you() {
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
        // in Needs you for a reason quiet leaves alone: a due reminder, a move by hand
        add(&db, "a7", Some("app"), Some("needs_you"));
        add(&db, "a8", Some("app"), Some("paused"));
        db.conn
            .execute(
                "UPDATE session SET remind_at=?1 WHERE id='a7'",
                [NOW - 30_000],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE session SET column_manual='needs_you', column_manual_at=?1 WHERE id='a8'",
                [NOW],
            )
            .unwrap();
        // not a program's
        add(&db, "t1", Some("terminal"), Some("needs_you"));
        add(&db, "u1", None, Some("needs_you"));
        let want = vec![Found {
            origin: "app",
            sessions: 6,
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
