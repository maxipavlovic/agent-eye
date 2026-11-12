//! Binaries a package manager owns: Homebrew's (in its Cellar) and cargo's (`cargo install`). eye
//! runs those from where they are, so an upgrade reaches the service and the hooks, and it never
//! copies, relinks or deletes them.

use super::BIN_NAME;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package {
    Homebrew,
    Cargo,
}

impl Package {
    pub fn name(self) -> &'static str {
        match self {
            Package::Homebrew => "Homebrew",
            Package::Cargo => "cargo",
        }
    }
    /// The command that removes the binary, for uninstall to point at.
    pub fn remove_hint(self) -> &'static str {
        match self {
            Package::Homebrew => "brew uninstall agent-eye",
            Package::Cargo => "cargo uninstall agent-eye",
        }
    }
}

/// cargo's home: `CARGO_HOME`, else `~/.cargo`.
pub fn cargo_home() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
}

/// The package manager that owns the binary at `exe`, and the folder that always holds its current
/// version: Homebrew's `<prefix>/opt/<formula>/bin`, which every `brew upgrade` moves along, or
/// cargo's bin folder, which `cargo install` writes over. None for any other binary.
pub fn owner(exe: &Path, cargo_home: Option<&Path>) -> Option<(Package, PathBuf)> {
    let real = std::fs::canonicalize(exe).ok()?;
    let runs = |dir: &Path| std::fs::canonicalize(dir.join(BIN_NAME)).ok().as_ref() == Some(&real);
    if let Some(dir) = homebrew_opt_bin(&real).filter(|d| runs(d)) {
        return Some((Package::Homebrew, dir));
    }
    let dir = cargo_home?.join("bin");
    runs(&dir).then_some((Package::Cargo, dir))
}

/// `<prefix>/opt/<formula>/bin` for a binary at `<prefix>/Cellar/<formula>/<version>/bin/<name>`.
fn homebrew_opt_bin(real: &Path) -> Option<PathBuf> {
    let parts: Vec<Component> = real.components().collect();
    let cellar = parts.len().checked_sub(5)?;
    let [Component::Normal(c), Component::Normal(formula), _, Component::Normal(bin), _] =
        parts[cellar..]
    else {
        return None;
    };
    if c != "Cellar" || bin != "bin" {
        return None;
    }
    let prefix: PathBuf = parts[..cellar].iter().collect();
    Some(prefix.join("opt").join(formula).join("bin"))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A Homebrew prefix under `t` with `agent-eye` at `version`, linked the way brew links it.
    /// The prefix comes back without symlinks (macOS keeps temp folders behind `/var`).
    pub(in crate::install) fn brew(t: &Path, version: &str) -> PathBuf {
        std::fs::create_dir_all(t).unwrap();
        let t = &std::fs::canonicalize(t).unwrap();
        let keg = t.join("Cellar/agent-eye").join(version);
        std::fs::create_dir_all(keg.join("bin")).unwrap();
        std::fs::write(keg.join("bin/agent-eye"), version).unwrap();
        symlink("agent-eye", keg.join("bin/eye")).unwrap();
        std::fs::create_dir_all(t.join("opt")).unwrap();
        std::fs::create_dir_all(t.join("bin")).unwrap();
        let _ = std::fs::remove_file(t.join("opt/agent-eye"));
        symlink(
            format!("../Cellar/agent-eye/{version}"),
            t.join("opt/agent-eye"),
        )
        .unwrap();
        for name in ["agent-eye", "eye"] {
            let _ = std::fs::remove_file(t.join("bin").join(name));
            symlink(
                format!("../Cellar/agent-eye/{version}/bin/{name}"),
                t.join("bin").join(name),
            )
            .unwrap();
        }
        t.to_path_buf()
    }

    #[test]
    fn a_homebrew_binary_runs_from_its_opt_folder_whichever_link_started_it() {
        let t = tempfile::tempdir().unwrap();
        let p = brew(t.path(), "0.2.0");
        let opt = p.join("opt/agent-eye/bin");
        for started in [
            p.join("bin/agent-eye"),
            p.join("bin/eye"),
            p.join("opt/agent-eye/bin/agent-eye"),
            p.join("Cellar/agent-eye/0.2.0/bin/agent-eye"),
        ] {
            assert_eq!(
                owner(&started, None),
                Some((Package::Homebrew, opt.clone())),
                "{}",
                started.display()
            );
        }
    }

    #[test]
    fn an_old_keg_that_opt_no_longer_points_at_is_not_homebrews_current_binary() {
        let t = tempfile::tempdir().unwrap();
        let p = brew(t.path(), "0.2.0");
        brew(t.path(), "0.2.1");
        assert_eq!(
            owner(&p.join("Cellar/agent-eye/0.2.0/bin/agent-eye"), None),
            None
        );
    }

    #[test]
    fn a_cargo_install_runs_from_cargos_bin_folder() {
        let t = tempfile::tempdir().unwrap();
        let cargo = t.path().join("cargo");
        std::fs::create_dir_all(cargo.join("bin")).unwrap();
        std::fs::write(cargo.join("bin/agent-eye"), "x").unwrap();
        assert_eq!(
            owner(&cargo.join("bin/agent-eye"), Some(&cargo)),
            Some((Package::Cargo, cargo.join("bin")))
        );
    }

    #[test]
    fn any_other_binary_has_no_owner() {
        let t = tempfile::tempdir().unwrap();
        let local = t.path().join("local/bin");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("agent-eye"), "x").unwrap();
        let cargo = t.path().join("cargo");
        assert_eq!(owner(&local.join("agent-eye"), Some(&cargo)), None);
        assert_eq!(owner(&t.path().join("missing"), Some(&cargo)), None);
        // a folder named Cellar that is not laid out like Homebrew's
        let odd = t.path().join("Cellar/agent-eye");
        std::fs::create_dir_all(&odd).unwrap();
        std::fs::write(odd.join("agent-eye"), "x").unwrap();
        assert_eq!(owner(&odd.join("agent-eye"), None), None);
    }
}
