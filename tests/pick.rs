use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const S1: &str = "aaaaaaaa-0000-4000-8000-000000000001";

struct Env {
    tmp: tempfile::TempDir,
    work: std::path::PathBuf,
}

fn line(ts: &str, ty: &str, cwd: &Path, text: &str) -> String {
    let content = if ty == "user" {
        format!("\"{text}\"")
    } else {
        format!("[{{\"type\":\"text\",\"text\":\"{text}\"}}]")
    };
    format!(
        "{{\"type\":\"{ty}\",\"timestamp\":\"{ts}\",\"cwd\":\"{}\",\"sessionId\":\"{S1}\",\"message\":{{\"content\":{content}}}}}\n",
        cwd.display()
    )
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work dir");
        std::fs::create_dir_all(&work).unwrap();
        let proj = tmp.path().join("claude/projects/-demo");
        std::fs::create_dir_all(&proj).unwrap();
        let t = line(
            "2026-11-01T10:00:00.000Z",
            "user",
            &work,
            "Plan the zebra catalog migration for PROJ-4242",
        ) + &line(
            "2026-11-01T10:01:00.000Z",
            "assistant",
            &work,
            "Started on the zebra work",
        ) + &line(
            "2026-11-01T10:02:00.000Z",
            "user",
            &work,
            "Continue with the migration",
        ) + &line(
            "2026-11-01T10:03:00.000Z",
            "assistant",
            &work,
            "Done for now",
        );
        std::fs::write(proj.join(format!("{S1}.jsonl")), t).unwrap();
        Env { tmp, work }
    }

    fn eye(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_agent-eye"));
        c.env("EYE_CLAUDE_DIR", self.tmp.path().join("claude"))
            .env("EYE_HOME", self.tmp.path().join("home"))
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(std::process::Stdio::null());
        c
    }
}

#[test]
fn filter_lists_and_first_prints_command() {
    let e = Env::new();
    let out = e.eye(&["pick", "--filter", "zebra mig"]).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("aaaaaaaa"), "{text}");
    // a word that is in the index only through the transcript text, as a prefix
    let out = e.eye(&["pick", "--filter", "proj-42"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("aaaaaaaa"));
    let out = e.eye(&["pick", "--filter", "qqqqzzzz"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("no sessions match"));

    let out = e
        .eye(&["pick", "--filter", "zebra", "--first", "--print"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let cmd = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(
        cmd,
        format!("cd '{}' && claude --resume {S1}", e.work.display())
    );
    // the printed line is valid shell: run it with a fake claude on PATH
    let bin = e.tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"), "#!/bin/sh\necho \"$PWD|$*\"\n").unwrap();
    std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let ran = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ran.stdout).contains(&format!("|--resume {S1}")));
}

#[test]
fn no_match_first_fails() {
    let e = Env::new();
    let out = e
        .eye(&["pick", "--filter", "nomatchatall", "--first", "--print"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn enter_path_execs_claude_in_the_session_folder() {
    let e = Env::new();
    let rec = e.tmp.path().join("rec.txt");
    let fake = e.tmp.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s\\n%s\\n' \"$PWD\" \"$*\" > '{}'\n",
            rec.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = e
        .eye(&["pick", "--filter", "zebra", "--first"])
        .env("EYE_CLAUDE_BIN", &fake)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = std::fs::read_to_string(&rec).unwrap();
    let mut it = got.lines();
    let cwd = std::fs::canonicalize(it.next().unwrap()).unwrap();
    assert_eq!(cwd, std::fs::canonicalize(&e.work).unwrap());
    assert_eq!(it.next().unwrap(), format!("--resume {S1}"));
}

#[test]
fn idea_start_records_and_runs_in_folder() {
    let e = Env::new();
    let w = e.work.to_string_lossy().to_string();
    assert!(e
        .eye(&[
            "idea",
            "add",
            "Zephyr retry",
            "--notes=my zebra list",
            "--prompt=-rm everything",
            "--dir",
            &w
        ])
        .output()
        .unwrap()
        .status
        .success());
    let out = e
        .eye(&["pick", "--filter", "zephyr", "--first", "--print"])
        .output()
        .unwrap();
    let cmd = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        cmd.contains("claude --session-id ") && cmd.contains("'Task: -rm everything'"),
        "{cmd}"
    );
    assert!(!cmd.contains("zebra"), "notes are never sent: {cmd}");
    // recorded: until its session shows up, starting again reuses the same session id
    let again = e
        .eye(&["pick", "--filter", "zephyr", "--first", "--print"])
        .output()
        .unwrap();
    assert!(again.status.success());
    let sid = |c: &str| {
        c.split("--session-id ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .map(str::to_string)
    };
    assert_eq!(sid(&String::from_utf8_lossy(&again.stdout)), sid(&cmd));
}

#[test]
fn init_widgets_and_no_tty_list() {
    let e = Env::new();
    let z = e.eye(&["init", "zsh"]).output().unwrap();
    let z = String::from_utf8_lossy(&z.stdout);
    assert!(z.contains("bindkey '^G'"));
    let b = e
        .eye(&["init", "bash", "--key", "ctrl-o"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&b.stdout).contains("\\C-o"));
    // plain `pick` with no terminal prints the list instead of drawing
    let out = e.eye(&["pick"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("aaaaaaaa"));
    let out = e.eye(&[]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("aaaaaaaa"));
    // the generated zsh widget is valid syntax
    if Command::new("zsh").arg("-c").arg("true").status().is_ok() {
        let ok = Command::new("zsh")
            .arg("-n")
            .arg("-c")
            .arg(&*z)
            .status()
            .unwrap();
        assert!(ok.success());
    }
    let bb = e.eye(&["init", "bash"]).output().unwrap();
    let ok = Command::new("bash")
        .arg("-n")
        .arg("-c")
        .arg(String::from_utf8_lossy(&bb.stdout).as_ref())
        .status()
        .unwrap();
    assert!(ok.success());
}

#[test]
fn fork_prints_and_execs_with_fork_session() {
    let e = Env::new();
    let out = e
        .eye(&["pick", "--filter", "zebra", "--first", "--fork", "--print"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!(
            "cd '{}' && claude --resume {S1} --fork-session",
            e.work.display()
        )
    );
    let rec = e.tmp.path().join("rec.txt");
    let fake = e.tmp.path().join("fake-claude");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\n", rec.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = e
        .eye(&["pick", "--filter", "zebra", "--first", "--fork"])
        .env("EYE_CLAUDE_BIN", &fake)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&rec).unwrap().trim(),
        format!("--resume {S1} --fork-session")
    );
}

#[test]
fn resume_fork_and_show_conversation() {
    let e = Env::new();
    assert!(e.eye(&["scan"]).output().unwrap().status.success());
    let out = e
        .eye(&["resume", "aaaaaaaa", "--fork", "--dry-run"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!(
            "cd '{}' && claude --resume {S1} --fork-session",
            e.work.display()
        )
    );
    let out = e
        .eye(&["show", "aaaaaaaa", "--conversation", "-n", "3"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{text}");
    // the last three of four messages, oldest first
    assert!(!text.contains("Plan the zebra"), "{text}");
    let a = text.find("Started on the zebra work").unwrap();
    let b = text.find("Continue with the migration").unwrap();
    let c = text.find("Done for now").unwrap();
    assert!(a < b && b < c, "{text}");
    assert!(text.contains("you") && text.contains("claude"));
    assert!(text.starts_with("(1 earlier message)"), "{text}");
}
