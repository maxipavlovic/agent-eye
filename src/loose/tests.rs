use super::find::{find, Input, Sess};
use super::probe::{parse_docker_ps, Docker, Probe, System};
use super::*;
use crate::db::Db;
use crate::platform::procs::{Info, Proc};
use crate::worked;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const NOW: i64 = 1_800_000_000_000;

/// A machine that answers from tables.
#[derive(Default)]
struct Fake {
    procs: Vec<Proc>,
    boxes: Vec<Docker>,
    git: HashMap<String, String>,
    roots: Vec<PathBuf>,
    infos: HashMap<u32, Info>,
    killed: Mutex<Vec<u32>>,
    acted: Mutex<Vec<String>>,
    act_fails: Option<String>,
}

impl Probe for Fake {
    fn processes(&self, want: &dyn Fn(&str) -> bool) -> Vec<Proc> {
        self.procs
            .iter()
            .filter(|p| want(&p.command))
            .cloned()
            .collect()
    }
    fn proc_info(&self, pid: u32) -> Option<Info> {
        self.infos.get(&pid).cloned()
    }
    fn terminate(&self, pid: u32) -> std::io::Result<()> {
        self.killed.lock().unwrap().push(pid);
        Ok(())
    }
    fn docker(&self) -> Vec<Docker> {
        self.boxes.clone()
    }
    fn docker_stop(&self, id: &str) -> Result<String, String> {
        self.acted.lock().unwrap().push(format!("docker stop {id}"));
        Ok(id.into())
    }
    fn git(&self, dir: &Path, args: &[&str]) -> Option<String> {
        self.git
            .get(&format!("{} {}", dir.display(), args.join(" ")))
            .cloned()
    }
    fn git_act(&self, dir: &Path, args: &[&str]) -> Result<String, String> {
        self.acted
            .lock()
            .unwrap()
            .push(format!("{} git {}", dir.display(), args.join(" ")));
        match &self.act_fails {
            Some(m) => Err(m.clone()),
            None => Ok(String::new()),
        }
    }
    fn scratch_roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }
}

fn sess(id: &str, dir: &str, last: i64) -> Sess {
    Sess {
        id: id.into(),
        cwd: Some(dir.into()),
        repo_root: None,
        last,
        live: false,
    }
}

fn proc(pid: u32, start_s: i64, cmd: &str, cwd: &str, ports: &[u16]) -> Proc {
    Proc {
        pid,
        ppid: 1,
        start: start_s,
        command: cmd.into(),
        cwd: Some(cwd.into()),
        ports: ports.to_vec(),
    }
}

fn input(sessions: Vec<Sess>, hours: &[(&str, i64, i64)]) -> Input {
    Input {
        sessions,
        events: hours
            .iter()
            .map(|(s, h, n)| ((s.to_string(), *h), *n))
            .collect(),
        home: Some(PathBuf::from("/Users/zed")),
        ..Input::default()
    }
}

#[test]
fn a_session_id_in_the_working_folder_names_the_owner() {
    let fake = Fake {
        procs: vec![proc(
            38090,
            NOW / 1000 - 3600,
            "node /x/server.js",
            "/private/tmp/claude-501/proj/aaaa-1111/scratch",
            &[4321],
        )],
        roots: vec!["/private/tmp/claude-501".into()],
        ..Fake::default()
    };
    let inp = input(vec![sess("aaaa-1111", "/Users/zed/code/a", NOW)], &[]);
    let found = find(&inp, &fake, NOW);
    assert_eq!(found.len(), 1);
    let f = &found[0];
    assert_eq!(
        (f.session_id.as_str(), f.kind),
        ("aaaa-1111", Kind::Process)
    );
    assert_eq!(f.key, format!("proc:38090:{}", NOW / 1000 - 3600));
    assert_eq!(
        f.confirm().unwrap(),
        "Stop node server.js (pid 38090, port 4321)?"
    );
    assert!(f.line.contains("port 4321") && f.line.ends_with("ago"));
}

