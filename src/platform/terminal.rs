//! Where a resume command goes, and how a terminal window is opened for it.
//!
//! `[board] resume` is a wish (`auto`, `iterm`, `terminal`, `copy`); a [`Target`] is what that wish
//! comes to on this machine, worked out once per request, so the label the board shows (ITERM,
//! TERMINAL, COPY) is always what really happens.
//!
//! - macOS, `terminal`: the command goes into a `.command` file under eye's `tmp/` and `open` runs
//!   it, which macOS hands to the app for `.command` files (Terminal.app unless it was changed).
//! - Linux, `terminal`: the first of `xdg-terminal-exec`, `x-terminal-emulator` and `$TERMINAL`
//!   runs `sh -c <command>` in a new window.
//! - `auto` picks iTerm2 on macOS when it is installed, else the terminal above; on Linux it picks
//!   the terminal only when there is a display, and copies when nothing applies.

use super::{search_path, MACOS};
use crate::util::find_in;
use anyhow::{Context, Result};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// What happens to a resume command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A tab in iTerm2 (macOS).
    Iterm,
    /// A new window of the system's terminal.
    Terminal,
    /// The command goes to the clipboard.
    Copy,
}

impl Mode {
    /// The name used in the API (`iterm`, `terminal` or `copy`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Iterm => "iterm",
            Mode::Terminal => "terminal",
            Mode::Copy => "copy",
        }
    }
}

/// How a command will be delivered, with what it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum How {
    Copy,
    Iterm,
    /// macOS: a `.command` file in `dir`, opened with the `open` program.
    CommandFile {
        open: PathBuf,
        dir: PathBuf,
    },
    /// Linux: `exe` `args` then `sh -c <command>`.
    Program {
        exe: PathBuf,
        args: &'static [&'static str],
        name: String,
    },
}

/// The outcome of resolving `[board] resume` on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    how: How,
}

/// What a machine looks like to the resolution: kept apart from the real machine so tests can
/// stand in a Mac or a Linux box.
#[derive(Debug, Clone)]
pub struct Env {
    pub macos: bool,
    /// Where programs are looked up (`$PATH`).
    pub path: OsString,
    /// Where applications live on a Mac (`/Applications`, `~/Applications`).
    pub app_dirs: Vec<PathBuf>,
    /// Is there a display a terminal can open on (`DISPLAY` or `WAYLAND_DISPLAY`)?
    pub display: bool,
    /// `$TERMINAL`, when set.
    pub terminal_var: Option<OsString>,
    /// Where `.command` files are written (`<eye home>/tmp`).
    pub tmp: PathBuf,
}

impl Env {
    /// This machine, with `tmp` as the folder for `.command` files.
    pub fn current(tmp: PathBuf) -> Env {
        let mut app_dirs = vec![PathBuf::from("/Applications")];
        if let Some(home) = super::account_home() {
            app_dirs.push(home.join("Applications"));
        }
        Env {
            macos: MACOS,
            path: search_path(),
            app_dirs,
            display: has_display(
                std::env::var_os("DISPLAY").as_deref(),
                std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            ),
            terminal_var: std::env::var_os("TERMINAL"),
            tmp,
        }
    }

    fn has_iterm(&self) -> bool {
        self.app_dirs.iter().any(|d| d.join("iTerm.app").is_dir())
    }

    fn mac_terminal(&self) -> Target {
        match find_in("open", &self.path) {
            Some(open) => Target::new(How::CommandFile {
                open,
                dir: self.tmp.clone(),
            }),
            None => Target::copy(),
        }
    }

    /// The first terminal program that is there. `auto` asks for a display as well.
    fn linux_terminal(&self, auto: bool) -> Target {
        if auto && !self.display {
            return Target::copy();
        }
        if let Some(exe) = find_in("xdg-terminal-exec", &self.path) {
            return Target::program(exe, &[], "xdg-terminal-exec".into());
        }
        if let Some(exe) = find_in("x-terminal-emulator", &self.path) {
            let name = alternative_name(&exe, "x-terminal-emulator");
            return Target::program(exe, &["-e"], name);
        }
        if let Some(exe) = self.terminal_from_var() {
            let name = exe
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            return Target::program(exe, &["-e"], name);
        }
        Target::copy()
    }

