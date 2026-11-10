//! Putting text on the clipboard with a program that reads it from standard input: `pbcopy` on
//! macOS, and on Linux the first of `wl-copy` (Wayland), `xclip` and `xsel` (X11) that works.

use super::{search_path, MACOS};
use crate::util::find_in;
use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

/// A program that copies its standard input to the clipboard.
#[derive(Debug, PartialEq, Eq)]
pub struct Tool {
    pub program: &'static str,
    pub args: &'static [&'static str],
}

const MAC_TOOLS: &[Tool] = &[Tool {
    program: "pbcopy",
    args: &[],
}];

const LINUX_TOOLS: &[Tool] = &[
    Tool {
        program: "wl-copy",
        args: &[],
    },
    Tool {
        program: "xclip",
        args: &["-selection", "clipboard"],
    },
    Tool {
        program: "xsel",
        args: &["-b", "-i"],
    },
];

/// The tools to try, in order.
pub fn tools() -> &'static [Tool] {
    if MACOS {
        MAC_TOOLS
    } else {
        LINUX_TOOLS
    }
}

/// Copy `text` to the clipboard. False when no tool took it: none is installed, or each one failed
/// (on Linux that is also what a missing display looks like, as for a daemon started by systemd).
pub fn copy(text: &str) -> bool {
    copy_with(&search_path(), tools(), text).is_some()
}

/// Try each tool found on `path` until one takes the text; the name of the one that did.
fn copy_with(path: &OsStr, tools: &'static [Tool], text: &str) -> Option<&'static str> {
    tools
        .iter()
        .find(|t| find_in(t.program, path).is_some_and(|exe| pipe_to(&exe, t.args, text)))
        .map(|t| t.program)
}

/// Run a clipboard tool with the text on its standard input; did it succeed?
/// The tools keep serving the clipboard from a background process, so their output is not held
/// open here: that would wait for as long as the text stays on the clipboard.
fn pipe_to(exe: &std::path::Path, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(exe)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let written = child
        .stdin
        .take()
        .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    let exited = child.wait().is_ok_and(|s| s.success());
    written && exited
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_support::script;
    use std::path::Path;

    /// A clipboard tool in `dir`: it saves what it is given to `<dir>/<name>.clip` and its
    /// arguments to `<dir>/<name>.args`, or, when `code` is not 0, does not read at all and fails
    /// (a tool that cannot reach a display).
    fn tool(dir: &Path, name: &str, code: i32) {
        let body = if code == 0 {
            format!("cat > \"$(dirname \"$0\")/{name}.clip\"\necho \"$@\" > \"$(dirname \"$0\")/{name}.args\"")
        } else {
            format!("echo \"no display\" >&2\nexit {code}")
        };
        script(dir, name, &body);
    }

    fn saved(dir: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(format!("{name}.clip"))).ok()
    }

    #[test]
    fn the_tools_are_tried_in_the_documented_order() {
        let names = |t: &[Tool]| t.iter().map(|t| t.program).collect::<Vec<_>>();
        assert_eq!(names(MAC_TOOLS), ["pbcopy"]);
        assert_eq!(names(LINUX_TOOLS), ["wl-copy", "xclip", "xsel"]);
        assert_eq!(LINUX_TOOLS[1].args, ["-selection", "clipboard"]);
        assert_eq!(LINUX_TOOLS[2].args, ["-b", "-i"]);
        assert_eq!(
            tools() == MAC_TOOLS,
            cfg!(target_os = "macos"),
            "the tools of this OS"
        );
    }

    #[test]
    fn the_first_tool_that_works_gets_the_text() {
        let t = tempfile::tempdir().unwrap();
        tool(t.path(), "wl-copy", 0);
        tool(t.path(), "xclip", 0);
        let got = copy_with(
            t.path().as_os_str(),
            LINUX_TOOLS,
            "cd '/a b' && claude --resume x",
        );
        assert_eq!(got, Some("wl-copy"));
        assert_eq!(
            saved(t.path(), "wl-copy").as_deref(),
            Some("cd '/a b' && claude --resume x")
        );
        assert_eq!(
            saved(t.path(), "xclip"),
            None,
            "the later tools are not run"
        );
    }

    #[test]
    fn a_tool_that_fails_gives_way_to_the_next_one() {
        let t = tempfile::tempdir().unwrap();
        tool(t.path(), "wl-copy", 1);
        tool(t.path(), "xclip", 1);
        tool(t.path(), "xsel", 0);
        let got = copy_with(t.path().as_os_str(), LINUX_TOOLS, "text");
        assert_eq!(got, Some("xsel"));
        assert_eq!(saved(t.path(), "xsel").as_deref(), Some("text"));
        let args = std::fs::read_to_string(t.path().join("xsel.args")).unwrap();
        assert_eq!(args.trim(), "-b -i");
    }

    #[test]
    fn xclip_gets_the_clipboard_selection() {
        let t = tempfile::tempdir().unwrap();
        tool(t.path(), "xclip", 0);
        assert_eq!(
            copy_with(t.path().as_os_str(), LINUX_TOOLS, "x"),
            Some("xclip")
        );
        let args = std::fs::read_to_string(t.path().join("xclip.args")).unwrap();
        assert_eq!(args.trim(), "-selection clipboard");
    }

    #[test]
    fn no_tool_means_no_copy() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(copy_with(t.path().as_os_str(), LINUX_TOOLS, "x"), None);
        tool(t.path(), "wl-copy", 1);
        assert_eq!(copy_with(t.path().as_os_str(), LINUX_TOOLS, "x"), None);
        assert_eq!(copy_with(OsStr::new(""), LINUX_TOOLS, "x"), None);
    }

    #[test]
    fn text_with_quotes_and_newlines_arrives_as_it_is() {
        let t = tempfile::tempdir().unwrap();
        tool(t.path(), "pbcopy", 0);
        let text = "a'b\"c $(x)\n`y` é";
        assert_eq!(
            copy_with(t.path().as_os_str(), MAC_TOOLS, text),
            Some("pbcopy")
        );
        assert_eq!(saved(t.path(), "pbcopy").as_deref(), Some(text));
    }
}
