use chrono::{DateTime, Local, TimeZone};

/// Shell-quote with single quotes: `it's` becomes `'it'\''s'`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// An argument for a shell line: kept as it is when it is made of plain characters, else
/// single-quoted. Session data (titles, ids, prompts) goes through this before it reaches a shell.
pub fn shell_word(s: &str) -> String {
    let plain = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:=@%+,-".contains(&b));
    if plain {
        s.to_string()
    } else {
        shell_quote(s)
    }
}

/// Modification time of a file in Unix ms (0 when unknown).
pub fn mtime_ms(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// Is a process with this pid running?
pub fn pid_alive(pid: i64) -> bool {
    if pid <= 0 || pid > i32::MAX as i64 {
        return false;
    }
    // SAFETY: kill with signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid as i32, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The first `name` file found on `PATH`.
pub fn find_in_path(name: &str) -> Option<std::path::PathBuf> {
    find_in(name, &std::env::var_os("PATH")?)
}

/// The first `name` file found in the folders of a `PATH`-style list.
pub fn find_in(name: &str, path: &std::ffi::OsStr) -> Option<std::path::PathBuf> {
    std::env::split_paths(path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// A random UUID (version 4) in the usual text form.
pub fn new_uuid() -> std::io::Result<String> {
    let mut b = random_bytes(16)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

/// An RFC 3339 time as Unix ms.
pub fn iso_to_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// First `max` chars.
pub fn head_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Last `max` chars.
pub fn tail_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else {
        s.chars().skip(n - max).collect()
    }
}

/// Keep at most `max` bytes from the end, cut on a char boundary.
pub fn trim_front_bytes(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut cut = s.len() - max;
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    s.drain(..cut);
}

/// Random bytes from the OS.
pub fn random_bytes(n: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf)
}

/// Lower-case hex.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Now, in Unix ms.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A Unix ms time as local `YYYY-MM-DD HH:MM`.
pub fn fmt_ts(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
        None => "?".into(),
    }
}

/// A Unix ms time as local `HH:MM`.
pub fn fmt_clock(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => d.format("%H:%M").to_string(),
        None => "?".into(),
    }
}

/// "now", "5m ago", "3h ago", "2d ago".
pub fn ago(now: i64, ms: i64) -> String {
    let s = (now - ms).max(0) / 1000;
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86400),
    }
}

/// Replace the user's home prefix with `~`.
pub fn home_rel(p: &str) -> String {
    if let Some(h) = std::env::var_os("HOME") {
        let h = h.to_string_lossy();
        if !h.is_empty() && (p == h || p.starts_with(&format!("{h}/"))) {
            return format!("~{}", &p[h.len()..]);
        }
    }
    p.to_string()
}

/// A path for a message: under the user's home it starts with `~`.
pub fn show_path(p: &std::path::Path) -> String {
    home_rel(&p.to_string_lossy())
}

