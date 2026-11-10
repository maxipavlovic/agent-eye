//! Codex's code mode runs tools from a JavaScript program: the model calls `exec` with source such
//! as `const r = await tools.exec_command({cmd: "ls"}); text(r.output);`. A paginated rollout also
//! records what the program ran as items, so this is for a rollout that does not. It reads only what
//! is written out as string literals: `tools.exec_command({cmd: "..."})` and any literal holding an
//! `apply_patch` text. A command built from variables or a template cannot be told without running
//! the program, so it is not guessed.

use super::rollout::PATCH_START;

/// A tool call found in a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// The `cmd` of `tools.exec_command`.
    Command(String),
    /// A string that is an `apply_patch` text.
    Patch(String),
}

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
}

/// The calls a program makes in the order they are written; none when its source cannot be read.
pub fn calls(js: &str) -> Vec<Call> {
    let Some(toks) = lex(js) else {
        return vec![];
    };
    let mut out = vec![];
    let mut i = 0;
    while i < toks.len() {
        match &toks[i..] {
            [Tok::Ident(t), Tok::Punct('.'), Tok::Ident(f), Tok::Punct('('), Tok::Punct('{'), ..]
                if t == "tools" && f == "exec_command" =>
            {
                let (cmd, used) = object_cmd(&toks[i + 5..]);
                out.extend(cmd.map(Call::Command));
                i += 5 + used;
            }
            [Tok::Str(s), ..] if s.contains(PATCH_START) => {
                out.push(Call::Patch(s.clone()));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// The `cmd` of the object literal whose tokens start just after its `{`, when it is one plain
/// string, and how many tokens the object takes.
fn object_cmd(toks: &[Tok]) -> (Option<String>, usize) {
    let mut cmd = None;
    let mut i = 0;
    while let Some(t) = toks.get(i) {
        i += 1;
        match t {
            Tok::Punct('}') => break,
            Tok::Ident(key) | Tok::Str(key) if toks.get(i) == Some(&Tok::Punct(':')) => {
                let start = i + 1;
                i = start;
                let mut depth = 0;
                while let Some(t) = toks.get(i) {
                    match t {
                        Tok::Punct('(' | '[' | '{') => depth += 1,
                        Tok::Punct(')' | ']') => depth -= 1,
                        Tok::Punct('}') if depth == 0 => break,
                        Tok::Punct('}') => depth -= 1,
                        Tok::Punct(',') if depth == 0 => break,
                        _ => {}
                    }
                    i += 1;
                }
                if let (true, [Tok::Str(s)]) = (key == "cmd", &toks[start..i]) {
                    cmd = Some(s.clone());
                }
            }
            _ => {}
        }
    }
    (cmd, i)
}

/// The tokens of a program: names, string literals and single characters. None when a string is
/// unfinished or is a template with `${}` in it.
fn lex(js: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = js.chars().collect();
    let word = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '$';
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            ch if ch.is_whitespace() => i += 1,
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            }
            '"' | '\'' | '`' => {
                let (s, next) = string(&c, i)?;
                out.push(Tok::Str(s));
                i = next;
            }
            ch if word(ch) => {
                let start = i;
                while i < c.len() && word(c[i]) {
                    i += 1;
                }
                out.push(Tok::Ident(c[start..i].iter().collect()));
            }
            ch => {
                out.push(Tok::Punct(ch));
                i += 1;
            }
        }
    }
    Some(out)
}

/// The string literal that starts at `c[start]` (its quote), unescaped, and where it ends.
fn string(c: &[char], start: usize) -> Option<(String, usize)> {
    let quote = c[start];
    let mut s = String::new();
    let mut i = start + 1;
    while let Some(&ch) = c.get(i) {
        i += 1;
        match ch {
            q if q == quote => return Some((s, i)),
            '\\' => i = escape(c, i, &mut s)?,
            '$' if quote == '`' && c.get(i) == Some(&'{') => return None,
            '\n' if quote != '`' => return None,
            ch => s.push(ch),
        }
    }
    None
}

/// Add what the escape whose first character is `c[at]` (after the backslash) stands for to `out`;
/// returns where the escape ends.
fn escape(c: &[char], at: usize, out: &mut String) -> Option<usize> {
    let hex = |from: usize, len: usize| {
        let digits: String = c.get(from..from + len)?.iter().collect();
        u32::from_str_radix(&digits, 16).ok()
    };
    let (ch, next) = match *c.get(at)? {
        'n' => (Some('\n'), at + 1),
        't' => (Some('\t'), at + 1),
        'r' => (Some('\r'), at + 1),
        'b' => (Some('\u{8}'), at + 1),
        'f' => (Some('\u{c}'), at + 1),
        'v' => (Some('\u{b}'), at + 1),
        '0' => (Some('\0'), at + 1),
        '\n' => (None, at + 1),
        'x' => (char::from_u32(hex(at + 1, 2)?), at + 3),
        'u' if c.get(at + 1) == Some(&'{') => {
            let end = at + 2 + c.get(at + 2..)?.iter().position(|&x| x == '}')?;
            let digits: String = c[at + 2..end].iter().collect();
            (
                char::from_u32(u32::from_str_radix(&digits, 16).ok()?),
                end + 1,
            )
        }
        'u' => {
            let high = hex(at + 1, 4)?;
            let low = (c.get(at + 5) == Some(&'\\') && c.get(at + 6) == Some(&'u'))
                .then(|| hex(at + 7, 4))
                .flatten();
            match low {
                // a pair of surrogates is one character
                Some(low)
                    if (0xd800..0xdc00).contains(&high) && (0xdc00..0xe000).contains(&low) =>
                {
                    (
                        char::from_u32(0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)),
                        at + 11,
                    )
                }
                _ => (char::from_u32(high), at + 5),
            }
        }
        other => (Some(other), at + 1),
    };
    match ch {
        Some(ch) => out.push(ch),
        // a line continuation adds nothing; a number that is no character adds the replacement one
        None if c[at] == '\n' => {}
        None => out.push('\u{fffd}'),
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(c: &str) -> Call {
        Call::Command(c.into())
    }

    #[test]
    fn the_programs_codex_writes_are_read() {
        assert_eq!(
            calls("const r = await tools.exec_command({cmd:\"sed -n '1,200p' calc.py\",max_output_tokens:2000});\ntext(r.output);\n"),
            [command("sed -n '1,200p' calc.py")]
        );
        assert_eq!(
            calls(
                r#"const r = await tools.exec_command({cmd:"rg --files -g 'calc.py'","max_output_tokens":2000}); text(r.output);"#
            ),
            [command("rg --files -g 'calc.py'")]
        );
        let patch = "*** Begin Patch\n*** Update File: calc.py\n@@\n+x\n*** End Patch";
        assert_eq!(
            calls(
                r#"const r = await tools.apply_patch("*** Begin Patch\n*** Update File: calc.py\n@@\n+x\n*** End Patch");
text(r);"#
            ),
            [Call::Patch(patch.into())]
        );
        assert_eq!(
            calls(
                r#"const patch = "*** Begin Patch\n*** Update File: calc.py\n@@\n+x\n*** End Patch";
text(await tools.apply_patch(patch));"#
            ),
            [Call::Patch(patch.into())]
        );
    }

    #[test]
    fn several_calls_come_out_in_the_order_they_are_written() {
        let js = r#"
            // first look around
            const [a, b] = await Promise.all([
              tools.exec_command({workdir: '/w', cmd: 'ls -la', yield_time_ms: 500}),
              tools.exec_command({cmd: `git status`}),
            ]);
            /* then change things */
            await tools.apply_patch("*** Begin Patch\n*** Add File: a.py\n+x\n*** End Patch");
            await tools.exec_command({cmd: "python a.py", env: {A: "1", B: [2, 3]}});
        "#;
        let got = calls(js);
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!(got[0], command("ls -la"));
        assert_eq!(got[1], command("git status"));
        assert!(matches!(&got[2], Call::Patch(p) if p.contains("Add File: a.py")));
        assert_eq!(got[3], command("python a.py"));
    }

    #[test]
    fn escapes_are_the_characters_they_stand_for() {
        let got = calls(r#"tools.exec_command({cmd:"echo \"hi\" \\ é \u{1F600} 😀 \x41\tz"})"#);
        assert_eq!(got, [command("echo \"hi\" \\ é 😀 😀 A\tz")]);
        let got = calls("tools.exec_command({cmd:\"a \\\nb\"})");
        assert_eq!(got, [command("a b")], "a line continuation adds nothing");
    }

    #[test]
    fn a_command_that_needs_the_program_to_run_is_not_guessed() {
        for js in [
            "const cmd = 'ls'; await tools.exec_command({cmd});",
            "await tools.exec_command({cmd: command});",
            "await tools.exec_command({cmd: \"git \" + sub});",
            "await tools.exec_command({cmd: `echo ${x}`});",
            "await tools.exec_command(args);",
            "await tools.exec_command({cmd: \"unfinished",
        ] {
            assert!(calls(js).is_empty(), "{js}");
        }
    }

    #[test]
    fn the_words_of_a_call_in_a_string_or_a_comment_are_no_call() {
        let js = r#"
            // tools.exec_command({cmd:"commented"})
            /* tools.exec_command({cmd:"blocked"}) */
            text("tools.exec_command({cmd:\"quoted\"})");
            await tools.exec_command({cmd: "real"});
        "#;
        assert_eq!(calls(js), [command("real")]);
    }

    #[test]
    fn a_patch_in_a_command_is_one_call_not_two() {
        let js = r#"await tools.exec_command({cmd:"apply_patch <<'EOF'\n*** Begin Patch\n*** Add File: a\n*** End Patch\nEOF"});"#;
        let got = calls(js);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(matches!(&got[0], Call::Command(c) if c.starts_with("apply_patch")));
    }
}
