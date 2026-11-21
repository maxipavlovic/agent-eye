//! The one `[y/N]` question of the command line.

use anyhow::{bail, Context, Result};
use std::io::{BufRead, IsTerminal, Write};

/// Where the answer comes from.
#[derive(Clone, Copy)]
pub enum From<'a> {
    /// Standard input, which must be a terminal; a script says `--yes`. The text finishes
    /// "add --yes to …".
    Terminal(&'a str),
    /// The terminal itself (`/dev/tty`), for a command whose standard input carries data.
    DevTty(&'a str),
    /// Standard input, a terminal or not.
    Stdin,
}

/// Ask `question` and the `[y/N]` after it; anything but y or yes is no.
pub fn confirm(question: &str, from: From) -> Result<bool> {
    let line = match from {
        From::Terminal(hint) => {
            if !std::io::stdin().is_terminal() {
                bail!("not a terminal: add --yes to {hint}");
            }
            prompt(question)?;
            read(std::io::stdin().lock())?
        }
        From::DevTty(hint) => {
            prompt(question)?;
            let tty = std::fs::File::open("/dev/tty")
                .with_context(|| format!("no terminal to ask on: add --yes to {hint}"))?;
            read(std::io::BufReader::new(tty))?
        }
        From::Stdin => {
            prompt(question)?;
            read(std::io::stdin().lock())?
        }
    };
    Ok(is_yes(&line))
}

fn prompt(question: &str) -> Result<()> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    Ok(())
}

fn read(mut from: impl BufRead) -> Result<String> {
    let mut line = String::new();
    from.read_line(&mut line).context("reading the answer")?;
    Ok(line)
}

/// `y`, `Y`, `yes` in any case.
fn is_yes(line: &str) -> bool {
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_y_and_yes_are_yes() {
        for y in ["y\n", "Y", " yes ", "YES\n"] {
            assert!(is_yes(y), "{y:?}");
        }
        for n in ["", "\n", "n", "no", "yep", "ye", "y y"] {
            assert!(!is_yes(n), "{n:?}");
        }
    }

    #[test]
    fn the_answer_is_one_line_of_the_reader() {
        assert_eq!(read(&b"y\nmore"[..]).unwrap(), "y\n");
        assert_eq!(read(&b""[..]).unwrap(), "");
    }
}
