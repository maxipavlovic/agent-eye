//! `eye export` and `eye import`: a pack moves everything to another home, merges into an existing
//! one, refuses damaged and newer packs, and writes one session as a page.

use agent_eye::backup::{self, Reason};
use agent_eye::config::Config;
use agent_eye::db::{Db, SCHEMA_VERSION};
use agent_eye::install::Opts;
use agent_eye::pack;
use agent_eye::paths::Paths;
use agent_eye::platform::service::{Manager, Timing};
use agent_eye::scan::scan;
use std::path::{Path, PathBuf};

const S1: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const S2: &str = "bbbbbbbb-0000-4000-8000-000000000002";
const S3: &str = "cccccccc-0000-4000-8000-000000000003";
const SECRET: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn transcript(id: &str, cwd: &str, prompt: &str, hour: u32) -> String {
    format!(
        "{{\"type\":\"user\",\"timestamp\":\"2026-11-01T{hour:02}:01:40.000Z\",\"cwd\":\"{cwd}\",\"sessionId\":\"{id}\",\"message\":{{\"content\":\"{prompt}\"}}}}\n{{\"type\":\"assistant\",\"timestamp\":\"2026-11-01T{hour:02}:02:40.000Z\",\"cwd\":\"{cwd}\",\"sessionId\":\"{id}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"Done with {prompt}\"}}]}}}}\n"
    )
}

