//! Moving a pack between machines: the home folder differs (`/Users/max` on a Mac, `/home/max` on
//! Linux), so every path that starts with the pack's home is rewritten to start with this
//! machine's, and so is Claude's encoded form of it (`-Users-max-code` -> `-home-max-code`).

/// How paths of the pack become paths of this machine. Two prefixes at most: eye's own home (which
/// may be set apart from the user's home with `EYE_HOME`) is tried before the user's home.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Remap {
    /// (from, to) pairs, most specific first.
    pairs: Vec<(String, String)>,
}

/// Claude's folder name for a path: every character that is not a letter or a digit becomes `-`.
pub fn encode(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `path` with the folder prefix `from` replaced by `to`, when it starts with it as a whole
/// folder (`/Users/max` matches `/Users/max/x` and itself, not `/Users/maxim`).
fn swap_prefix(path: &str, from: &str, to: &str) -> Option<String> {
    let from = from.trim_end_matches('/');
    let rest = path.strip_prefix(from)?;
    (rest.is_empty() || rest.starts_with('/'))
        .then(|| format!("{}{rest}", to.trim_end_matches('/')))
}

impl Remap {
    /// The rewrite from the pack's folders to this machine's. Empty (nothing to do) when they are
    /// the same.
    pub fn new(pack_home: &str, home: &str, pack_eye: &str, eye: &str) -> Remap {
        let mut pairs = vec![];
        if pack_eye != eye && !pack_eye.is_empty() && !eye.is_empty() {
            pairs.push((pack_eye.to_string(), eye.to_string()));
        }
        if pack_home != home && !pack_home.is_empty() && !home.is_empty() {
            pairs.push((pack_home.to_string(), home.to_string()));
        }
        Remap { pairs }
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The pairs, for the plan to show.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    /// A path of the pack as a path of this machine.
    pub fn path(&self, p: &str) -> String {
        self.pairs
            .iter()
            .find_map(|(from, to)| swap_prefix(p, from, to))
            .unwrap_or_else(|| p.to_string())
    }

    /// A project folder name: Claude's encoded path (`-Users-max-code`) is rewritten the same way,
    /// anything else (another agent's name for a project) is left alone unless it is a path.
    pub fn project_dir(&self, pd: &str) -> String {
        if pd.starts_with('/') {
            return self.path(pd);
        }
        for (from, to) in &self.pairs {
            let (from, to) = (
                encode(from.trim_end_matches('/')),
                encode(to.trim_end_matches('/')),
            );
            if let Some(rest) = pd.strip_prefix(&from) {
                if rest.is_empty() || rest.starts_with('-') {
                    return format!("{to}{rest}");
                }
            }
        }
        pd.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_prefix_is_swapped_as_a_whole_folder() {
        let r = Remap::new(
            "/Users/max",
            "/home/max",
            "/Users/max/.agent-eye",
            "/home/max/.agent-eye",
        );
        assert_eq!(r.path("/Users/max/code/x"), "/home/max/code/x");
        assert_eq!(r.path("/Users/max"), "/home/max");
        assert_eq!(r.path("/Users/maxim/code"), "/Users/maxim/code");
        assert_eq!(r.path("/tmp/x"), "/tmp/x");
        assert_eq!(
            r.path("/Users/max/.agent-eye/archive/-Users-max-code/a.jsonl.zst"),
            "/home/max/.agent-eye/archive/-Users-max-code/a.jsonl.zst"
        );
    }

    #[test]
    fn claudes_encoded_form_follows() {
        let r = Remap::new(
            "/Users/max",
            "/home/max",
            "/Users/max/.agent-eye",
            "/home/max/.agent-eye",
        );
        assert_eq!(r.project_dir("-Users-max-code"), "-home-max-code");
        assert_eq!(r.project_dir("-Users-max"), "-home-max");
        assert_eq!(r.project_dir("-Users-max--claude-x"), "-home-max--claude-x");
        assert_eq!(r.project_dir("-Users-maxim-code"), "-Users-maxim-code");
        assert_eq!(r.project_dir("-tmp-demo"), "-tmp-demo");
        assert_eq!(r.project_dir("2026/11/21"), "2026/11/21");
    }

    #[test]
    fn the_same_home_changes_nothing() {
        let r = Remap::new(
            "/Users/max",
            "/Users/max",
            "/Users/max/.agent-eye",
            "/Users/max/.agent-eye",
        );
        assert!(r.is_empty());
        assert_eq!(r.path("/Users/max/x"), "/Users/max/x");
    }
}
