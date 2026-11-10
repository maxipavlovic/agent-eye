//! The `/eye` skill an agent reads: one markdown file in the agent's skills folder.

use crate::files::write_file;
use anyhow::Result;
use std::path::Path;

/// What installing the skill found and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Added,
    Updated,
    Unchanged,
}

/// Write the skill to `path` unless it is there as it should be. Nothing is written in a dry run.
pub fn install(path: &Path, text: &str, dry_run: bool) -> Result<State> {
    let state = match std::fs::read_to_string(path) {
        Ok(t) if t == text => State::Unchanged,
        Ok(_) => State::Updated,
        Err(_) => State::Added,
    };
    if !dry_run && state != State::Unchanged {
        write_file(path, text.as_bytes(), 0o644)?;
    }
    Ok(state)
}

impl State {
    /// For the summary: `skill added`, `skill ok`; `would add skill` in a dry run.
    pub fn said(self, dry: bool) -> &'static str {
        match (self, dry) {
            (State::Added, false) => "skill added",
            (State::Added, true) => "would add skill",
            (State::Updated, false) => "skill updated",
            (State::Updated, true) => "would update skill",
            (State::Unchanged, _) => "skill ok",
        }
    }
}

/// Remove the skill, and its folder when that is empty. False when there was no skill.
pub fn remove(path: &Path, dry_run: bool) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if !dry_run {
        std::fs::remove_file(path)?;
        if let Some(d) = path.parent() {
            let _ = std::fs::remove_dir(d); // only when empty
        }
    }
    Ok(true)
}

/// For the summary of an uninstall.
pub fn removed_said(removed: bool, dry: bool) -> &'static str {
    match (removed, dry) {
        (true, false) => "skill removed",
        (true, true) => "would remove skill",
        (false, _) => "no skill found",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_skill_is_added_then_left_alone_then_updated() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("skills/eye/SKILL.md");
        assert_eq!(install(&p, "one", true).unwrap(), State::Added);
        assert!(!p.exists(), "a dry run writes nothing");
        assert_eq!(install(&p, "one", false).unwrap(), State::Added);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one");
        assert_eq!(install(&p, "one", false).unwrap(), State::Unchanged);
        assert_eq!(install(&p, "two", true).unwrap(), State::Updated);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one");
        assert_eq!(install(&p, "two", false).unwrap(), State::Updated);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
    }

    #[test]
    fn the_summary_words_follow_the_state_and_the_dry_run() {
        assert_eq!(State::Added.said(false), "skill added");
        assert_eq!(State::Added.said(true), "would add skill");
        assert_eq!(State::Updated.said(false), "skill updated");
        assert_eq!(State::Updated.said(true), "would update skill");
        assert_eq!(State::Unchanged.said(true), "skill ok");
        assert_eq!(removed_said(true, false), "skill removed");
        assert_eq!(removed_said(true, true), "would remove skill");
        assert_eq!(removed_said(false, true), "no skill found");
    }

    #[test]
    fn removing_takes_the_folder_too_when_it_is_empty_and_only_then() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("skills/eye");
        let p = dir.join("SKILL.md");
        assert!(!remove(&p, false).unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, "x").unwrap();
        assert!(remove(&p, true).unwrap());
        assert!(p.exists(), "a dry run removes nothing");
        assert!(remove(&p, false).unwrap());
        assert!(!dir.exists());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, "x").unwrap();
        std::fs::write(dir.join("mine.md"), "y").unwrap();
        assert!(remove(&p, false).unwrap());
        assert!(dir.join("mine.md").exists(), "somebody else's file stays");
    }
}