#[test]
fn the_folder_and_the_hour_name_the_owner_and_most_events_wins() {
    let start = NOW / 1000 - 5 * 3600;
    let hour = worked::hour_of(start * 1000);
    let fake = Fake {
        procs: vec![proc(
            7,
            start,
            "python3 -m http.server 8000",
            "/Users/zed/code/a/web",
            &[8000],
        )],
        ..Fake::default()
    };
    let inp = input(
        vec![
            sess("s-quiet", "/Users/zed/code/a", NOW - 1),
            sess("s-busy", "/Users/zed/code/a", NOW - 2),
            sess("s-other-hour", "/Users/zed/code/a", NOW),
            sess("s-elsewhere", "/Users/zed/code/b", NOW),
        ],
        &[
            ("s-quiet", hour, 2),
            ("s-busy", hour, 9),
            ("s-other-hour", hour + 1, 50),
            ("s-elsewhere", hour, 99),
        ],
    );
    let found = find(&inp, &fake, NOW);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].session_id, "s-busy");
    // nobody worked in that hour: not listed
    let inp = input(
        vec![sess("s-x", "/Users/zed/code/a", NOW)],
        &[("s-x", hour + 3, 5)],
    );
    assert!(find(&inp, &fake, NOW).is_empty());
    // a session folder that is the home folder claims nothing
    let inp = input(
        vec![sess("s-home", "/Users/zed", NOW)],
        &[("s-home", hour, 5)],
    );
    assert!(find(&inp, &fake, NOW).is_empty());
}

#[test]
fn agents_shells_editors_and_helpers_are_never_listed() {
    let start = NOW / 1000 - 3600;
    let hour = worked::hour_of(start * 1000);
    let dir = "/Users/zed/code/a";
    let fake = Fake {
        procs: vec![
            proc(1, start, "/opt/homebrew/bin/claude --resume x", dir, &[]),
            proc(2, start, "codex resume y", dir, &[]),
            proc(3, start, "-zsh", dir, &[]),
            proc(4, start, "/Users/zed/.local/bin/eye daemon", dir, &[7420]),
            proc(5, start, "nvim notes.md", dir, &[]),
            proc(
                6,
                start,
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                dir,
                &[9222],
            ),
            proc(7, start, "npx @modelcontextprotocol/server-x mcp", dir, &[]),
            proc(8, start, "rust-analyzer", dir, &[]),
            proc(9, start, "sleep 600", dir, &[]),
        ],
        ..Fake::default()
    };
    let inp = input(vec![sess("s1", dir, NOW)], &[("s1", hour, 3)]);
    let found = find(&inp, &fake, NOW);
    let pids: Vec<&str> = found.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(pids.len(), 1, "{pids:?}");
    assert!(pids[0].starts_with("proc:9:"));
    assert!(found[0].line.starts_with("sleep (pid 9) is still running"));
}

#[test]
fn a_container_is_owned_through_its_compose_folder_or_a_bind_mount() {
    let created = NOW / 1000 - 7200;
    let hour = worked::hour_of(created * 1000);
    let web = Docker {
        id: "a".repeat(64),
        name: "web".into(),
        image: "nginx".into(),
        created,
        workdir: Some("/Users/zed/code/a".into()),
        mounts: vec![],
    };
    let db_box = Docker {
        id: "b".repeat(64),
        name: "db".into(),
        image: "postgres".into(),
        created,
        workdir: None,
        mounts: vec!["/Users/zed/code/c/data".into()],
    };
    let stranger = Docker {
        id: "c".repeat(64),
        name: "other".into(),
        image: "redis".into(),
        created,
        workdir: Some("/Users/zed/code/zzz".into()),
        mounts: vec![],
    };
    let fake = Fake {
        boxes: vec![web, db_box, stranger],
        ..Fake::default()
    };
    let inp = input(
        vec![
            sess("s-a", "/Users/zed/code/a", NOW),
            sess("s-c", "/Users/zed/code/c", NOW),
        ],
        &[("s-a", hour, 1), ("s-c", hour, 1)],
    );
    let found = find(&inp, &fake, NOW);
    let by: Vec<(&str, &str)> = found
        .iter()
        .map(|f| (f.session_id.as_str(), f.kind.as_str()))
        .collect();
    assert_eq!(by, vec![("s-a", "container"), ("s-c", "container")]);
    assert!(found[0]
        .confirm()
        .unwrap()
        .starts_with("Stop container web (aaaaaaaaaaaa)"));
}

