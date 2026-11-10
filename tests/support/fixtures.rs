//! The synthetic Claude Code home in `tests/fixtures/claude-home`, copied somewhere temporary.

use std::path::{Path, PathBuf};

pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// Copy the fixture Claude home to `<tmp>/claude` and return that path.
pub fn claude_home(tmp: &Path) -> PathBuf {
    let claude = tmp.join("claude");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude-home"),
        &claude,
    );
    claude
}
