//! The processes of the current user: what runs, where from, since when, and on which TCP port.
//! Read only; `terminate` is the one call that acts, and only the loose-ends code makes it, after
//! a confirmed request that names the process by pid and start time.
//!
//! macOS reads `ps` and `lsof`, Linux reads `/proc`. Both are compiled everywhere; the choice is
//! made in `list_own` and `info`.

use super::MACOS;
use chrono::TimeZone;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

/// One process of the current user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    /// When it started, in Unix seconds.
    pub start: i64,
    /// The program and its arguments, as `ps` shows them.
    pub command: String,
    /// Its working folder, when it could be read.
    pub cwd: Option<PathBuf>,
    /// The TCP ports it listens on.
    pub ports: Vec<u16>,
}

/// What `info` reports about one pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub uid: u32,
    pub start: i64,
}

// ---------- parsing (shared, tested with fixed text) ----------

/// One line of `ps -axo pid=,ppid=,uid=,lstart=,command=` (lstart is `Sat Nov 21 10:11:12 2026`):
/// pid, ppid, uid, start in Unix seconds and the command.
pub fn parse_ps_line(line: &str) -> Option<(u32, u32, u32, i64, String)> {
    let mut it = line.split_whitespace();
    let pid = it.next()?.parse().ok()?;
    let ppid = it.next()?.parse().ok()?;
    let uid = it.next()?.parse().ok()?;
    let start = parse_lstart(&[it.next()?, it.next()?, it.next()?, it.next()?, it.next()?])?;
    let command: Vec<&str> = it.collect();
    // the command keeps its own spacing as far as split_whitespace allows; paths with double
    // spaces are rare and only used for display and the name filter
    Some((pid, ppid, uid, start, command.join(" ")))
}

/// `Sat Nov 21 10:11:12 2026` in local time, as Unix seconds.
pub fn parse_lstart(w: &[&str; 5]) -> Option<i64> {
    let text = w.join(" ");
    let naive = chrono::NaiveDateTime::parse_from_str(&text, "%a %b %e %H:%M:%S %Y").ok()?;
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.timestamp())
}

/// `lsof -a -d cwd -Fpn` output: pid to working folder.
pub fn parse_lsof_cwd(text: &str) -> HashMap<u32, PathBuf> {
    let mut out = HashMap::new();
    let mut pid = None;
    for l in text.lines() {
        if let Some(p) = l.strip_prefix('p') {
            pid = p.parse::<u32>().ok();
        } else if let (Some(n), Some(p)) = (l.strip_prefix('n'), pid) {
            out.entry(p).or_insert_with(|| PathBuf::from(n));
        }
    }
    out
}

/// `lsof -nP -iTCP -sTCP:LISTEN -Fpn` output: pid to its listening ports.
pub fn parse_lsof_listen(text: &str) -> HashMap<u32, Vec<u16>> {
    let mut out: HashMap<u32, Vec<u16>> = HashMap::new();
    let mut pid = None;
    for l in text.lines() {
        if let Some(p) = l.strip_prefix('p') {
            pid = p.parse::<u32>().ok();
        } else if let (Some(n), Some(p)) = (l.strip_prefix('n'), pid) {
            // `*:4321`, `127.0.0.1:4321`, `[::1]:4321`
            if let Some(port) = n.rsplit(':').next().and_then(|x| x.parse::<u16>().ok()) {
                let v = out.entry(p).or_default();
                if !v.contains(&port) {
                    v.push(port);
                }
            }
        }
    }
    for v in out.values_mut() {
        v.sort_unstable();
    }
    out
}

/// The start time of `/proc/<pid>/stat` in Unix seconds: its field 22 counts clock ticks since
/// boot. `btime` is the boot time from `/proc/stat` and `hz` the ticks per second.
pub fn linux_start(stat: &str, btime: i64, hz: i64) -> Option<i64> {
    // the command (field 2) is in parentheses and may hold spaces or `)`: count from the last one
    let rest = &stat[stat.rfind(')')? + 1..];
    let ticks: i64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    Some(btime + ticks / hz.max(1))
}

/// The parent pid out of `/proc/<pid>/stat`.
pub fn linux_ppid(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Inodes of listening IPv4/IPv6 TCP sockets out of `/proc/net/tcp`(6): inode to port.
pub fn parse_proc_net_tcp(text: &str) -> HashMap<u64, u16> {
    let mut out = HashMap::new();
    for l in text.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        // sl local rem st ... inode is field 9; st 0A is LISTEN
        if f.len() > 9 && f[3] == "0A" {
            let port = f[1]
                .rsplit(':')
                .next()
                .and_then(|p| u16::from_str_radix(p, 16).ok());
            if let (Some(port), Ok(inode)) = (port, f[9].parse::<u64>()) {
                out.insert(inode, port);
            }
        }
    }
    out
}