    /// `$TERMINAL` as a program: a path, or a name looked up on the path.
    fn terminal_from_var(&self) -> Option<PathBuf> {
        let var = self.terminal_var.as_deref().filter(|v| !v.is_empty())?;
        if var.to_string_lossy().contains('/') {
            Some(PathBuf::from(var)).filter(|p| p.is_file())
        } else {
            find_in(&var.to_string_lossy(), &self.path)
        }
    }
}

/// Is a display set? An empty value does not count.
fn has_display(display: Option<&OsStr>, wayland: Option<&OsStr>) -> bool {
    [display, wayland]
        .iter()
        .any(|v| v.is_some_and(|v| !v.is_empty()))
}

/// What a `x-terminal-emulator`-style link points at, e.g. `gnome-terminal` for Debian's
/// `/usr/bin/x-terminal-emulator`; `fallback` when it points nowhere useful.
fn alternative_name(exe: &Path, fallback: &str) -> String {
    let real = std::fs::canonicalize(exe).ok();
    let name = real
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = name.strip_suffix(".wrapper").unwrap_or(&name);
    if name.is_empty() {
        fallback.into()
    } else {
        name.into()
    }
}

impl Target {
    fn new(how: How) -> Target {
        Target { how }
    }
    fn program(exe: PathBuf, args: &'static [&'static str], name: String) -> Target {
        Target::new(How::Program { exe, args, name })
    }

    /// The command goes to the clipboard.
    pub fn copy() -> Target {
        Target::new(How::Copy)
    }

    /// What `setting` (`[board] resume`) comes to on the machine `env` describes. `copy` always
    /// copies; `iterm` is an iTerm2 tab on a Mac and a copy elsewhere; `terminal` opens the system's
    /// terminal when there is one; anything else is `auto`.
    pub fn resolve(setting: &str, env: &Env) -> Target {
        match setting.trim().to_ascii_lowercase().as_str() {
            "copy" => Target::copy(),
            "iterm" if env.macos => Target::new(How::Iterm),
            "iterm" => Target::copy(),
            "terminal" if env.macos => env.mac_terminal(),
            "terminal" => env.linux_terminal(false),
            _ if env.macos && env.has_iterm() => Target::new(How::Iterm),
            _ if env.macos => env.mac_terminal(),
            _ => env.linux_terminal(true),
        }
    }

    pub fn mode(&self) -> Mode {
        match self.how {
            How::Copy => Mode::Copy,
            How::Iterm => Mode::Iterm,
            How::CommandFile { .. } | How::Program { .. } => Mode::Terminal,
        }
    }

    /// The name of the terminal program on Linux, when it is known.
    pub fn terminal(&self) -> Option<&str> {
        match &self.how {
            How::Program { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Open a new terminal window that runs `command`; a one-line note on what was done.
    pub fn open_terminal(&self, command: &str) -> Result<String> {
        match &self.how {
            How::CommandFile { open, dir } => {
                let now = SystemTime::now();
                prune(dir, now);
                let file = write_command_file(dir, command, now)?;
                let out = Command::new(open)
                    .arg(&file)
                    .stdin(Stdio::null())
                    .output()
                    .context("running open");
                let failed = match &out {
                    Ok(o) if o.status.success() => None,
                    Ok(o) => Some(format!(
                        "open failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    )),
                    Err(e) => Some(format!("{e:#}")),
                };
                if let Some(why) = failed {
                    let _ = std::fs::remove_file(&file);
                    anyhow::bail!("the terminal did not open: {why}");
                }
                Ok("opened in a new terminal window".into())
            }
            How::Program { exe, args, name } => {
                run_terminal(exe, args, command, name, SETTLE)?;
                Ok(format!("opened in a new terminal ({name})"))
            }
            How::Copy | How::Iterm => anyhow::bail!("this target does not open a terminal"),
        }
    }
}

/// How long a terminal program is watched for failing to start (no display, bad arguments). One
/// that is still running after this has a window up; one that came back with success has handed
/// the window to a server it talks to.
const SETTLE: Duration = Duration::from_millis(300);

/// Run `exe args... sh -c command`, detached: no standard streams, and not waited for beyond
/// `settle`. An exit with failure in that time is an error.
fn run_terminal(
    exe: &Path,
    args: &[&str],
    command: &str,
    name: &str,
    settle: Duration,
) -> Result<()> {
    let mut child = Command::new(exe)
        .args(args)
        .args(["sh", "-c", command])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("running {name}"))?;
    let until = std::time::Instant::now() + settle;
    loop {
        if let Some(status) = child.try_wait().context("waiting for the terminal")? {
            anyhow::ensure!(
                status.success(),
                "the terminal did not open: {name} exited with {status}"
            );
            return Ok(());
        }
        if std::time::Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // The window is up. Reap the program when it ends, so a long-running daemon keeps no zombies.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// `.command` files are named so that eye only ever removes its own.
const FILE_PREFIX: &str = "resume-";
const FILE_SUFFIX: &str = ".command";
/// A `.command` file is needed for the moment Terminal reads it; older ones are removed.
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

/// Write a `.command` file for `command` in `dir` (made if missing, owner only) and return its
/// path. The file is executable by its owner alone; control characters are dropped from the line
/// as they are for iTerm.
fn write_command_file(dir: &Path, command: &str, now: SystemTime) -> Result<PathBuf> {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    let ms = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!(
        "{FILE_PREFIX}{ms}-{}-{}{FILE_SUFFIX}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    );
    let path = dir.join(name);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&path)
        .with_context(|| format!("writing {}", path.display()))?;
    f.write_all(format!("#!/bin/sh\n{}\n", crate::resume::typed(command)).as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Remove eye's `.command` files in `dir` that are older than a day. Anything else is left alone,
/// and failures are ignored: this tidies, it never blocks a resume.
fn prune(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(FILE_PREFIX) && name.ends_with(FILE_SUFFIX)) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_support::{script, stub};
    use std::os::unix::fs::PermissionsExt;

    /// A machine in a temp folder: `bin` is its whole `PATH`, `apps` its application folder.
    struct Machine {
        _t: tempfile::TempDir,
        bin: PathBuf,
        apps: PathBuf,
        home_apps: PathBuf,
        tmp: PathBuf,
        macos: bool,
        display: bool,
        terminal_var: Option<OsString>,
    }

    impl Machine {
        fn new(macos: bool) -> Machine {
            let t = tempfile::tempdir().unwrap();
            let bin = t.path().join("bin");
            let apps = t.path().join("Applications");
            let home_apps = t.path().join("home/Applications");
            for d in [&bin, &apps, &home_apps] {
                std::fs::create_dir_all(d).unwrap();
            }
            Machine {
                tmp: t.path().join("eye/tmp"),
                _t: t,
                bin,
                apps,
                home_apps,
                macos,
                display: true,
                terminal_var: None,
            }
        }
        fn env(&self) -> Env {
            Env {
                macos: self.macos,
                path: self.bin.clone().into_os_string(),
                app_dirs: vec![self.apps.clone(), self.home_apps.clone()],
                display: self.display,
                terminal_var: self.terminal_var.clone(),
                tmp: self.tmp.clone(),
            }
        }
        fn resolve(&self, setting: &str) -> Target {
            Target::resolve(setting, &self.env())
        }
        fn iterm_in(&self, dir: &Path) {
            std::fs::create_dir_all(dir.join("iTerm.app/Contents")).unwrap();
        }
        /// A terminal program that records its arguments, one per line, in `<name>.args`.
        fn terminal(&self, name: &str) {
            script(
                &self.bin,
                name,
                &format!("printf '%s\\n' \"$@\" > \"$(dirname \"$0\")/{name}.args\""),
            );
        }
        fn args(&self, name: &str) -> Vec<String> {
            std::fs::read_to_string(self.bin.join(format!("{name}.args")))
                .unwrap()
                .lines()
                .map(String::from)
                .collect()
        }
    }

    fn what(t: &Target) -> (Mode, Option<&str>) {
        (t.mode(), t.terminal())
    }

    #[test]
    fn the_modes_have_their_api_names() {
        assert_eq!(Mode::Iterm.as_str(), "iterm");
        assert_eq!(Mode::Terminal.as_str(), "terminal");
        assert_eq!(Mode::Copy.as_str(), "copy");
    }

    #[test]
    fn a_mac_with_iterm_gets_an_iterm_tab_from_auto() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        b.iterm_in(&b.apps);
        for setting in ["auto", "AUTO", " auto ", "", "whatever"] {
            assert_eq!(
                what(&b.resolve(setting)),
                (Mode::Iterm, None),
                "{setting:?}"
            );
        }
    }

    #[test]
    fn iterm_in_the_users_own_applications_folder_counts() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        assert_eq!(b.resolve("auto").mode(), Mode::Terminal);
        b.iterm_in(&b.home_apps);
        assert_eq!(b.resolve("auto").mode(), Mode::Iterm);
    }

    #[test]
    fn a_mac_without_iterm_gets_the_terminal_from_auto() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        let t = b.resolve("auto");
        assert_eq!(what(&t), (Mode::Terminal, None));
        assert!(matches!(t.how, How::CommandFile { .. }));
        // an iTerm2 that is only a plain file is not an app
        std::fs::write(b.apps.join("iTerm.app"), "").unwrap();
        assert_eq!(b.resolve("auto").mode(), Mode::Terminal);
    }

    #[test]
    fn a_mac_that_cannot_open_a_file_copies() {
        let b = Machine::new(true);
        for setting in ["auto", "terminal"] {
            assert_eq!(b.resolve(setting), Target::copy(), "{setting}");
        }
    }

    #[test]
    fn on_a_mac_the_explicit_settings_are_taken_at_their_word() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        b.iterm_in(&b.apps);
        assert_eq!(
            b.resolve("terminal").mode(),
            Mode::Terminal,
            "even with iTerm2"
        );
        assert_eq!(b.resolve("Terminal").mode(), Mode::Terminal);
        assert_eq!(b.resolve("copy").mode(), Mode::Copy);
        assert_eq!(b.resolve("COPY").mode(), Mode::Copy);
        // `iterm` is not looked up: an iTerm2 outside the usual folders still works
        let b = Machine::new(true);
        assert_eq!(b.resolve("iterm").mode(), Mode::Iterm);
        assert_eq!(b.resolve("ITERM").mode(), Mode::Iterm);
    }

    #[test]
    fn linux_auto_needs_a_display() {
        let mut b = Machine::new(false);
        b.terminal("xdg-terminal-exec");
        b.display = false;
        assert_eq!(b.resolve("auto"), Target::copy());
        b.display = true;
        assert_eq!(
            what(&b.resolve("auto")),
            (Mode::Terminal, Some("xdg-terminal-exec"))
        );
    }

    #[test]
    fn a_display_is_a_non_empty_display_or_wayland_display() {
        let v = |s: &'static str| Some(OsStr::new(s));
        assert!(has_display(v(":0"), None));
        assert!(has_display(None, v("wayland-0")));
        assert!(has_display(v(""), v("wayland-1")));
        assert!(!has_display(None, None));
        assert!(!has_display(v(""), v("")));
    }

    #[test]
    fn linux_tries_the_terminals_in_the_documented_order() {
        let mut b = Machine::new(false);
        b.terminal("xdg-terminal-exec");
        b.terminal("x-terminal-emulator");
        b.terminal("kitty");
        b.terminal_var = Some("kitty".into());
        assert_eq!(b.resolve("auto").terminal(), Some("xdg-terminal-exec"));
        std::fs::remove_file(b.bin.join("xdg-terminal-exec")).unwrap();
        assert_eq!(b.resolve("auto").terminal(), Some("x-terminal-emulator"));
        std::fs::remove_file(b.bin.join("x-terminal-emulator")).unwrap();
        assert_eq!(b.resolve("auto").terminal(), Some("kitty"));
        std::fs::remove_file(b.bin.join("kitty")).unwrap();
        assert_eq!(b.resolve("auto"), Target::copy(), "nothing to open");
        assert_eq!(b.resolve("terminal"), Target::copy());
    }

    #[test]
    fn x_terminal_emulator_is_named_after_what_it_points_at() {
        let b = Machine::new(false);
        b.terminal("gnome-terminal.wrapper");
        std::os::unix::fs::symlink(
            b.bin.join("gnome-terminal.wrapper"),
            b.bin.join("x-terminal-emulator"),
        )
        .unwrap();
        assert_eq!(b.resolve("auto").terminal(), Some("gnome-terminal"));
        // a link that goes nowhere keeps the name it was found under
        assert_eq!(
            alternative_name(Path::new("/nonexistent/x-terminal-emulator"), "fallback"),
            "fallback"
        );
    }

    #[test]
    fn terminal_from_the_environment_is_a_name_or_a_path() {
        let mut b = Machine::new(false);
        b.terminal("foot");
        b.terminal_var = Some("foot".into());
        assert_eq!(b.resolve("auto").terminal(), Some("foot"));
        b.terminal_var = Some(b.bin.join("foot").into_os_string());
        assert_eq!(b.resolve("auto").terminal(), Some("foot"));
        for bad in ["", "missing", "/nonexistent/foot"] {
            b.terminal_var = Some(bad.into());
            assert_eq!(b.resolve("auto"), Target::copy(), "{bad:?}");
        }
        b.terminal_var = None;
        assert_eq!(b.resolve("auto"), Target::copy());
    }

    #[test]
    fn on_linux_the_terminal_setting_does_not_ask_for_a_display() {
        let mut b = Machine::new(false);
        b.terminal("x-terminal-emulator");
        b.display = false;
        assert_eq!(b.resolve("auto"), Target::copy());
        assert_eq!(b.resolve("terminal").mode(), Mode::Terminal);
    }

    #[test]
    fn on_linux_iterm_and_copy_copy() {
        let b = Machine::new(false);
        b.terminal("xdg-terminal-exec");
        assert_eq!(b.resolve("iterm"), Target::copy());
        assert_eq!(b.resolve("ITERM"), Target::copy());
        assert_eq!(b.resolve("copy"), Target::copy());
    }

    #[test]
    fn linux_terminals_get_the_command_as_one_argument_after_sh_c() {
        let b = Machine::new(false);
        b.terminal("xdg-terminal-exec");
        let cmd = "cd '/a b' && claude --resume 'x; touch /tmp/pwn'";
        let note = b.resolve("auto").open_terminal(cmd).unwrap();
        assert_eq!(note, "opened in a new terminal (xdg-terminal-exec)");
        assert_eq!(b.args("xdg-terminal-exec"), ["sh", "-c", cmd]);

        std::fs::remove_file(b.bin.join("xdg-terminal-exec")).unwrap();
        b.terminal("x-terminal-emulator");
        let nasty = "cd 'it'\\''s' && echo \"$(id)\" `id` & ; é";
        b.resolve("auto").open_terminal(nasty).unwrap();
        assert_eq!(b.args("x-terminal-emulator"), ["-e", "sh", "-c", nasty]);

        let mut b = Machine::new(false);
        b.terminal("foot");
        b.terminal_var = Some("foot".into());
        b.resolve("auto").open_terminal("claude").unwrap();
        assert_eq!(b.args("foot"), ["-e", "sh", "-c", "claude"]);
    }

    #[test]
    fn a_terminal_that_fails_to_start_is_an_error() {
        let b = Machine::new(false);
        stub(&b.bin, "xdg-terminal-exec", 1);
        let e = b.resolve("auto").open_terminal("claude").unwrap_err();
        let e = format!("{e:#}");
        assert!(
            e.contains("did not open") && e.contains("xdg-terminal-exec"),
            "{e}"
        );
    }

    #[test]
    fn a_terminal_that_cannot_run_is_an_error() {
        let e = run_terminal(
            Path::new("/nonexistent/term"),
            &[],
            "claude",
            "term",
            Duration::from_millis(50),
        )
        .unwrap_err();
        assert!(format!("{e:#}").contains("running term"), "{e:#}");
    }

    #[test]
    fn a_terminal_that_stays_up_has_opened() {
        let b = Machine::new(false);
        script(&b.bin, "term", "exec sleep 1");
        let start = std::time::Instant::now();
        run_terminal(
            &b.bin.join("term"),
            &["-e"],
            "claude",
            "term",
            Duration::from_millis(100),
        )
        .unwrap();
        assert!(
            start.elapsed() < Duration::from_millis(900),
            "did not wait for it to end"
        );
    }

    #[test]
    fn a_mac_terminal_runs_a_command_file_through_open() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        let cmd = "cd '/a b' && claude --resume abc";
        let note = b.resolve("terminal").open_terminal(cmd).unwrap();
        assert_eq!(note, "opened in a new terminal window");
        let opened = std::fs::read_to_string(b.bin.join("open.log")).unwrap();
        let file = PathBuf::from(opened.trim());
        assert_eq!(file.parent(), Some(b.tmp.as_path()));
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("resume-") && name.ends_with(".command"),
            "{name}"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "#!/bin/sh\ncd '/a b' && claude --resume abc\n"
        );
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o700);
        assert_eq!(mode(&b.tmp), 0o700);
    }

    #[test]
    fn the_command_file_runs_what_a_terminal_would_type() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        let workdir = b.apps.join("w d");
        std::fs::create_dir_all(&workdir).unwrap();
        let line = format!(
            "cd {} && printf %s \"$PWD\" > out.txt",
            crate::util::shell_quote(&workdir.to_string_lossy())
        );
        b.resolve("terminal").open_terminal(&line).unwrap();
        let opened = std::fs::read_to_string(b.bin.join("open.log")).unwrap();
        let status = Command::new("sh").arg(opened.trim()).status().unwrap();
        assert!(status.success());
        let got = std::fs::read_to_string(workdir.join("out.txt")).unwrap();
        assert_eq!(
            std::fs::canonicalize(got).unwrap(),
            std::fs::canonicalize(&workdir).unwrap()
        );
    }

    #[test]
    fn control_characters_stay_out_of_the_command_file() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        b.resolve("terminal")
            .open_terminal("cd '/a\u{1b}[2Jb\u{3}' && claude")
            .unwrap();
        let opened = std::fs::read_to_string(b.bin.join("open.log")).unwrap();
        let text = std::fs::read_to_string(opened.trim()).unwrap();
        assert_eq!(text, "#!/bin/sh\ncd '/a[2Jb' && claude\n");
    }

    #[test]
    fn a_failed_open_says_so_and_leaves_no_file() {
        let b = Machine::new(true);
        script(
            &b.bin,
            "open",
            "echo 'no application knows how to open it' >&2\nexit 1",
        );
        let e = b.resolve("terminal").open_terminal("claude").unwrap_err();
        let e = format!("{e:#}");
        assert!(
            e.contains("did not open") && e.contains("no application knows how"),
            "{e}"
        );
        let left: Vec<_> = std::fs::read_dir(&b.tmp).unwrap().flatten().collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn two_files_in_the_same_millisecond_do_not_clash() {
        let t = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let a = write_command_file(t.path(), "a", now).unwrap();
        let b = write_command_file(t.path(), "b", now).unwrap();
        assert_ne!(a, b);
    }

    fn age(p: &Path, by: Duration) {
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - by).unwrap();
    }

    #[test]
    fn old_command_files_are_removed_on_the_next_use() {
        let b = Machine::new(true);
        stub(&b.bin, "open", 0);
        std::fs::create_dir_all(&b.tmp).unwrap();
        let day = Duration::from_secs(24 * 60 * 60);
        let make = |name: &str, by: Duration| {
            let p = b.tmp.join(name);
            std::fs::write(&p, "#!/bin/sh\ntrue\n").unwrap();
            age(&p, by);
            p
        };
        let old = make("resume-1-1-0.command", day * 2);
        let old2 = make("resume-2-1-0.command", day + Duration::from_secs(60));
        let fresh = make("resume-3-1-0.command", Duration::from_secs(3600));
        let almost = make("resume-4-1-0.command", day - Duration::from_secs(60));
        // other files are not eye's to remove, however old
        let other = make("notes.command", day * 30);
        let text = make("resume-5-1-0.txt", day * 30);
        b.resolve("terminal").open_terminal("claude").unwrap();
        assert!(!old.exists() && !old2.exists(), "files over a day old go");
        assert!(fresh.exists() && almost.exists(), "recent ones stay");
        assert!(other.exists() && text.exists(), "only eye's own files go");
        // and the file just written is there for Terminal to read
        let opened = std::fs::read_to_string(b.bin.join("open.log")).unwrap();
        assert!(Path::new(opened.trim()).exists());
    }

    #[test]
    fn a_missing_tmp_folder_is_no_trouble_for_the_cleanup() {
        prune(Path::new("/nonexistent/eye/tmp"), SystemTime::now());
    }

    #[test]
    fn copy_and_iterm_targets_open_no_terminal() {
        assert!(Target::copy().open_terminal("x").is_err());
        let b = Machine::new(true);
        assert!(b.resolve("iterm").open_terminal("x").is_err());
    }
}