#[test]
fn docker_ps_json_is_read() {
    let line = r#"{"ID":"abc123","Names":"web","Image":"nginx","Labels":"a=b,com.docker.compose.project.working_dir=/w/x,c=d","Mounts":"vol1,/host/dir","CreatedAt":"2026-11-21 10:00:00 +0100 CET"}"#;
    let c = &parse_docker_ps(&format!("{line}\nnot json\n"))[0];
    assert_eq!(c.workdir.as_deref(), Some("/w/x"));
    assert_eq!(c.mounts, vec!["/host/dir".to_string()]);
    assert_eq!(c.created, 1_795_251_600);
}

/// Real temp folders for sessions, and git answers keyed `dir args`.
fn git_world() -> (tempfile::TempDir, Input, Fake) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let scratch = tmp.path().join("scratch/proj/s-git/wt");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();
    let r = repo.display().to_string();
    let wl = format!(
        "worktree {r}\nHEAD abc\nbranch refs/heads/main\n\nworktree {}\nHEAD def\nbranch refs/heads/x\n\nworktree /gone/place\nHEAD 111\ndetached\nprunable gitdir file points to non-existent location\n",
        scratch.display()
    );
    let mut git = HashMap::new();
    git.insert(format!("{r} worktree list --porcelain"), wl);
    git.insert(format!("{r} remote"), "origin\n".into());
    git.insert(
        format!("{r} log --branches --not --remotes --format=%h%x09%ct%x09%s"),
        "aaa1111\t1799999000\tnewest\nbbb2222\t1799990000\toldest\n".into(),
    );
    git.insert(
        format!("{r} status --porcelain -uno"),
        " M src/a.rs\nM  src/b.rs\n?? scratch.txt\n".into(),
    );
    let inp = input(
        vec![Sess {
            id: "s-git".into(),
            cwd: Some(r.clone()),
            repo_root: Some(r),
            last: NOW,
            live: false,
        }],
        &[],
    );
    let fake = Fake {
        git,
        roots: vec![tmp.path().join("scratch")],
        ..Fake::default()
    };
    (tmp, inp, fake)
}

#[test]
fn git_leftovers_are_found_with_plain_lines() {
    let (_tmp, inp, fake) = git_world();
    let found = find(&inp, &fake, NOW);
    let kinds: Vec<Kind> = found.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Kind::Worktree,
            Kind::Worktree,
            Kind::Unpushed,
            Kind::Uncommitted
        ]
    );
    let by = |k: Kind| found.iter().filter(|f| f.kind == k).collect::<Vec<_>>();
    assert!(by(Kind::Unpushed)[0]
        .line
        .starts_with("2 commits in repo are on no remote"));
    assert_eq!(by(Kind::Unpushed)[0].since_ms, 1_799_990_000_000);
    assert!(by(Kind::Unpushed)[0].key.ends_with(":aaa1111"));
    let u = by(Kind::Uncommitted)[0];
    assert!(
        u.line
            .starts_with("2 modified files in repo are not committed"),
        "{}",
        u.line
    );
    assert!(!u.detail.contains("scratch.txt"));
    let w: Vec<&str> = by(Kind::Worktree)
        .iter()
        .map(|f| f.detail.lines().last().unwrap())
        .collect();
    assert!(w.contains(&"scratchpad") && w.contains(&"folder gone"));
    // the copy commands are plain git, never run
    assert!(copy_command(by(Kind::Unpushed)[0])
        .unwrap()
        .ends_with("&& git push --all"));
    assert!(copy_command(u).unwrap().ends_with("&& git commit -a"));
    assert_eq!(copy_command(by(Kind::Worktree)[0]), None);
}

