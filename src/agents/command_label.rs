//! A shell command as the name of what it runs, for the card's `background: ...` line. The command
//! a model writes often starts with set-up (`S=/tmp/x; cd /repo && FOO=1 exec $S/bin serve`) that
//! says nothing about the program, so the label starts at the program.

use crate::activity::one_line;

/// A path longer than this is cut to its last component when it is not the program itself.
const LONG_PATH: usize = 24;

/// The words of `cmd` up to the first `;`, `&&`, `||` or newline outside quotes, and what follows.
fn first_segment(cmd: &str) -> (&str, &str) {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let bytes: Vec<(usize, char)> = cmd.char_indices().collect();
    for (n, &(i, c)) in bytes.iter().enumerate() {
        match (quote, c) {
            _ if escaped => escaped = false,
            (_, '\\') if quote != Some('\'') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, ';' | '\n') => return (&cmd[..i], &cmd[i + 1..]),
            (None, '&' | '|') if bytes.get(n + 1).is_some_and(|&(_, d)| d == c) => {
                return (&cmd[..i], &cmd[i + 2..])
            }
            _ => {}
        }
    }
    (cmd, "")
}

/// The words of one segment; a quoted part stays inside its word.
fn words(seg: &str) -> Vec<&str> {
    let mut out = vec![];
    let (mut start, mut quote) = (None, None::<char>);
    for (i, c) in seg.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => {
                quote = Some(c);
                start.get_or_insert(i);
            }
            (None, c) if c.is_whitespace() => {
                if let Some(s) = start.take() {
                    out.push(&seg[s..i]);
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        out.push(&seg[s..]);
    }
    out
}

/// `NAME=value`, the value possibly quoted or made of `$VAR`s.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn is_path(word: &str) -> bool {
    word.starts_with(['/', '~']) || word.starts_with("./") || word.starts_with('$')
}

/// A path word as its last component: `/opt/app/bin/serve` and `$S/bin/serve` are `serve`.
fn last_component(word: &str) -> &str {
    let t = word.trim_end_matches('/');
    match t.rsplit('/').next() {
        Some(last) if is_path(word) && word.contains('/') && !last.is_empty() => last,
        _ => word,
    }
}

/// The set-up in front of a command: `cd <dir>`, and assignments alone on a segment.
fn is_setup(words: &[&str]) -> bool {
    match words.first() {
        None => true,
        Some(&"cd" | &"pushd" | &"export") => true,
        Some(_) => words.iter().all(|w| is_assignment(w)),
    }
}

/// The command from its program on, one line: leading set-up segments, `NAME=value` words and
/// `exec` are left out, and a path that is long, or is the program, is its last component. A
/// command that is only set-up keeps its own words.
pub fn of(command: &str) -> String {
    let mut rest = command;
    let program: Vec<&str> = loop {
        let (seg, after) = first_segment(rest);
        if after.trim().is_empty() || !is_setup(&words(seg)) {
            let w = words(rest);
            let skip = w
                .iter()
                .take_while(|x| is_assignment(x) || **x == "exec")
                .count();
            break w[skip..].to_vec();
        }
        rest = after;
    };
    if program.is_empty() {
        return one_line(command);
    }
    let shown: Vec<&str> = program
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if is_path(w) && (i == 0 || w.len() > LONG_PATH) {
                last_component(w)
            } else {
                w
            }
        })
        .collect();
    one_line(&shown.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_up_in_front_of_a_background_daemon_is_left_out() {
        let cmd = "S=/private/tmp/claude-501/-Users-max-code-projects-claude-eye/8f291878/scratchpad; EYE_HOME=$S/x exec $S/bin daemon --port 7421";
        assert_eq!(of(cmd), "bin daemon --port 7421");
    }

    #[test]
    fn cd_assignments_and_exec_are_dropped_and_the_program_stays() {
        assert_eq!(of("cd /repo/app && npm run dev"), "npm run dev");
        assert_eq!(
            of("cd /repo && FOO=1 BAR='a b' exec node server.js"),
            "node server.js"
        );
        assert_eq!(of("export A=1; cd ~/x; make watch"), "make watch");
        assert_eq!(
            of("A=1 B=2 python -m http.server 8000"),
            "python -m http.server 8000"
        );
    }

    #[test]
    fn a_long_path_is_its_last_component_and_a_short_argument_is_kept() {
        assert_eq!(
            of("/opt/homebrew/bin/cargo watch -x test"),
            "cargo watch -x test"
        );
        assert_eq!(
            of("tail -f /private/tmp/claude-501/some-session/tasks/b1x.output"),
            "tail -f b1x.output"
        );
        assert_eq!(of("tail -f /tmp/a.log"), "tail -f /tmp/a.log");
        assert_eq!(of("./run.sh --fast"), "run.sh --fast");
    }

    #[test]
    fn what_is_not_set_up_is_not_touched() {
        assert_eq!(of("npm run dev"), "npm run dev");
        assert_eq!(of("sleep 80"), "sleep 80");
        assert_eq!(of("echo 'a; b' && sleep 5"), "echo 'a; b' && sleep 5");
        assert_eq!(of("npm run dev | tee out.log"), "npm run dev | tee out.log");
    }

    #[test]
    fn a_command_that_is_only_set_up_keeps_its_words() {
        assert_eq!(of("cd /repo"), "cd /repo");
        assert_eq!(of("A=1"), "A=1");
        assert_eq!(of(""), "");
        assert_eq!(of("  \n "), "");
    }

    #[test]
    fn a_label_is_one_short_line() {
        let long = format!("cd /x && run {}", "word ".repeat(60));
        let l = of(&long);
        assert!(!l.contains('\n') && l.chars().count() <= crate::activity::LABEL_MAX);
        assert_eq!(of("cd /x &&\n  sleep 3"), "sleep 3");
    }
}