// ---------- reading the machine ----------

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    // lsof exits 1 when it found nothing for some of the names it was given
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn ps_path() -> &'static str {
    if std::path::Path::new("/bin/ps").exists() {
        "/bin/ps"
    } else {
        "ps"
    }
}

fn lsof_path() -> &'static str {
    if std::path::Path::new("/usr/sbin/lsof").exists() {
        "/usr/sbin/lsof"
    } else {
        "lsof"
    }
}

/// Programs that are never worth the cwd lookup: macOS system daemons and app bundles.
fn skip_for_cwd(command: &str) -> bool {
    ["/System/", "/usr/libexec/", "/usr/sbin/", "/sbin/"]
        .iter()
        .any(|p| command.starts_with(p))
        || command.contains(".app/Contents/")
}

/// The processes of the current user, with working folder and listening ports. `want` is asked
/// about each command first: one that says no is kept out before the slower lookups.
pub fn list_own(want: &dyn Fn(&str) -> bool) -> Vec<Proc> {
    if MACOS {
        list_macos(want)
    } else {
        list_linux(want)
    }
}

fn list_macos(want: &dyn Fn(&str) -> bool) -> Vec<Proc> {
    let me = super::uid();
    let Some(ps) = run(ps_path(), &["-axo", "pid=,ppid=,uid=,lstart=,command="]) else {
        return vec![];
    };
    let mut procs: Vec<Proc> = ps
        .lines()
        .filter_map(parse_ps_line)
        .filter(|(_, _, uid, _, cmd)| *uid == me && want(cmd) && !skip_for_cwd(cmd))
        .map(|(pid, ppid, _, start, command)| Proc {
            pid,
            ppid,
            start,
            command,
            cwd: None,
            ports: vec![],
        })
        .collect();
    if procs.is_empty() {
        return procs;
    }
    // the listening sockets and the working folders are asked at the same time
    let pids: Vec<String> = procs.iter().map(|p| p.pid.to_string()).collect();
    let (listen, cwds) = std::thread::scope(|sc| {
        let listen = sc.spawn(|| {
            run(
                lsof_path(),
                &[
                    "-nP",
                    "-iTCP",
                    "-sTCP:LISTEN",
                    "-Fpn",
                    "-a",
                    "-u",
                    &me.to_string(),
                ],
            )
            .map(|t| parse_lsof_listen(&t))
            .unwrap_or_default()
        });
        let cwds = run(
            lsof_path(),
            &["-a", "-d", "cwd", "-Fpn", "-p", &pids.join(",")],
        )
        .map(|t| parse_lsof_cwd(&t))
        .unwrap_or_default();
        (listen.join().unwrap_or_default(), cwds)
    });
    for p in &mut procs {
        p.cwd = cwds.get(&p.pid).cloned();
        p.ports = listen.get(&p.pid).cloned().unwrap_or_default();
    }
    procs
}

fn boot_time() -> Option<i64> {
    std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
}

fn clock_ticks() -> i64 {
    // SAFETY: sysconf has no preconditions.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if hz > 0 {
        hz as i64
    } else {
        100
    }
}

fn proc_uid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn list_linux(want: &dyn Fn(&str) -> bool) -> Vec<Proc> {
    let me = super::uid();
    let (Some(btime), hz) = (boot_time(), clock_ticks()) else {
        return vec![];
    };
    let mut sockets =
        parse_proc_net_tcp(&std::fs::read_to_string("/proc/net/tcp").unwrap_or_default());
    sockets.extend(parse_proc_net_tcp(
        &std::fs::read_to_string("/proc/net/tcp6").unwrap_or_default(),
    ));
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return vec![];
    };
    let mut out = vec![];
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        if proc_uid(pid) != Some(me) {
            continue;
        }
        let base = format!("/proc/{pid}");
        let Ok(stat) = std::fs::read_to_string(format!("{base}/stat")) else {
            continue;
        };
        let (Some(start), Some(ppid)) = (linux_start(&stat, btime, hz), linux_ppid(&stat)) else {
            continue;
        };
        let command = std::fs::read(format!("{base}/cmdline"))
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .trim_end_matches('\0')
                    .replace('\0', " ")
            })
            .unwrap_or_default();
        if command.is_empty() || !want(&command) {
            continue;
        }
        let cwd = std::fs::read_link(format!("{base}/cwd")).ok();
        let mut ports = vec![];
        if let Ok(fds) = std::fs::read_dir(format!("{base}/fd")) {
            for fd in fds.flatten() {
                let Ok(t) = std::fs::read_link(fd.path()) else {
                    continue;
                };
                let t = t.to_string_lossy();
                let inode = t
                    .strip_prefix("socket:[")
                    .and_then(|x| x.strip_suffix(']'))
                    .and_then(|x| x.parse::<u64>().ok());
                if let Some(port) = inode.and_then(|i| sockets.get(&i)) {
                    if !ports.contains(port) {
                        ports.push(*port);
                    }
                }
            }
        }
        ports.sort_unstable();
        out.push(Proc {
            pid,
            ppid,
            start,
            command,
            cwd,
            ports,
        });
    }
    out
}