#[test]
fn a_repo_with_no_remote_has_no_unpushed_and_an_open_session_owns_nothing() {
    let (_tmp, inp, mut fake) = git_world();
    let key = fake
        .git
        .keys()
        .find(|k| k.ends_with(" remote"))
        .unwrap()
        .clone();
    fake.git.insert(key, String::new());
    let kinds: Vec<Kind> = find(&inp, &fake, NOW).iter().map(|f| f.kind).collect();
    assert!(!kinds.contains(&Kind::Unpushed));
    let mut live = inp.clone();
    live.sessions[0].live = true;
    assert!(find(&live, &fake, NOW).is_empty());
}

fn open_db() -> (tempfile::TempDir, Db) {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(&tmp.path().join("eye.db")).unwrap();
    (tmp, db)
}

fn item(key: &str, sid: &str, kind: Kind, what: &str) -> LooseEnd {
    LooseEnd {
        key: key.into(),
        session_id: sid.into(),
        kind,
        line: format!("{what} line"),
        detail: what.into(),
        since_ms: 5,
    }
}

#[test]
fn results_are_replaced_and_an_ignore_survives_the_next_scan() {
    let (_t, db) = open_db();
    assert!(list(&db, None, false).unwrap().is_empty());
    assert!(counts(&db).unwrap().is_empty());
    let a = item("proc:1:2", "s1", Kind::Process, "node (pid 1, port 4321)");
    let b = item("unpushed:/r:abc", "s1", Kind::Unpushed, "3 commits in r");
    let c = item("proc:9:9", "s2", Kind::Process, "sleep (pid 9)");
    replace(&db, &[a.clone(), b.clone(), c], NOW).unwrap();
    assert_eq!(counts(&db).unwrap()["s1"], 2);
    assert_eq!(list(&db, Some("s2"), false).unwrap().len(), 1);
    assert!(ignore(&db, "nope", NOW).is_err());
    ignore(&db, &a.key, NOW).unwrap();
    assert_eq!(counts(&db).unwrap()["s1"], 1);
    assert_eq!(list(&db, Some("s1"), true).unwrap().len(), 2);
    // a new scan replaces the rows, and the ignore still holds
    replace(&db, &[a.clone(), b.clone()], NOW + 1).unwrap();
    assert_eq!(list(&db, None, false).unwrap(), vec![b.clone()]);
    // all ignored: the count is gone
    ignore(&db, &b.key, NOW).unwrap();
    assert!(!counts(&db).unwrap().contains_key("s1"));
    assert_eq!(summary_line(&[]), None);
}

#[test]
fn the_summary_line_reads_like_a_sentence() {
    let items = vec![
        item(
            "proc:1:1",
            "s",
            Kind::Process,
            "node vite (pid 1, port 4350)",
        ),
        item(
            "proc:2:1",
            "s",
            Kind::Process,
            "node api (pid 2, port 4321)",
        ),
        item("proc:3:1", "s", Kind::Process, "sleep (pid 3)"),
        item("unpushed:/r:a", "s", Kind::Unpushed, "3 commits in r"),
        item(
            "uncommitted:/r",
            "s",
            Kind::Uncommitted,
            "1 modified file in /r",
        ),
    ];
    assert_eq!(
        summary_line(&items).unwrap(),
        "Left behind: 2 servers (ports 4321, 4350), 1 process still running, 3 commits not pushed, 1 file not committed"
    );
}

