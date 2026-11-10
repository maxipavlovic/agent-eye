//! What `install`, `uninstall` and `board on|off` print: a line for each area, short, with the
//! details behind a line (a file diff) kept for `--dry-run` and `--verbose`.

/// One line of the summary: an area (`binary`, `claude`, `daemon`), what happened to it, and the
/// details a person only wants to see on request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub area: String,
    pub text: String,
    pub detail: Vec<String>,
}

impl Row {
    pub fn new(area: impl Into<String>, text: impl Into<String>) -> Row {
        Row {
            area: area.into(),
            text: text.into(),
            detail: vec![],
        }
    }

    pub fn with_detail(mut self, detail: Vec<String>) -> Row {
        self.detail = detail;
        self
    }
}

/// The rows, what the person has to do next, and whether something failed.
#[derive(Debug, Default)]
pub struct Report {
    pub rows: Vec<Row>,
    /// Things to do after this, each said once at the end (`Next: ...`).
    pub next: Vec<String>,
    /// Something it set out to do did not work, so the command exits with an error.
    pub failed: bool,
}

impl Report {
    pub fn push(&mut self, row: Row) {
        self.rows.push(row);
    }

    /// Add what another report says, after what this one says.
    pub fn extend(&mut self, other: Report) {
        self.rows.extend(other.rows);
        for n in other.next {
            if !self.next.contains(&n) {
                self.next.push(n);
            }
        }
        self.failed |= other.failed;
    }

    /// Nothing to say.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.next.is_empty()
    }

    /// The row of an area, if there is one.
    pub fn row(&self, area: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.area == area)
    }

    /// The text, a title first when there is one. Details are shown under their line when
    /// `details` is set.
    pub fn render(&self, title: &str, details: bool) -> String {
        let mut out = String::new();
        if !title.is_empty() {
            out.push_str(title);
            out.push('\n');
        }
        out.push_str(&self.body(details));
        out
    }

    /// The rows, then a blank line and the `Next:` lines.
    pub fn body(&self, details: bool) -> String {
        let width = self.rows.iter().map(|r| r.area.chars().count()).max();
        let width = width.unwrap_or(0).max(MIN_AREA);
        let mut out = String::new();
        for r in &self.rows {
            out.push_str(&format!("  {:<width$}  {}\n", r.area, r.text));
            if details {
                for d in &r.detail {
                    out.push_str(&format!("  {:<width$}    {d}\n", ""));
                }
            }
        }
        if !self.next.is_empty() {
            out.push('\n');
            for n in &self.next {
                out.push_str(&format!("Next: {n}\n"));
            }
        }
        out
    }
}

/// The area column is at least as wide as `daemon`, so short reports line up with long ones.
const MIN_AREA: usize = 6;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        let mut r = Report::default();
        r.push(Row::new("binary", "~/.local/bin/agent-eye (unchanged)"));
        r.push(
            Row::new("codex", "hooks added (6 events)")
                .with_detail(vec!["+    1: {".into(), "+    2:   \"hooks\": {".into()]),
        );
        r.next.push("start Codex and run /hooks".into());
        r
    }

    #[test]
    fn rows_line_up_and_details_come_only_when_asked_for() {
        let r = sample();
        assert_eq!(
            r.render("agent-eye install", false),
            "agent-eye install\n  binary  ~/.local/bin/agent-eye (unchanged)\n  codex   hooks added (6 events)\n\nNext: start Codex and run /hooks\n"
        );
        assert_eq!(
            r.render("agent-eye install", true),
            "agent-eye install\n  binary  ~/.local/bin/agent-eye (unchanged)\n  codex   hooks added (6 events)\n            +    1: {\n            +    2:   \"hooks\": {\n\nNext: start Codex and run /hooks\n"
        );
    }

    #[test]
    fn a_long_area_widens_the_column_and_no_title_means_no_heading() {
        let mut r = Report::default();
        r.push(Row::new("antigravity", "hooks ok"));
        r.push(Row::new("board", "off"));
        assert_eq!(
            r.render("", false),
            "  antigravity  hooks ok\n  board        off\n"
        );
    }

    #[test]
    fn extending_keeps_each_next_line_once_and_remembers_a_failure() {
        let mut a = sample();
        let mut b = Report::default();
        b.push(Row::new("daemon", "started"));
        b.next.push("start Codex and run /hooks".into());
        b.next.push("something else".into());
        b.failed = true;
        a.extend(b);
        assert_eq!(a.rows.len(), 3);
        assert_eq!(a.next, ["start Codex and run /hooks", "something else"]);
        assert!(a.failed);
        assert_eq!(a.row("daemon").unwrap().text, "started");
        assert!(a.row("nope").is_none());
    }
}