/// Owner and start time of one pid, now; None when there is no such process.
pub fn info(pid: u32) -> Option<Info> {
    if MACOS {
        let t = run(ps_path(), &["-p", &pid.to_string(), "-o", "uid=,lstart="])?;
        let mut it = t.split_whitespace();
        let uid = it.next()?.parse().ok()?;
        let start = parse_lstart(&[it.next()?, it.next()?, it.next()?, it.next()?, it.next()?])?;
        Some(Info { uid, start })
    } else {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        Some(Info {
            uid: proc_uid(pid)?,
            start: linux_start(&stat, boot_time()?, clock_ticks())?,
        })
    }
}

/// Send SIGTERM to `pid`. Never SIGKILL.
pub fn terminate(pid: u32) -> std::io::Result<()> {
    let pid = libc::pid_t::try_from(pid)
        .ok()
        .filter(|p| *p > 1)
        .ok_or_else(|| std::io::Error::other("not a pid eye will signal"))?;
    // SAFETY: kill with a pid and a signal number touches no memory.
    if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ps_line_is_read() {
        let (pid, ppid, uid, start, cmd) = parse_ps_line(
            "38090     1   501 Sat Nov 21 10:11:12 2026 /opt/homebrew/bin/node /x/vite.js --port 4321",
        )
        .unwrap();
        assert_eq!((pid, ppid, uid), (38090, 1, 501));
        let local = chrono::Local.timestamp_opt(start, 0).unwrap();
        assert_eq!(
            local.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-11-21 10:11:12"
        );
        assert_eq!(cmd, "/opt/homebrew/bin/node /x/vite.js --port 4321");
        assert!(parse_ps_line("garbage").is_none());
        // a single-digit day is padded with a space by ps
        assert!(parse_ps_line("1 0 501 Fri Nov  6 09:00:00 2026 /sbin/launchd").is_some());
    }

    #[test]
    fn lsof_output_is_read() {
        let cwd = parse_lsof_cwd("p12\nfcwd\nn/a/b\np13\nfcwd\nn/c d\n");
        assert_eq!(cwd[&12], PathBuf::from("/a/b"));
        assert_eq!(cwd[&13], PathBuf::from("/c d"));
        let l = parse_lsof_listen(
            "p5\nf9\nn*:4350\nf10\nn127.0.0.1:4321\nf11\nn[::1]:4321\np6\nfx\nn*:80\n",
        );
        assert_eq!(l[&5], vec![4321, 4350]);
        assert_eq!(l[&6], vec![80]);
    }

    #[test]
    fn linux_files_are_read() {
        let stat = "123 (my (odd) cmd) S 77 1 1 0 -1 4194560 100 0 0 0 1 1 0 0 20 0 1 0 5000 1000 100 18446744073709551615";
        assert_eq!(linux_ppid(stat), Some(77));
        assert_eq!(linux_start(stat, 1_000_000, 100), Some(1_000_050));
        let tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:10E1 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0\n   1: 0100007F:9999 0100007F:10E1 01 00000000:00000000 00:00000000 00000000  1000        0 4343 1 0\n";
        let m = parse_proc_net_tcp(tcp);
        assert_eq!(m.get(&4242), Some(&4321));
        assert!(!m.contains_key(&4343));
    }

    #[test]
    fn info_reads_this_process_and_terminate_refuses_pid_one() {
        let me = info(std::process::id()).unwrap();
        assert_eq!(me.uid, super::super::uid());
        assert!((chrono::Utc::now().timestamp() - me.start) >= 0);
        assert!(terminate(0).is_err() && terminate(1).is_err());
        assert!(info(u32::MAX - 1).is_none());
    }
}