#[test]
fn stop_acts_only_when_pid_and_start_still_match() {
    let (_t, db) = open_db();
    let me = crate::platform::uid();
    let it = item(
        "proc:500:1000",
        "s1",
        Kind::Process,
        "node (pid 500, port 80)",
    );
    replace(&db, std::slice::from_ref(&it), NOW).unwrap();
    // the pid now belongs to a process that started later
    let mut fake = Fake::default();
    fake.infos.insert(
        500,
        Info {
            uid: me,
            start: 2000,
        },
    );
    let e = stop(&db, &fake, &it.key).unwrap_err().to_string();
    assert!(e.contains("another process"), "{e}");
    assert!(fake.killed.lock().unwrap().is_empty());
    assert!(
        get(&db, &it.key).unwrap().is_none(),
        "a stale item is forgotten"
    );
    // gone
    replace(&db, std::slice::from_ref(&it), NOW).unwrap();
    let e = stop(&db, &Fake::default(), &it.key)
        .unwrap_err()
        .to_string();
    assert!(e.contains("not running any more"));
    // someone else's process
    replace(&db, std::slice::from_ref(&it), NOW).unwrap();
    let mut fake = Fake::default();
    fake.infos.insert(
        500,
        Info {
            uid: me + 1,
            start: 1000,
        },
    );
    assert!(stop(&db, &fake, &it.key)
        .unwrap_err()
        .to_string()
        .contains("not yours"));
    assert!(fake.killed.lock().unwrap().is_empty());
    // a match is stopped, with SIGTERM only
    let mut fake = Fake::default();
    fake.infos.insert(
        500,
        Info {
            uid: me,
            start: 1000,
        },
    );
    let said = stop(&db, &fake, &it.key).unwrap();
    assert!(said.starts_with("sent SIGTERM to node (pid 500"));
    assert_eq!(*fake.killed.lock().unwrap(), vec![500]);
    // a key that no scan stored is refused, and so is the wrong kind
    assert!(stop(&db, &fake, "proc:1:1").is_err());
    let wt = item("worktree:/w", "s1", Kind::Worktree, "/w\nrepo /r");
    replace(&db, std::slice::from_ref(&wt), NOW).unwrap();
    assert!(stop(&db, &fake, &wt.key).is_err());
}

#[test]
fn a_container_is_stopped_only_when_id_and_creation_match() {
    let (_t, db) = open_db();
    let it = item(
        "box:abcdef:77",
        "s1",
        Kind::Container,
        "container web (abcdef)",
    );
    replace(&db, std::slice::from_ref(&it), NOW).unwrap();
    let mut fake = Fake::default();
    fake.boxes.push(Docker {
        id: "abcdef".into(),
        name: "web".into(),
        image: "i".into(),
        created: 78,
        workdir: None,
        mounts: vec![],
    });
    assert!(stop(&db, &fake, &it.key).is_err());
    assert!(fake.acted.lock().unwrap().is_empty());
    replace(&db, std::slice::from_ref(&it), NOW).unwrap();
    fake.boxes[0].created = 77;
    stop(&db, &fake, &it.key).unwrap();
    assert_eq!(
        *fake.acted.lock().unwrap(),
        vec!["docker stop abcdef".to_string()]
    );
}

#[test]
fn a_worktree_is_removed_without_force_and_a_refusal_is_passed_on() {
    let (_t, db) = open_db();
    let tmp = tempfile::tempdir().unwrap();
    let wt_dir = tmp.path().join("wt");
    std::fs::create_dir_all(&wt_dir).unwrap();
    let wt = wt_dir.display().to_string();
    let it = item(
        &format!("worktree:{wt}"),
        "s1",
        Kind::Worktree,
        &format!("{wt}\nrepo /r\nscratchpad"),
    );
    replace(&db, std::slice::from_ref(&it), NOW).unwrap();
    let mut fake = Fake::default();
    fake.git.insert(
        "/r worktree list --porcelain".into(),
        format!("worktree /r\nHEAD a\n\nworktree {wt}\nHEAD b\n"),
    );
    fake.act_fails =
        Some("fatal: contains modified or untracked files, use --force to delete it".into());
    let e = remove(&db, &fake, &it.key).unwrap_err().to_string();
    assert_eq!(
        e,
        "fatal: contains modified or untracked files, use --force to delete it"
    );
    let acted = fake.acted.lock().unwrap().clone();
    assert_eq!(acted, vec![format!("/r git worktree remove -- {wt}")]);
    assert!(!acted[0].contains("--force"));
    assert!(
        get(&db, &it.key).unwrap().is_some(),
        "a refused removal stays listed"
    );
    fake.act_fails = None;
    remove(&db, &fake, &it.key).unwrap();
    assert!(get(&db, &it.key).unwrap().is_none());
    // git does not list it any more: nothing is run
    replace(&db, std::slice::from_ref(&it), NOW).unwrap();
    let mut fake = Fake::default();
    fake.git.insert(
        "/r worktree list --porcelain".into(),
        "worktree /r\nHEAD a\n".into(),
    );
    assert!(remove(&db, &fake, &it.key).is_err());
    assert!(fake.acted.lock().unwrap().is_empty());
}

