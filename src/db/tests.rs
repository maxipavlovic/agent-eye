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
