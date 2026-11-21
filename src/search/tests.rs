use super::*;
use rusqlite::params;

const DAY_MS: i64 = 24 * 3600 * 1000;
const NOW: i64 = 100 * DAY_MS;

/// A session last active `days` ago: its title, summary (`recap`, searched as `extra`) and prompts.
fn put(db: &Db, id: &str, title: &str, recap: &str, body: &str, days: i64) {
    db.conn
        .execute(
            "INSERT INTO session(id,title,recap,cwd,kind,prompt_count,last_activity_at,ai_status,transcript_state)
             VALUES(?1,?2,?3,'/tmp','interactive',3,?4,'paused','present')",
            params![id, title, recap, NOW - days * DAY_MS],
        )
        .unwrap();
    crate::db::fts_write(&db.conn, id, body, "", true, 10_000, 10_000).unwrap();
}

fn sql(db: &Db, q: &str) {
    db.conn.execute(q, []).unwrap();
}

fn ids(h: Vec<Hit>) -> Vec<String> {
    h.into_iter().map(|h| h.session.id).collect()
}

fn find_q(db: &Db, q: &str, all: bool) -> Vec<String> {
    let words: Vec<String> = q.split_whitespace().map(String::from).collect();
    ids(find(db, &words, 10, all, NOW).unwrap())
}

fn first(db: &Db, q: &str) -> Option<String> {
    find_q(db, q, false).into_iter().next()
}

#[test]
fn word_forms_match() {
    let db = Db::open_memory().unwrap();
    put(&db, "tok", "Stale account token detection", "", "x", 5);
    put(&db, "git", "GitHub rebrand", "", "x", 5);
    put(&db, "dep", "Module deprecation research", "", "x", 5);
    assert_eq!(first(&db, "stale tokens").as_deref(), Some("tok"));
    assert_eq!(first(&db, "github rebranding").as_deref(), Some("git"));
    assert_eq!(first(&db, "deprecated modules").as_deref(), Some("dep"));
}

#[test]
fn a_title_with_all_the_words_beats_some_and_body_only() {
    let db = Db::open_memory().unwrap();
    put(&db, "body", "Alpha", "", "catalog migration plan", 1);
    put(&db, "some", "Catalog notes", "", "x", 2);
    put(&db, "all", "Catalog migration", "", "x", 90);
    let r = find_q(&db, "catalog migration", false);
    assert_eq!(r.len(), 3);
    assert_eq!(r[0], "all");
}

#[test]
fn a_rare_word_beats_a_common_word() {
    let db = Db::open_memory().unwrap();
    for n in 0..20 {
        put(&db, &format!("c{n}"), "Connect notes", "", "x", 5);
    }
    put(&db, "rare", "Alpha", "the quokka launch", "x", 5);
    // the title with only the common word does not beat the summary with the rare word
    assert_eq!(first(&db, "connect quokka").as_deref(), Some("rare"));
}

#[test]
fn half_the_words_are_enough_fewer_are_not() {
    let db = Db::open_memory().unwrap();
    put(&db, "dec", "Decimal separator", "", "x", 5);
    put(&db, "one", "Mexico trip", "", "x", 5);
    assert_eq!(find_q(&db, "decimal separator mexico", false), ["dec"]);
    // one word of four is less than half
    assert!(find_q(&db, "decimal alpha beta gamma", false).is_empty());
}

#[test]
fn typos_are_fixed_from_the_words_in_the_index() {
    let db = Db::open_memory().unwrap();
    put(&db, "tw", "Tradewind", "", "x", 5);
    put(&db, "hy", "Ory hydra", "", "x", 5);
    put(&db, "cm", "Catalog Migration", "", "x", 5);
    assert_eq!(first(&db, "tradwind").as_deref(), Some("tw"));
    assert_eq!(first(&db, "hydar").as_deref(), Some("hy"));
    assert_eq!(first(&db, "catalg migraton").as_deref(), Some("cm"));
    // too far from anything
    assert!(find_q(&db, "zzzzzz", false).is_empty());
}

#[test]
fn a_typo_one_session_holds_still_finds_the_right_spelling() {
    let db = Db::open_memory().unwrap();
    // enough other sessions for word rarity to mean what it does on a real index
    for i in 0..40 {
        put(
            &db,
            &format!("f{i}"),
            &format!("Unrelated work {i}"),
            "",
            "build and test",
            9,
        );
    }
    put(&db, "tw1", "Tradewind", "", "x", 5);
    put(&db, "tw2", "Tradewind account status", "", "x", 6);
    put(&db, "typed", "Search check", "", "find tradwind please", 1);
    let got = find_q(&db, "tradwind", false);
    assert_eq!(got.first().map(String::as_str), Some("tw1"), "{got:?}");
    assert!(got.contains(&"tw2".to_string()) && got.contains(&"typed".to_string()));
    // a short rare word is taken as typed: "lago" is not widened to "logo"
    put(&db, "lg", "Metrx vs Lago", "", "x", 2);
    put(&db, "lo", "New logo", "", "x", 2);
    put(&db, "lo2", "Logo colours", "", "x", 3);
    assert_eq!(find_q(&db, "lago", false), vec!["lg".to_string()]);
}