/// Claude's folder name for a path.
fn enc(p: &Path) -> String {
    p.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A fake user home with Claude Code's folder and eye's home inside, as the binary sees it.
struct Home {
    home: PathBuf,
}

impl Home {
    fn new(root: &Path, rel: &str) -> Home {
        let home = root.join(rel);
        std::fs::create_dir_all(home.join(".claude/projects")).unwrap();
        Home { home }
    }
    fn code(&self) -> PathBuf {
        self.home.join("code")
    }
    fn eye_home(&self) -> PathBuf {
        self.home.join(".agent-eye")
    }
    fn write_transcript(&self, id: &str, prompt: &str, hour: u32) -> PathBuf {
        let dir = self.home.join(".claude/projects").join(enc(&self.code()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{id}.jsonl"));
        std::fs::write(
            &p,
            transcript(id, &self.code().to_string_lossy(), prompt, hour),
        )
        .unwrap();
        p
    }
    fn eye(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", &self.home)
            .env_remove("EYE_HOME")
            .env_remove("EYE_CLAUDE_DIR")
            .env("PATH", "/usr/bin:/bin")
            .env("NO_COLOR", "1")
            .args(args)
            .current_dir(&self.home)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let o = self.eye(args);
        let t = text(&o);
        assert!(o.status.success(), "eye {args:?}: {t}");
        t
    }
}

fn text(o: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn json(h: &Home, args: &[&str]) -> serde_json::Value {
    let o = h.eye(args);
    assert!(o.status.success(), "{}", text(&o));
    serde_json::from_slice(&o.stdout).unwrap()
}

/// The ids in a home's database, read directly (`eye ls` leaves old sessions out).
fn session_ids(h: &Home) -> Vec<String> {
    let db = Db::open(&h.eye_home().join("eye.db")).unwrap();
    let mut v: Vec<String> = db
        .all_sessions()
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    v.sort();
    v
}

fn find_pack(dir: &Path) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "eyepack"))
        .expect("a pack in the folder")
}

/// Home A: two sessions (one archived, one only with the agent), an idea.
fn home_a(root: &Path) -> Home {
    let a = Home::new(root, "a/Users/max");
    a.write_transcript(S1, "first task", 10);
    let p2 = a.write_transcript(S2, &format!("second task with {SECRET}"), 11);
    a.ok(&["scan"]);
    a.ok(&["archive", S1]);
    assert!(p2.exists());
    a.ok(&[
        "idea",
        "add",
        "Try the pack",
        "--dir",
        &a.code().to_string_lossy(),
    ]);
    a
}

#[test]
fn a_pack_moves_everything_to_an_empty_home_with_another_path() {
    let t = tempfile::tempdir().unwrap();
    let a = home_a(t.path());
    let out = t.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let said = a.ok(&["export", "--to", &out.to_string_lossy()]);
    assert!(
        said.contains("2 sessions") && said.contains("1 ideas"),
        "{said}"
    );
    assert!(said.contains("1 transcripts copied"), "{said}");
    let pack_file = find_pack(&out);
    let name = pack_file
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with("agent-eye-") && name.ends_with(".eyepack"),
        "{name}"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&pack_file).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let b = Home::new(t.path(), "b/home/max");
    std::fs::create_dir_all(b.eye_home()).unwrap();
    let plan = b.ok(&["import", &pack_file.to_string_lossy(), "--dry-run"]);
    assert!(plan.contains("/Users/max") && plan.contains("->"), "{plan}");
    assert!(
        plan.contains("2 sessions") && plan.contains("do not exist"),
        "{plan}"
    );
    assert!(plan.contains("dry run"), "{plan}");
    // a dry run changed nothing
    assert_eq!(session_ids(&b).len(), 0);
    assert!(backup::list(&backup::dir(&Paths { home: b.eye_home() })).is_empty());
    assert!(!b.eye_home().join("archive").exists());

    let done = b.ok(&["import", &pack_file.to_string_lossy(), "--yes"]);
    assert!(
        done.contains("2 new and 0 updated sessions, 1 ideas"),
        "{done}"
    );

    // same sessions, same searches, paths remapped
    assert_eq!(session_ids(&a), session_ids(&b));
    assert_eq!(session_ids(&b).len(), 2);
    let ids = |v: &serde_json::Value| {
        let mut x: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect();
        x.sort();
        x
    };
    let fa = json(&a, &["find", "second", "--json"]);
    let fb = json(&b, &["find", "second", "--json"]);
    assert_eq!(ids(&fa), ids(&fb));
    assert_eq!(ids(&fb), vec![S2.to_string()]);
    let dbb = Db::open(&b.eye_home().join("eye.db")).unwrap();
    for s in dbb.all_sessions().unwrap() {
        assert_eq!(s.cwd.as_deref(), Some(b.code().to_str().unwrap()));
    }
    drop(dbb);
    let ideas = json(&b, &["idea", "ls", "--json"]);
    assert_eq!(ideas.as_array().unwrap().len(), 1);
    assert_eq!(
        ideas[0]["folder"].as_str().unwrap(),
        b.code().to_string_lossy()
    );

    // resume would put each transcript back in the remapped Claude project folder
    let db = Db::open(&b.eye_home().join("eye.db")).unwrap();
    for id in [S1, S2] {
        let s = db.session_by_id(id).unwrap().unwrap();
        let want = b
            .home
            .join(".claude/projects")
            .join(enc(&b.code()))
            .join(format!("{id}.jsonl"));
        assert_eq!(s.transcript_path.as_deref(), Some(want.to_str().unwrap()));
        assert_eq!(s.project_dir.as_deref(), Some(enc(&b.code()).as_str()));
        let copy = s.archive_path.clone().unwrap();
        assert!(
            Path::new(&copy).starts_with(b.eye_home().join("archive")),
            "{copy}"
        );
        assert!(Path::new(&copy).exists(), "{copy}");
        assert_eq!(s.transcript_state.as_deref(), Some("archived"));
        assert!(!want.exists());
        b.ok(&["restore", id]);
        assert!(want.exists());
        assert_eq!(
            std::fs::read(&want).unwrap(),
            std::fs::read(
                a.home
                    .join(".claude/projects")
                    .join(enc(&a.code()))
                    .join(format!("{id}.jsonl"))
            )
            .unwrap()
        );
    }
    drop(db);
    // even a fresh home keeps a snapshot of what it had (an empty database) before the import
    assert!(b.ok(&["backup", "ls"]).contains("pre-import"));
    // the board's token never travels
    assert!(!b.eye_home().join("board.token").exists());
}

fn opts(paths: &Paths) -> Opts {
    Opts {
        eye_home: paths.home.clone(),
        bin_dir: paths.home.join("bin"),
        package: None,
        old_bin_dir: None,
        manager: Manager::current(),
        service_dir: paths.home.join("agents"),
        exe: std::env::current_exe().unwrap(),
        dry_run: false,
        no_service: true,
        purge: false,
        board: false,
        programs: None,
        timing: Timing::REAL,
    }
}

/// An eye home with its own Claude folder, scanned.
struct Local {
    paths: Paths,
    cfg: Config,
    claude: PathBuf,
}

