//! An upgrade replaces the binary under a running daemon (`brew upgrade`, `cargo install`, a new
//! copy). The daemon must notice and run the new binary in its place, in the same process.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Stops the daemon when the test ends, however it ends.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        // SAFETY: a signal to the child this test started.
        unsafe { libc::kill(self.0.id() as i32, libc::SIGTERM) };
        let _ = self.0.wait();
    }
}

/// Wait until the daemon's log has `n` lines containing `what`; the log as it is then.
fn wait_for(log: &Path, what: &str, n: usize) -> String {
    let since = Instant::now();
    loop {
        let text = std::fs::read_to_string(log).unwrap_or_default();
        if text.matches(what).count() >= n {
            return text;
        }
        assert!(
            since.elapsed() < Duration::from_secs(30),
            "no {n} × '{what}' in the log:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A daemon started from a copy of the binary in `home/bin`, with `home` as HOME.
fn start(home: &Path, args: &[&str]) -> Daemon {
    let bin = home.join("bin/agent-eye");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_agent-eye"), &bin).unwrap();
    let child = Command::new(&bin)
        .arg("daemon")
        .args(args)
        .env("HOME", home)
        .env_remove("EYE_HOME")
        .env_remove("EYE_CLAUDE_DIR")
        .env_remove("EYE_CLAUDE_BIN")
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    Daemon(child)
}

/// The upgrade: a new file renamed over the binary, as installers do. Then the same process must
/// start again, from the new file.
fn upgrade_and_expect_a_restart(home: &Path, daemon: &mut Daemon) {
    let started = format!("daemon start pid {}", daemon.0.id());
    let log = home.join(".agent-eye/logs/daemon.log");
    wait_for(&log, &started, 1);
    let new = home.join("bin/agent-eye.new");
    std::fs::copy(env!("CARGO_BIN_EXE_agent-eye"), &new).unwrap();
    std::fs::rename(&new, home.join("bin/agent-eye")).unwrap();
    let text = wait_for(&log, &started, 2);
    assert!(text.contains("the binary was upgraded"), "{text}");
    assert!(
        daemon.0.try_wait().unwrap().is_none(),
        "the same process runs on: {text}"
    );
}

#[test]
fn a_replaced_binary_takes_over_the_running_daemon() {
    let t = tempfile::tempdir().unwrap();
    let mut daemon = start(t.path(), &["--no-board"]);
    upgrade_and_expect_a_restart(t.path(), &mut daemon);
}

#[test]
fn an_open_board_does_not_hold_the_restart_up() {
    // a browser with the board open keeps its event stream connected for good
    let t = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let home = t.path().join(".agent-eye");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        format!("[board]\nenabled = true\nport = {port}\n"),
    )
    .unwrap();
    let token = "e".repeat(64);
    std::fs::write(home.join("board.token"), &token).unwrap();
    let mut daemon = start(t.path(), &[]);
    wait_for(&home.join("logs/daemon.log"), "board at", 1);
    let mut events = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        events,
        "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nx-eye-token: {token}\r\n\r\n"
    )
    .unwrap();
    let mut head = [0u8; 12];
    events.read_exact(&mut head).unwrap();
    assert_eq!(&head, b"HTTP/1.1 200");
    upgrade_and_expect_a_restart(t.path(), &mut daemon);
}