#[test]
fn a_pasted_link_finds_the_session_that_made_it() {
    let db = Db::open_memory().unwrap();
    put(&db, "made", "Pricing page", "", "x", 30);
    sql(
        &db,
        "INSERT INTO output(session_id,kind,value,first_seen_at)
         VALUES('made','artifact','https://claude.ai/artifact/7YcJXxwvqTV94FL6',1)",
    );
    crate::db::fts_write(&db.conn, "made", "x", "", true, 10_000, 10_000).unwrap();
    put(&db, "title", "Artifact notes", "", "x", 1);
    let q = "https://claude.ai/artifact/7YcJXxwvqTV94FL6?sk=abc123";
    assert_eq!(find_q(&db, q, false), ["made"]);
}

#[test]
fn a_dotted_word_is_a_phrase() {
    let db = Db::open_memory().unwrap();
    put(&db, "dns", "Retire connect.cloud.im", "", "x", 5);
    put(&db, "apart", "Cloud and im", "", "x", 1);
    assert_eq!(find_q(&db, "cloud.im", false), ["dns"]);
}

#[test]
fn an_old_session_can_be_first_and_an_archived_one_is_last() {
    let db = Db::open_memory().unwrap();
    put(&db, "old", "Zebra", "", "x", 90);
    sql(
        &db,
        "UPDATE session SET kind='history_only', transcript_state='gone' WHERE id='old'",
    );
    put(&db, "new", "Zebra notes and more words", "", "x", 0);
    put(&db, "arch", "Zebra", "", "x", 0);
    sql(&db, "UPDATE session SET archived=1 WHERE id='arch'");
    assert_eq!(find_q(&db, "zebra", true), ["old", "new", "arch"]);
    assert_eq!(find_q(&db, "zebra", false), ["old", "new"]);
}

#[test]
fn the_same_title_twice_is_newest_first_whatever_the_status() {
    let db = Db::open_memory().unwrap();
    put(&db, "older", "Zebra plan", "", "x", 20);
    put(&db, "newer", "Zebra plan", "", "x", 10);
    put(&db, "newest", "Zebra plan", "", "x", 1);
    sql(
        &db,
        "UPDATE session SET ai_status='needs_you' WHERE id='older'",
    );
    sql(&db, "UPDATE session SET ai_status='done' WHERE id='newest'");
    assert_eq!(
        find_q(&db, "zebra plan", false),
        ["newest", "newer", "older"]
    );
}

#[test]
fn only_the_last_word_is_a_prefix() {
    let db = Db::open_memory().unwrap();
    put(&db, "w", "Weekly report for Tarik", "", "x", 5);
    assert_eq!(first(&db, "weekly rep").as_deref(), Some("w"));
    // an earlier word is not a prefix: "rep weekly" has one word that matches, not two
    let score = |q: &str| {
        let w: Vec<String> = q.split_whitespace().map(String::from).collect();
        find(&db, &w, 5, false, NOW).unwrap()[0].score
    };
    assert!(score("weekly rep") > score("rep weekly"));
}

#[test]
fn a_session_id_prefix_comes_first() {
    let db = Db::open_memory().unwrap();
    put(
        &db,
        "5b8fe153-0000",
        "Pricing currencies",
        "",
        "old work",
        40,
    );
    put(
        &db,
        "other",
        "Mentions 5b8fe153 in text",
        "",
        "see 5b8fe153",
        1,
    );
    assert_eq!(find_q(&db, "5b8fe153", false)[0], "5b8fe153-0000");
}

#[test]
fn the_picker_and_find_give_the_same_order() {
    use crate::config::Config;
    let db = Db::open_memory().unwrap();
    put(&db, "a", "Catalog migration", "", "x", 30);
    put(
        &db,
        "b",
        "Catalog migration plan for the quarter",
        "",
        "x",
        1,
    );
    put(&db, "c", "Alpha", "catalog migration notes", "x", 2);
    put(&db, "d", "Catalog", "", "migration", 3);
    put(&db, "e", "Catalog migration", "", "x", 4);
    let idx = crate::pick::Index::load(&db, &Config::default(), NOW).unwrap();
    for q in ["catalog migration", "catalog", "migration cat", "catalg"] {
        let picker: Vec<String> = crate::pick::search(&db, &idx, q, false)
            .into_iter()
            .map(|i| idx.rows[i].id.clone())
            .collect();
        assert_eq!(picker, find_q(&db, q, false), "{q}");
    }
}