#[test]
fn the_message_sent_back_is_one_plain_line() {
    let items = vec![
        item("proc:1:1", "s", Kind::Process, "node (pid 1)"),
        item(
            "uncommitted:/r",
            "s",
            Kind::Uncommitted,
            "2 modified files in r",
        ),
    ];
    let p = send_prompt(&items);
    assert!(!p.contains('\n'));
    assert!(p.contains("node (pid 1) line; 2 modified files in r line"));
    assert!(p.contains("instead of pushing anything"));
}

#[test]
fn a_schema_13_file_gets_empty_loose_tables() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("eye.db");
    {
        let db = Db::open(&path).unwrap();
        db.conn
            .execute_batch("DROP TABLE loose; DROP TABLE loose_ignore;")
            .unwrap();
        db.meta_set("schema_version", "13").unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(crate::db::SCHEMA_VERSION.to_string())
    );
    assert!(list(&db, None, true).unwrap().is_empty());
}

/// The one test that starts a real process: its own `sleep`, in a folder named like a session
/// scratchpad. It is found, stopped, and a key with another start time is refused.
#[test]
fn a_process_of_our_own_is_found_and_stopped_only_on_a_matching_start() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude-501");
    let sid = "11111111-2222-3333-4444-555555555555";
    let dir = root.join("proj").join(sid);
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("120")
        .current_dir(&dir)
        .spawn()
        .unwrap();
    let pid = child.id();
    let sys = System::with_roots(vec![root]);
    let inp = input(vec![sess(sid, "/Users/zed/code/none", NOW)], &[]);
    // the process table needs a moment to show a new process with its folder
    let mut mine = vec![];
    for _ in 0..40 {
        mine = find(&inp, &sys, crate::util::now_ms())
            .into_iter()
            .filter(|f| f.key.starts_with(&format!("proc:{pid}:")))
            .collect();
        if !mine.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let outcome = std::panic::catch_unwind(|| {
        assert_eq!(mine.len(), 1, "the scan finds the throwaway process");
        assert_eq!(mine[0].session_id, sid);
        let (_t, db) = open_db();
        // a key with the wrong start time is refused and nothing is signalled
        let wrong = format!("proc:{pid}:1");
        let bad = LooseEnd {
            key: wrong.clone(),
            ..mine[0].clone()
        };
        replace(&db, &[bad], NOW).unwrap();
        assert!(stop(&db, &sys, &wrong)
            .unwrap_err()
            .to_string()
            .contains("another process"));
        // the right key stops it
        replace(&db, &mine, NOW).unwrap();
        stop(&db, &sys, &mine[0].key).unwrap();
    });
    match outcome {
        Ok(()) => {
            let st = child.wait().unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(st.signal(), Some(libc::SIGTERM));
        }
        Err(panic) => {
            // only our own throwaway child is ever killed here
            let _ = child.kill();
            let _ = child.wait();
            std::panic::resume_unwind(panic);
        }
    }
}