impl Local {
    fn new(root: &Path, name: &str) -> Local {
        let claude = root.join(name).join("claude");
        std::fs::create_dir_all(claude.join("projects/-tmp-demo")).unwrap();
        let paths = Paths {
            home: root.join(name).join("eye"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::load_or_create(&paths.config_path()).unwrap();
        cfg.override_agent_dir("claude", claude.clone());
        Local { paths, cfg, claude }
    }
    fn session(&self, id: &str, prompt: &str, hour: u32) {
        std::fs::write(
            self.claude.join(format!("projects/-tmp-demo/{id}.jsonl")),
            transcript(id, "/tmp/demo", prompt, hour),
        )
        .unwrap();
    }
    fn scan(&self) -> Db {
        let mut db = Db::open(&self.paths.db_path()).unwrap();
        scan(&mut db, &self.cfg, false).unwrap();
        db
    }
}

fn export_to(l: &Local, db: &Db, file: &Path) {
    let agents = agent_eye::agents::enabled(&l.cfg);
    pack::export(db, &l.paths, &agents, file).unwrap();
}

fn import_in(l: &Local, file: &Path) -> pack::Plan {
    pack::plan(&l.paths, file).unwrap()
}

fn count(db: &Db, sql: &str) -> i64 {
    db.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn merging_adds_new_sessions_takes_newer_rows_and_adds_only_new_ideas() {
    let t = tempfile::tempdir().unwrap();
    let (a, c) = (Local::new(t.path(), "a"), Local::new(t.path(), "c"));
    // S1 is on both, newer in the pack (A); S2 only in A; S3 only in C; S4 on both, newer in C
    a.session(S1, "alpha newer", 12);
    a.session(S2, "beta only here", 10);
    c.session(S1, "alpha older", 9);
    c.session(S3, "gamma local", 10);
    const S4: &str = "dddddddd-0000-4000-8000-000000000004";
    a.session(S4, "delta in a", 8);
    c.session(S4, "delta in c", 13);
    let da = a.scan();
    let dc = c.scan();
    for (db, title, folder, at) in [
        (&da, "shared idea", "/tmp/demo", 100),
        (&da, "idea only in a", "/tmp/demo", 200),
        (&dc, "shared idea", "/tmp/demo", 100),
    ] {
        db.conn
            .execute(
                "INSERT INTO idea(title, folder, created_at, archived) VALUES(?1,?2,?3,0)",
                rusqlite::params![title, folder, at],
            )
            .unwrap();
    }
    agent_eye::archive::archive_all(&da, &a.paths, &agent_eye::agents::enabled(&a.cfg)).unwrap();
    let pack_file = t.path().join("a.eyepack");
    export_to(&a, &da, &pack_file);
    drop((da, dc));

    let plan = import_in(&c, &pack_file);
    assert!(!plan.fresh);
    assert_eq!(plan.new_sessions, vec![S2.to_string()]);
    assert_eq!(plan.updated_sessions, vec![S1.to_string()]);
    assert_eq!(plan.unchanged_sessions, 1);
    assert_eq!(plan.new_ideas, 1);
    assert_eq!(plan.files_copy, 3);
    let done = pack::apply(&opts(&c.paths), &c.cfg, plan).unwrap();
    assert_eq!(done.ideas_added, 1);
    assert_eq!(done.files_copied, 3);
    assert!(done.applied.pre_restore.is_some());

    let db = Db::open(&c.paths.db_path()).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM session"), 4);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM idea"), 2);
    let first = |id: &str| db.session_by_id(id).unwrap().unwrap().first_prompt.unwrap();
    assert_eq!(first(S1), "alpha newer", "the pack's row is newer");
    assert_eq!(first(S4), "delta in c", "the local row is newer");
    assert_eq!(first(S2), "beta only here");
    // the text index follows the rows: an imported and an updated session are found
    let found = |word: &str| -> i64 {
        db.conn
            .query_row(
                "SELECT COUNT(*) FROM session s JOIN session_fts f ON f.rowid=s.rowid WHERE session_fts MATCH ?1",
                [word],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(found("beta"), 1);
    assert_eq!(found("newer"), 1);
    assert_eq!(found("older"), 0, "the replaced row's text is gone");
    assert_eq!(found("gamma"), 1);
    // a pre-import snapshot was made, and the archive files came across
    assert!(backup::list(&backup::dir(&c.paths))
        .iter()
        .any(|e| e.reason == Reason::PreImport));
    assert!(c
        .paths
        .home
        .join("archive/-tmp-demo")
        .join(format!("{S2}.jsonl.zst"))
        .exists());
    // importing the same pack again changes nothing
    let again = import_in(&c, &pack_file);
    assert!(again.new_sessions.is_empty() && again.updated_sessions.is_empty());
    assert_eq!((again.new_ideas, again.files_copy), (0, 0));
}

/// Unpack `pack_file`, let `tamper` change the manifest, and write it again as `out`.
fn repack(pack_file: &Path, out: &Path, tamper: impl FnOnce(&mut pack::Manifest)) {
    let dir = tempfile::tempdir().unwrap();
    let mut m = pack::read(pack_file, dir.path()).unwrap().manifest;
    tamper(&mut m);
    let items: Vec<pack::Item> = m
        .files
        .keys()
        .map(|n| pack::Item {
            name: n.clone(),
            src: dir.path().join(n),
        })
        .collect();
    pack::write(out, &m, &items).unwrap();
}

#[test]
fn a_damaged_or_newer_pack_is_refused_before_anything_changes() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (Local::new(t.path(), "a"), Local::new(t.path(), "b"));
    a.session(S1, "alpha", 10);
    let da = a.scan();
    let good = t.path().join("good.eyepack");
    export_to(&a, &da, &good);

    let bad = t.path().join("bad.eyepack");
    repack(&good, &bad, |m| {
        m.files.get_mut("eye.db").unwrap().sha256 = "0".repeat(64);
    });
    let e = pack::plan(&b.paths, &bad)
        .err()
        .expect("refused")
        .to_string();
    assert!(e.contains("damaged") && e.contains("eye.db"), "{e}");

    let newer = t.path().join("newer.eyepack");
    repack(&good, &newer, |m| m.schema = SCHEMA_VERSION + 1);
    let e = pack::plan(&b.paths, &newer)
        .err()
        .expect("refused")
        .to_string();
    assert!(e.contains("newer eye"), "{e}");

    let future = t.path().join("future.eyepack");
    repack(&good, &future, |m| m.format = pack::FORMAT + 1);
    assert!(pack::plan(&b.paths, &future).is_err());

    // cut short: the end of the file is missing
    let cut = t.path().join("cut.eyepack");
    let bytes = std::fs::read(&good).unwrap();
    std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
    assert!(pack::plan(&b.paths, &cut).is_err());
    // not a pack at all
    let junk = t.path().join("junk.eyepack");
    std::fs::write(&junk, b"hello").unwrap();
    assert!(pack::plan(&b.paths, &junk).is_err());

    // nothing changed in B, and the scratch folder is gone
    let db = Db::open(&b.paths.db_path()).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM session"), 0);
    assert!(!b.paths.home.join("archive").exists());
    assert!(backup::list(&backup::dir(&b.paths)).is_empty());
    assert!(std::fs::read_dir(b.paths.home.join("tmp")).map_or(true, |mut d| d.next().is_none()));
}

#[test]
fn the_commands_refuse_without_asking_in_a_script_and_keep_the_old_database() {
    let t = tempfile::tempdir().unwrap();
    let a = home_a(t.path());
    a.ok(&[
        "export",
        "--to",
        &t.path().join("p.eyepack").to_string_lossy(),
    ]);
    let b = Home::new(t.path(), "b/home/max");
    b.write_transcript(S3, "local work", 9);
    b.ok(&["scan"]);
    let asked = b.eye(&["import", &t.path().join("p.eyepack").to_string_lossy()]);
    assert!(!asked.status.success());
    assert!(text(&asked).contains("--yes"), "{}", text(&asked));
    assert_eq!(session_ids(&b).len(), 1);
    let done = b.ok(&[
        "import",
        &t.path().join("p.eyepack").to_string_lossy(),
        "--yes",
    ]);
    assert!(done.contains("2 new and 0 updated"), "{done}");
    assert_eq!(session_ids(&b).len(), 3);
    let ls = b.ok(&["backup", "ls"]);
    assert!(ls.contains("pre-import"), "{ls}");
}

#[test]
fn one_session_exports_as_a_page_with_secrets_masked() {
    let t = tempfile::tempdir().unwrap();
    let a = home_a(t.path());
    let out = t.path().join("page");
    std::fs::create_dir_all(&out).unwrap();
    let said = a.ok(&["export", S2, "--html", "--to", &out.to_string_lossy()]);
    assert!(said.contains(".html"), "{said}");
    let file = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    let html = std::fs::read_to_string(&file).unwrap();
    assert!(
        html.contains("second task") && html.contains("Done with"),
        "{html}"
    );
    assert!(!html.contains(SECRET), "the secret must be masked");
    assert!(html.contains("[masked]"));
    for bad in ["<link", "<script", "<img", "src=\"http", "@import"] {
        assert!(!html.contains(bad), "{bad}");
    }
    // the agent deleted the transcript: the page is made from the archive
    a.ok(&["archive", S2]);
    std::fs::remove_file(
        a.home
            .join(".claude/projects")
            .join(enc(&a.code()))
            .join(format!("{S2}.jsonl")),
    )
    .unwrap();
    let md = t.path().join("s2.md");
    a.ok(&["export", S2, "--md", "--to", &md.to_string_lossy()]);
    let md = std::fs::read_to_string(&md).unwrap();
    assert!(md.contains("### You") && md.contains("second task"), "{md}");
    // flags without a session are an error
    assert!(!a.eye(&["export", "--html"]).status.success());
}
