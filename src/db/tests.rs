use super::*;

#[test]
fn migration_adds_hidden_to_an_old_database() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE session(id TEXT PRIMARY KEY, archived INTEGER NOT NULL DEFAULT 0)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO session(id) VALUES('old')", [])
        .unwrap();
    let db = Db::init(conn).unwrap();
    let v: Option<i64> = db
        .conn
        .query_row("SELECT hidden FROM session WHERE id='old'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(v, None);
    db.set_hidden("old", Some(true)).unwrap();
    let v: Option<i64> = db
        .conn
        .query_row("SELECT hidden FROM session WHERE id='old'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(v, Some(1));
}

#[test]
fn migration_to_v4_adds_token_columns_and_rereads_transcripts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eye.db");
    {
        // a v3 database: the current tables without the token columns, the idea prompt, the
        // waiting note, the agent columns and the cursor marker
        let db = Db::open(&path).unwrap();
        db.conn
            .execute("ALTER TABLE cursor DROP COLUMN marker", [])
            .unwrap();
        db.conn
            .execute("ALTER TABLE session DROP COLUMN agent", [])
            .unwrap();
        db.conn
            .execute("ALTER TABLE idea DROP COLUMN agent", [])
            .unwrap();
        for c in ["waiting_note", "waiting_prompt"] {
            db.conn
                .execute(&format!("ALTER TABLE session DROP COLUMN {c}"), [])
                .unwrap();
        }
        db.conn
            .execute("ALTER TABLE idea DROP COLUMN prompt", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO idea(title, notes) VALUES('kept', 'my notes')",
                [],
            )
            .unwrap();
        for c in [
            "tok_in",
            "tok_out",
            "tok_cw",
            "tok_cr",
            "tok_sub",
            "tok_last_id",
        ] {
            db.conn
                .execute(&format!("ALTER TABLE session DROP COLUMN {c}"), [])
                .unwrap();
        }
        db.conn
            .execute("INSERT INTO session(id) VALUES('old')", [])
            .unwrap();
        db.conn
            .execute("INSERT INTO cursor VALUES('/t.jsonl',1,1,1)", [])
            .unwrap();
        db.meta_set("schema_version", "3").unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert!(db.cursor_get("/t.jsonl").unwrap().is_none());
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    assert_eq!(db.session_by_id("old").unwrap().unwrap().tokens, None);
    // rows from before eye knew several agents are Claude Code's; an idea has no agent set
    assert_eq!(db.session_by_id("old").unwrap().unwrap().agent, "claude");
    let i = crate::ideas::get(&db, 1).unwrap().unwrap();
    assert_eq!((i.notes.as_deref(), i.prompt), (Some("my notes"), None));
    assert_eq!(i.agent, None);
    db.conn
        .execute("UPDATE session SET tok_in=5, tok_sub=2 WHERE id='old'", [])
        .unwrap();
    let t = db.session_by_id("old").unwrap().unwrap().tokens.unwrap();
    assert_eq!(
        (t.input, t.output, t.subagents_total, t.total()),
        (5, 0, 2, 7)
    );
    // opening again keeps cursors written since
    db.conn
        .execute(
            "INSERT INTO cursor(path,size,mtime,offset) VALUES('/u.jsonl',1,1,1)",
            [],
        )
        .unwrap();
    drop(db);
    assert!(Db::open(&path)
        .unwrap()
        .cursor_get("/u.jsonl")
        .unwrap()
        .is_some());
}

#[test]
fn migration_to_v12_adds_the_worked_tables_and_keeps_the_cursors_as_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eye.db");
    {
        // a v11 database: no tables for the work timeline, sessions read to the end
        let db = Db::open(&path).unwrap();
        db.conn.execute("DROP TABLE worked_hour", []).unwrap();
        db.conn.execute("DROP TABLE worked_state", []).unwrap();
        db.conn
            .execute(
                "INSERT INTO session(id, transcript_state) VALUES('s1','present')",
                [],
            )
            .unwrap();
        cursor_set(&db.conn, "/t.jsonl", Stamp { size: 9, mtime: 4 }, "9").unwrap();
        db.meta_set("schema_version", "11").unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    let c = db.cursor_get("/t.jsonl").unwrap().unwrap();
    assert_eq!((c.stamp.size, c.marker.as_str()), (9, "9"));
    let waiting: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM session s WHERE NOT EXISTS (SELECT 1 FROM worked_state w WHERE w.session_id=s.id)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(waiting, 1);
}

#[test]
fn migration_to_v10_adds_the_live_columns_and_keeps_the_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eye.db");
    {
        // a v9 database: sessions without the columns of background work
        let db = Db::open(&path).unwrap();
        for c in ["live_wait", "remote_url", "signals", "live_loops"] {
            db.conn
                .execute(&format!("ALTER TABLE session DROP COLUMN {c}"), [])
                .unwrap();
        }
        db.conn
            .execute(
                "INSERT INTO session(id, agent, live_status) VALUES('s1', 'claude', 'idle')",
                [],
            )
            .unwrap();
        db.meta_set("schema_version", "9").unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    let s = db.session_by_id("s1").unwrap().unwrap();
    assert_eq!(s.live_status.as_deref(), Some("idle"));
    assert!(s.signals.is_empty() && s.live_loops.is_empty());
    assert_eq!((s.live_wait, s.remote_url), (None, None));
    db.conn
        .execute(
            "UPDATE session SET signals='{\"tasks\":[{\"id\":\"b\",\"kind\":\"shell\",\"label\":\"x\",\"started_at\":1}]}' WHERE id='s1'",
            [],
        )
        .unwrap();
    drop(db);
    // opening it again changes nothing, and tasks without loops still read
    let db = Db::open(&path).unwrap();
    let s = db.session_by_id("s1").unwrap().unwrap();
    assert_eq!(s.signals.tasks.len(), 1);
}

#[test]
fn migration_to_v9_keeps_every_offset_as_a_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eye.db");
    {
        // a v8 database: the cursors still hold byte offsets and have no marker column
        let db = Db::open(&path).unwrap();
        db.conn
            .execute("ALTER TABLE cursor DROP COLUMN marker", [])
            .unwrap();
        for (p, size, mtime, offset) in [("/a.jsonl", 100, 20, 90), ("/b.jsonl", 0, 7, 0)] {
            db.conn
                .execute(
                    "INSERT INTO cursor(path,size,mtime,offset) VALUES(?1,?2,?3,?4)",
                    params![p, size, mtime, offset],
                )
                .unwrap();
        }
        db.meta_set("schema_version", "8").unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    let at = |p: &str| {
        let c = db.cursor_get(p).unwrap().unwrap();
        (c.stamp, c.marker)
    };
    assert_eq!(
        at("/a.jsonl"),
        (
            Stamp {
                size: 100,
                mtime: 20
            },
            "90".into()
        )
    );
    assert_eq!(at("/b.jsonl"), (Stamp { size: 0, mtime: 7 }, "0".into()));
    let stored: String = db
        .conn
        .query_row("SELECT marker FROM cursor WHERE path='/a.jsonl'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, "90", "copied into the column, not only derived");
    drop(db);
    // opening again changes nothing
    let db = Db::open(&path).unwrap();
    assert_eq!(db.cursor_get("/a.jsonl").unwrap().unwrap().marker, "90");
}

#[test]
fn a_cursor_written_now_is_safe_for_a_build_from_before_markers() {
    let db = Db::open_memory().unwrap();
    let stamp = Stamp { size: 50, mtime: 9 };
    cursor_set(&db.conn, "/s", stamp, "row:42").unwrap();
    let c = db.cursor_get("/s").unwrap().unwrap();
    assert_eq!((c.stamp, c.marker.as_str()), (stamp, "row:42"));
    // the old column holds a number past any end of file, so an older build reads from the start
    let legacy: i64 = db
        .conn
        .query_row("SELECT offset FROM cursor WHERE path='/s'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(legacy, i64::MAX);
    // an older build ran in between and moved the offset: that is the newest word
    db.conn
        .execute("UPDATE cursor SET offset=33 WHERE path='/s'", [])
        .unwrap();
    assert_eq!(db.cursor_get("/s").unwrap().unwrap().marker, "33");
    cursor_set(&db.conn, "/s", stamp, "row:60").unwrap();
    assert_eq!(db.cursor_get("/s").unwrap().unwrap().marker, "row:60");
    assert!(db.cursor_get("/none").unwrap().is_none());
}

#[test]
fn a_column_another_process_added_first_is_not_an_error() {
    let db = Db::open_memory().unwrap();
    db.add_column("session", "remind_at INTEGER").unwrap();
    assert!(db.add_column("nope", "x INTEGER").is_err());
}

#[test]
fn schema_and_meta() {
    let db = Db::open_memory().unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    assert!(db.conn.prepare("SELECT hidden FROM session").is_ok());
    db.conn
        .execute("INSERT INTO session(id) VALUES('abcdef12-0000')", [])
        .unwrap();
    assert_eq!(db.resolve_id("abcdef").unwrap(), "abcdef12-0000");
    assert!(db.resolve_id("abc").is_err());
    assert!(db.resolve_id("zzzzzz").is_err());
    db.conn
        .execute("INSERT INTO session(id) VALUES('abcdef99-0000')", [])
        .unwrap();
    assert!(db
        .resolve_id("abcdef")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
}

#[test]
fn fts_append_and_cap() {
    let db = Db::open_memory().unwrap();
    db.conn
        .execute("INSERT INTO session(id,title) VALUES('s1','T')", [])
        .unwrap();
    fts_write(&db.conn, "s1", "alpha beta ", "", true, 20, 20).unwrap();
    fts_write(&db.conn, "s1", "gamma delta", "", false, 20, 20).unwrap();
    let p: String = db
        .conn
        .query_row("SELECT prompts FROM session_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(p, "pha beta gamma delta");
    let n: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM session_fts WHERE session_fts MATCH 'delta'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

/// A schema-12 file: the index has the old tokenizer and rows already in it.
fn old_index() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute(
        "CREATE TABLE session(id TEXT PRIMARY KEY, title TEXT, recap TEXT, state_line TEXT, next_step TEXT, archived INTEGER NOT NULL DEFAULT 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE VIRTUAL TABLE session_fts USING fts5(session_id UNINDEXED, title, prompts, replies, extra)",
        [],
    )
    .unwrap();
    for (n, (id, title)) in [("s1", "Stale tokens"), ("s2", "Billing"), ("s3", "Notes")]
        .into_iter()
        .enumerate()
    {
        conn.execute("INSERT INTO session(id,title) VALUES(?1,?2)", [id, title])
            .unwrap();
        conn.execute(
            "INSERT INTO session_fts(rowid,session_id,title,prompts,replies,extra) VALUES(?1,?2,?3,'we rebranded it','','')",
            params![n as i64 + 1, id, title],
        )
        .unwrap();
    }
    conn.execute("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT)", [])
        .unwrap();
    conn.execute("INSERT INTO meta VALUES('schema_version','12')", [])
        .unwrap();
    conn
}

fn count(db: &Db, q: &str) -> i64 {
    db.conn
        .query_row(
            "SELECT count(*) FROM session_fts WHERE session_fts MATCH ?1",
            [q],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn migration_13_stems_the_index_and_keeps_every_row() {
    let conn = old_index();
    // the old tokenizer does not know word forms
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM session_fts WHERE session_fts MATCH 'token'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    let db = Db::init(conn).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    let rows: Vec<(i64, String, String)> = db
        .conn
        .prepare("SELECT rowid, session_id, title FROM session_fts ORDER BY rowid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        [
            (1, "s1".into(), "Stale tokens".into()),
            (2, "s2".into(), "Billing".into()),
            (3, "s3".into(), "Notes".into())
        ]
    );
    // word forms work, in the title and in the text
    assert_eq!(count(&db, "token"), 1);
    assert_eq!(count(&db, "rebranding"), 3);
    // the word list for typo fixing exists and holds stems
    let doc: i64 = db
        .conn
        .query_row(
            "SELECT doc FROM session_fts_vocab WHERE term='rebrand'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(doc, 3);
    // opening it again, or another process opening it, changes nothing
    db.migrate().unwrap();
    db.stem_session_fts().unwrap();
    assert_eq!(count(&db, "token"), 1);
    // a row written afterwards is stemmed too
    db.conn
        .execute(
            "INSERT INTO session(id,title) VALUES('s4','Tokens again')",
            [],
        )
        .unwrap();
    fts_write(&db.conn, "s4", "x", "", true, 100, 100).unwrap();
    assert_eq!(count(&db, "token"), 2);
}

#[test]
fn a_new_database_stems_from_the_start() {
    let db = Db::open_memory().unwrap();
    let sql: String = db
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='session_fts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sql.contains("porter"));
}

#[test]
fn the_word_list_is_readable_and_not_empty_after_migration_13() {
    let db = Db::init(old_index()).unwrap();
    let n: i64 = db
        .conn
        .query_row("SELECT count(*) FROM session_fts_vocab", [], |r| r.get(0))
        .unwrap();
    assert!(n > 0);
}

/// An index from before `usage_day`: the tables are gone and the version is `from`.
fn index_without_usage(dir: &std::path::Path, from: i64) -> Connection {
    let path = dir.join("old.db");
    {
        let db = Db::open(&path).unwrap();
        db.conn
            .execute_batch("DROP TABLE usage_day; DROP TABLE usage_state;")
            .unwrap();
        db.meta_set("schema_version", &from.to_string()).unwrap();
        db.conn
            .execute("INSERT INTO session(id, tok_in) VALUES('s1', 5)", [])
            .unwrap();
    }
    Connection::open(&path).unwrap()
}

#[test]
fn migration_15_adds_the_usage_tables_from_13_and_from_14_and_keeps_every_row() {
    for from in [13, 14] {
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::init(index_without_usage(tmp.path(), from)).unwrap();
        assert_eq!(
            db.meta_get("schema_version").unwrap(),
            Some(SCHEMA_VERSION.to_string())
        );
        assert_eq!(count_rows(&db, "usage_day"), 0);
        assert_eq!(count_rows(&db, "usage_state"), 0);
        assert_eq!(count_rows(&db, "session"), 1);
        // nothing is re-read for them: the cursors are left alone
        assert!(db.cursor_get("/none").unwrap().is_none());
    }
}

fn count_rows(db: &Db, table: &str) -> i64 {
    db.conn
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn migration_16_adds_night_run_from_15_and_keeps_every_row() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("old.db");
    {
        let db = Db::open(&path).unwrap();
        db.conn.execute_batch("DROP TABLE night_run;").unwrap();
        db.meta_set("schema_version", "15").unwrap();
        db.conn
            .execute("INSERT INTO session(id, tok_in) VALUES('s1', 5)", [])
            .unwrap();
    }
    let db = Db::init(Connection::open(&path).unwrap()).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    // the migration to 16 ran: this test only asks that a later schema keeps its table
    // the table is there and empty, and nothing else moved
    assert_eq!(count_rows(&db, "night_run"), 0);
    assert_eq!(count_rows(&db, "session"), 1);
    assert!(crate::night::list(&db, false).unwrap().is_empty());
    assert_eq!(crate::night::planned_count(&db).unwrap(), 0);
}

#[test]
fn migration_17_adds_origin_from_16_and_keeps_every_row() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("old.db");
    {
        let db = Db::open(&path).unwrap();
        db.conn
            .execute_batch("ALTER TABLE session DROP COLUMN origin; ALTER TABLE session DROP COLUMN origin_raw;")
            .unwrap();
        db.meta_set("schema_version", "16").unwrap();
        db.conn
            .execute("INSERT INTO session(id, tok_in) VALUES('s1', 5)", [])
            .unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    assert_eq!(SCHEMA_VERSION, 17);
    // the columns are there and empty (the scan fills them), and nothing else moved
    let s = db.session_by_id("s1").unwrap().unwrap();
    assert_eq!((s.origin, s.origin_raw), (None, None));
    assert_eq!(s.tokens.unwrap().input, 5);
    assert_eq!(count_rows(&db, "session"), 1);
    // the night folder sits next to the database
    assert_eq!(db.night_dir, Some(tmp.path().join("night")));
    assert_eq!(Db::open_memory().unwrap().night_dir, None);
}