#[test]
fn only_punctuation_finds_nothing_and_quotes_are_safe() {
    let db = Db::open_memory().unwrap();
    put(&db, "a", "Zebra", "", "x", 5);
    assert!(find_q(&db, "!! --", false).is_empty());
    assert!(find_q(&db, "\"", false).is_empty());
    // a quote inside a word cannot break the query out of its phrase
    assert_eq!(first(&db, "zebra\" OR \"x").as_deref(), Some("a"));
}

#[test]
fn find_cuts_to_the_limit_after_ordering() {
    let db = Db::open_memory().unwrap();
    for n in 0..30 {
        put(
            &db,
            &format!("o{n:02}"),
            "Old",
            "",
            &"zebra ".repeat(10),
            60,
        );
    }
    put(&db, "title", "Zebra", "", "x", 90);
    let r = ids(find(&db, &["zebra".to_string()], 3, false, NOW).unwrap());
    assert_eq!(r.len(), 3);
    assert_eq!(r[0], "title");
}

#[test]
fn a_hit_has_a_snippet_and_a_score() {
    let db = Db::open_memory().unwrap();
    put(&db, "a", "Alpha", "", "we talked about the quokka today", 5);
    put(
        &db,
        "b",
        "Beta",
        "",
        "quokka quokka quokka and more about it",
        6,
    );
    let h = find(&db, &["quokka".to_string()], 5, false, NOW).unwrap();
    assert_eq!(h.len(), 2);
    assert!(h[0].snippet.contains("[quokka]"), "{}", h[0].snippet);
    assert!(h[0].score >= h[1].score);
}

#[test]
fn id_queries() {
    let w = |s: &str| vec![s.to_string()];
    assert_eq!(id_query(&w("5B8FE153")).as_deref(), Some("5b8fe153"));
    assert_eq!(
        id_query(&w("5b8fe153-f7cf")).as_deref(),
        Some("5b8fe153-f7cf")
    );
    assert_eq!(id_query(&w("cafe")), None); // a word, not an id
    assert_eq!(id_query(&w("5b8")), None);
    assert_eq!(id_query(&["5b8fe153".into(), "x".into()]), None);
}

#[test]
fn parsing_units() {
    let p = |s: &str| query::parse(&[s.to_string()]);
    let q = p("catalog mig\"ration! --");
    let phrases: Vec<String> = q.units.iter().map(|u| u.phrase()).collect();
    assert_eq!(phrases, ["\"catalog\"", "\"mig ration\"*"]);
    assert_eq!(q.words, 2);
    assert_eq!(p("LITE-35385").units[0].phrase(), "\"lite 35385\"*");
    assert_eq!(p("fail_pending").units[0].parts, ["fail", "pending"]);
    // two letters at the end are not enough to be a prefix
    assert_eq!(p("cloud im").units[1].phrase(), "\"im\"");
    assert!(p("!! --").is_empty());
    assert_eq!(p("a b c").need(), 2);
}

#[test]
fn parsing_links() {
    let p = |s: &str| query::parse(&[s.to_string()]);
    // an id-like segment is the whole search
    let q = p("https://claude.ai/artifact/7YcJXxwvqTV94FL6?sk=1#top");
    assert_eq!(q.units.len(), 1);
    assert_eq!(q.units[0].phrase(), "\"7ycjxxwvqtv94fl6\"");
    // otherwise the path
    let q = p("https://example.com/docs/guide?x=1");
    let t: Vec<String> = q.units.iter().map(|u| u.phrase()).collect();
    assert_eq!(t, ["\"docs\"", "\"guide\""]);
    assert!(p("https://example.com").is_empty());
}

#[test]
fn edit_distance() {
    use super::typo::distance;
    assert_eq!(distance("hydar", "hydra"), 1); // a swap is one change
    assert_eq!(distance("tradwind", "tradewind"), 1);
    assert_eq!(distance("migraton", "migrat"), 2);
    assert_eq!(distance("same", "same"), 0);
}

#[test]
fn a_3000_word_query_is_cut_and_still_finds_its_first_words() {
    let db = Db::open_memory().unwrap();
    put(&db, "a", "Quokka habitat", "", "x", 5);
    put(&db, "b", "Other", "", "x", 5);
    let q = "quokka habitat ".repeat(1500);
    let t = std::time::Instant::now();
    let got = find_q(&db, &q, false);
    assert!(t.elapsed().as_millis() < 500, "{:?}", t.elapsed());
    assert_eq!(parse_len(&q), 20);
    assert_eq!(got, ["a"]);
}