/// A session id that is safe to put in a command: letters, digits, `-`, `_` and `.` only, and not
/// starting with a dash or a dot (so it is never read as an option or a path).
pub fn valid_session_id(id: &str) -> bool {
    (4..=128).contains(&id.len())
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Text for a terminal: control characters (escape sequences, NUL, bell, the C1 range) become
/// spaces, so session text cannot move the cursor, retitle the window or write the clipboard. New
/// lines and tabs stay.
pub fn printable(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// The first 8 characters of a session id, as people see it. The whole id when it is shorter or
/// has a multi-byte character in that stretch (ids are file names, so they can).
pub fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// 2300000 as "2.3M", 410000 as "410k", 950 as "950".
pub fn fmt_tokens(n: i64) -> String {
    let n = n.max(0);
    let one = |v: f64, unit: &str| {
        let t = format!("{v:.1}");
        format!("{}{unit}", t.strip_suffix(".0").unwrap_or(&t))
    };
    match n {
        0..=999 => n.to_string(),
        1_000..=99_999 => one(n as f64 / 1e3, "k"),
        100_000..=999_499 => format!("{}k", (n + 500) / 1000),
        _ => one(n as f64 / 1e6, "M"),
    }
}

/// "2.3M (sub-agents 410k)": everything counted, with the sub-agent share when there is one.
pub fn tokens_line(t: &crate::model::Tokens) -> String {
    let mut l = fmt_tokens(t.total());
    if t.subagents_total > 0 {
        l.push_str(&format!(" (sub-agents {})", fmt_tokens(t.subagents_total)));
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable_text_has_no_terminal_controls() {
        assert_eq!(printable("a\x1b[31mb\u{9b}c\x07"), "a [31mb c ");
        assert_eq!(printable("two\nlines\tand é 🙂"), "two\nlines\tand é 🙂");
    }

    #[test]
    fn short_ids_never_split_a_character() {
        assert_eq!(short_id("a0000001-0000-4000"), "a0000001");
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id("abcdefg\u{e9}h"), "abcdefg\u{e9}h");
    }

    #[test]
    fn quotes() {
        assert_eq!(shell_quote("/a b/it's"), "'/a b/it'\\''s'");
    }
    #[test]
    fn shell_words() {
        assert_eq!(shell_word("--resume"), "--resume");
        assert_eq!(
            shell_word("30194328-aaaa-4bbb-8ccc"),
            "30194328-aaaa-4bbb-8ccc"
        );
        assert_eq!(shell_word("a; touch /tmp/pwn"), "'a; touch /tmp/pwn'");
        assert_eq!(shell_word("it's"), "'it'\\''s'");
        assert_eq!(shell_word(""), "''");
        assert_eq!(shell_word("$(id)"), "'$(id)'");
        assert_eq!(shell_word("~"), "'~'");
    }
    #[test]
    fn uuids_look_right() {
        let u = new_uuid().unwrap();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[14], b'4');
        assert_ne!(u, new_uuid().unwrap());
        assert!(valid_session_id(&u));
    }
    #[test]
    fn own_pid_alive_and_bogus_dead() {
        assert!(pid_alive(std::process::id() as i64));
        assert!(!pid_alive(0));
        assert!(!pid_alive(i32::MAX as i64 - 1));
    }
    #[test]
    fn token_format() {
        assert_eq!(fmt_tokens(950), "950");
        assert_eq!(fmt_tokens(1_000), "1k");
        assert_eq!(fmt_tokens(12_340), "12.3k");
        assert_eq!(fmt_tokens(410_000), "410k");
        assert_eq!(fmt_tokens(2_300_000), "2.3M");
        assert_eq!(fmt_tokens(999_600), "1M");
        assert_eq!(fmt_tokens(0), "0");
        let t = crate::model::Tokens {
            input: 1_000_000,
            output: 500_000,
            cache_write: 300_000,
            cache_read: 100_000,
            subagents_total: 410_000,
        };
        assert_eq!(tokens_line(&t), "2.3M (sub-agents 410k)");
        assert_eq!(tokens_line(&crate::model::Tokens::default()), "0");
    }
    #[test]
    fn session_ids() {
        assert!(valid_session_id("30194328-aaaa-4bbb-8ccc-0123456789ab"));
        assert!(!valid_session_id("abc; rm -rf ~"));
        assert!(!valid_session_id("$(id)aaaaaaaa"));
        assert!(!valid_session_id("--dangerously-skip-permissions"));
        assert!(!valid_session_id("-rf-everything"));
        assert!(!valid_session_id(".hidden-id"));
        assert!(!valid_session_id("../../etc"));
        assert!(!valid_session_id("id\nwith-newline"));
        assert!(valid_session_id("fake-session-0001"));
    }
    #[test]
    fn trims() {
        let mut s = "aé€bcd".to_string();
        trim_front_bytes(&mut s, 4);
        assert!(s.ends_with("bcd"));
        assert_eq!(tail_chars("abcdef", 2), "ef");
    }
}
