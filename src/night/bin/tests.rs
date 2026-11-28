use super::*;
use std::os::unix::fs::PermissionsExt;

fn program(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap(); // NOSONAR: a test program must run
    p
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn the_folder_holds_eye_and_agent_eye_as_links_to_the_running_binary() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = program(tmp.path(), "agent-eye", "echo new");
    let bin = tmp.path().join("home/bin");
    link_to(&bin, &exe).unwrap();
    assert_eq!(names(&bin), ["agent-eye", "eye"]);
    for n in ["agent-eye", "eye"] {
        assert_eq!(std::fs::read_link(bin.join(n)).unwrap(), exe, "{n}");
    }
    // the home is private, like the rest of it
    let mode = std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn an_upgrade_points_the_links_at_the_new_binary_and_leaves_nothing_else_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let old = program(tmp.path(), "old-eye", "echo old");
    let new = program(tmp.path(), "new-eye", "echo new");
    let bin = tmp.path().join("bin");
    link_to(&bin, &old).unwrap();
    link_to(&bin, &new).unwrap();
    link_to(&bin, &new).unwrap();
    assert_eq!(names(&bin), ["agent-eye", "eye"]);
    for n in ["agent-eye", "eye"] {
        assert_eq!(std::fs::read_link(bin.join(n)).unwrap(), new, "{n}");
    }
}

#[test]
fn a_binary_that_is_gone_is_said_not_linked() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    let e = link_to(&bin, &tmp.path().join("missing"))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("is gone") && e.contains("restart the daemon"),
        "{e}"
    );
    assert!(!bin.exists());
}

#[test]
fn refreshing_links_the_binary_that_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    refresh(&bin).unwrap();
    assert_eq!(
        std::fs::read_link(bin.join("eye")).unwrap(),
        std::env::current_exe().unwrap()
    );
}

#[test]
fn the_path_of_a_run_starts_with_the_folder_and_keeps_the_rest() {
    let path = run_path(Path::new("/home/x/bin"));
    let text = path.to_string_lossy().into_owned();
    assert!(text.starts_with("/home/x/bin:"), "{text}");
    assert_eq!(
        text["/home/x/bin:".len()..],
        crate::platform::search_path().to_string_lossy()
    );
}

/// An `eye` from a package manager next to the agents is what a run found first, one version
/// behind the daemon; with the folder first the shell finds the link.
#[test]
fn the_shell_finds_the_link_before_an_older_eye_on_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let decoy = tmp.path().join("homebrew");
    std::fs::create_dir(&decoy).unwrap();
    program(&decoy, "eye", "echo 0.4.0");
    let exe = program(tmp.path(), "agent-eye", "echo 0.5");
    let bin = tmp.path().join("bin");
    link_to(&bin, &exe).unwrap();
    let rest = format!("{}:/usr/bin:/bin", decoy.display());
    let found = |path: OsString| {
        let o = std::process::Command::new("/bin/sh")
            .args(["-c", "command -v eye"])
            .env("PATH", path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    assert_eq!(
        found(rest.clone().into()),
        decoy.join("eye").to_string_lossy()
    );
    assert_eq!(
        found(path_before(&bin, OsStr::new(&rest))),
        bin.join("eye").to_string_lossy()
    );
}