fn parse_len(q: &str) -> usize {
    let words: Vec<String> = q.split_whitespace().map(String::from).collect();
    query::parse(&words).units.len()
}

#[test]
fn a_link_as_the_last_unit_is_not_a_prefix_and_not_fixed() {
    let db = Db::open_memory().unwrap();
    put(&db, "made", "Pricing page", "", "x", 30);
    sql(
        &db,
        "INSERT INTO output(session_id,kind,value,first_seen_at)
         VALUES('made','artifact','https://claude.ai/artifact/7YcJXxwvqTV94FL6',1)",
    );
    crate::db::fts_write(&db.conn, "made", "x", "", true, 10_000, 10_000).unwrap();
    put(&db, "other", "Pricing review", "", "x", 1);
    let w = |s: &str| vec![s.to_string()];
    let q = query::parse(&w("https://claude.ai/artifact/7YcJXxwvqTV94FL6"));
    assert!(q.units.iter().all(|u| !u.prefix && !u.typo_ok));
    let q = query::parse(&[
        "pricing".into(),
        "https://claude.ai/artifact/7YcJXxwvqTV94FL6".into(),
    ]);
    assert!(!q.units.last().unwrap().prefix);
    // a cut-off id is neither completed as a prefix nor fixed as a typo
    assert!(find_q(&db, "https://claude.ai/artifact/7YcJXxwvqTV9", false).is_empty());
    assert!(find_q(&db, "https://claude.ai/artifact/7YcJXxwvqTV94FL7", false).is_empty());
    assert_eq!(
        first(&db, "pricing https://claude.ai/artifact/7YcJXxwvqTV94FL6").as_deref(),
        Some("made")
    );
}

#[test]
fn a_typo_in_the_last_unit_is_fixed() {
    let db = Db::open_memory().unwrap();
    for i in 0..20 {
        put(
            &db,
            &format!("f{i}"),
            &format!("Unrelated {i}"),
            "",
            "build",
            9,
        );
    }
    put(&db, "tw", "Tradewind pricing", "", "x", 5);
    assert_eq!(first(&db, "pricing tradwnd").as_deref(), Some("tw"));
    assert_eq!(first(&db, "pricing tradwind").as_deref(), Some("tw"));
}

#[test]
fn the_picker_and_find_agree_for_an_untitled_session_and_an_id_prefix() {
    use crate::config::Config;
    let db = Db::open_memory().unwrap();
    // untitled: the title is the start of the first prompt
    for (id, first_prompt, days) in [
        ("u1", "migrate the catalog service", 3),
        (
            "u2",
            "migrate the catalog service to the new cluster this week",
            2,
        ),
        ("u3", "catalog", 1),
    ] {
        put(&db, id, "", "", first_prompt, days);
        db.conn
            .execute(
                "UPDATE session SET title=NULL, first_prompt=?2 WHERE id=?1",
                params![id, first_prompt],
            )
            .unwrap();
    }
    // a hex prefix shared by 8 sessions, more than the old cap of 5
    for n in 0..8 {
        put(&db, &format!("ab12{n}000"), "Same", "", "x", n);
    }
    let idx = crate::pick::Index::load(&db, &Config::default(), NOW).unwrap();
    for q in ["catalog", "migrate catalog", "ab12"] {
        let picker: Vec<String> = crate::pick::search(&db, &idx, q, false)
            .into_iter()
            .map(|i| idx.rows[i].id.clone())
            .collect();
        let words: Vec<String> = q.split_whitespace().map(String::from).collect();
        let found = ids(find(&db, &words, 50, false, NOW).unwrap());
        assert_eq!(picker, found, "{q}");
    }
    assert_eq!(find_q(&db, "ab12", false).len(), 8);
}

#[test]
fn an_id_hit_has_a_finite_score_above_the_scored_hits() {
    let db = Db::open_memory().unwrap();
    put(&db, "5b8fe153-0000", "Other", "", "x", 1);
    put(&db, "m", "Mentions 5b8fe153", "", "see 5b8fe153", 1);
    let w = vec!["5b8fe153".to_string()];
    let h = find(&db, &w, 5, false, NOW).unwrap();
    assert_eq!(h[0].session.id, "5b8fe153-0000");
    assert!(h[0].score.is_finite());
    assert_eq!(h[0].score, h[1].score + 1.0);
    // an id hit alone scores 1.0
    let db = Db::open_memory().unwrap();
    put(&db, "5b8fe153-0000", "Other", "", "x", 1);
    assert_eq!(find(&db, &w, 5, false, NOW).unwrap()[0].score, 1.0);
}