#[test]
fn a_command_that_just_started_and_listens_on_no_port_is_not_a_leftover_yet() {
    let dir = "/Users/zed/code/a";
    let start = NOW / 1000 - 30;
    let hour = worked::hour_of(start * 1000);
    let fake = Fake {
        procs: vec![
            proc(1, start, "ugrep -r thing .", dir, &[]),
            proc(2, start, "node server.js", dir, &[3000]),
            proc(3, NOW / 1000 - 600, "tail -f log", dir, &[]),
        ],
        ..Fake::default()
    };
    let mut inp = input(
        vec![sess("s1", dir, NOW)],
        &[("s1", hour, 3), ("s1", worked::hour_of(NOW - 600_000), 3)],
    );
    inp.quiet_min_age_s = 120;
    let keys: Vec<String> = find(&inp, &fake, NOW).into_iter().map(|f| f.key).collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(
        keys.iter().any(|k| k.starts_with("proc:2:"))
            && keys.iter().any(|k| k.starts_with("proc:3:"))
    );
    let line = &find(&inp, &fake, NOW)
        .into_iter()
        .find(|f| f.key.starts_with("proc:2:"))
        .unwrap()
        .line;
    assert!(line.ends_with("started just now"), "{line}");
}

fn real_git(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Real git in temp folders: a worktree in a scratchpad is found, git itself refuses to remove it
/// while it has changes, and removes it once it has none. A worktree whose folder was deleted is
/// found as prunable and cleared.
#[test]
fn real_git_finds_and_removes_a_scratch_worktree_and_refuses_one_with_changes() {
    if !real_git(Path::new("/"), &["--version"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("work/repo");
    let sid = "99999999-0000-4000-8000-000000000009";
    let wt = tmp.path().join("claude-501/proj").join(sid).join("wt");
    let gone = tmp.path().join("elsewhere/gone");
    std::fs::create_dir_all(&repo).unwrap();
    assert!(real_git(&repo, &["init", "-q", "-b", "main"]));
    std::fs::write(repo.join("a.txt"), "1").unwrap();
    assert!(real_git(&repo, &["add", "a.txt"]));
    assert!(real_git(&repo, &["commit", "-q", "-m", "first"]));
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    assert!(real_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "scratch",
            &wt.display().to_string()
        ]
    ));
    std::fs::create_dir_all(gone.parent().unwrap()).unwrap();
    assert!(real_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            &gone.display().to_string()
        ]
    ));
    std::fs::remove_dir_all(&gone).unwrap();
    std::fs::write(wt.join("a.txt"), "changed").unwrap();

    let sys = System::with_roots(vec![tmp.path().join("claude-501")]);
    let r = repo.display().to_string();
    let inp = input(
        vec![Sess {
            id: sid.into(),
            cwd: Some(r.clone()),
            repo_root: Some(r),
            last: NOW,
            live: false,
        }],
        &[],
    );
    let found: Vec<LooseEnd> = find(&inp, &sys, NOW)
        .into_iter()
        .filter(|f| f.kind == Kind::Worktree)
        .collect();
    assert_eq!(found.len(), 2, "{found:?}");
    let scratch = found
        .iter()
        .find(|f| f.detail.ends_with("scratchpad"))
        .unwrap();
    let missing = found
        .iter()
        .find(|f| f.detail.ends_with("folder gone"))
        .unwrap();
    let (_t, db) = open_db();
    replace(&db, &found, NOW).unwrap();
    // git refuses, and says so; the folder and the item stay
    let e = remove(&db, &sys, &scratch.key).unwrap_err().to_string();
    assert!(e.contains("contains modified or untracked files"), "{e}");
    assert!(wt.exists() && get(&db, &scratch.key).unwrap().is_some());
    // once nothing is modified, it goes (without --force)
    assert!(real_git(&wt, &["checkout", "--", "a.txt"]));
    remove(&db, &sys, &scratch.key).unwrap();
    assert!(!wt.exists() && get(&db, &scratch.key).unwrap().is_none());
    // a folder that is gone: eye drops its item, says what to run, and git's record stays
    let said = remove(&db, &sys, &missing.key).unwrap();
    assert!(
        said.contains("the folder is gone") && said.contains("git worktree prune"),
        "{said}"
    );
    assert!(said.contains(&repo.display().to_string()), "{said}");
    assert!(get(&db, &missing.key).unwrap().is_none());
    let left = System::new()
        .git(&repo, &["worktree", "list", "--porcelain"])
        .unwrap();
    assert_eq!(left.matches("worktree ").count(), 2, "{left}");
}
