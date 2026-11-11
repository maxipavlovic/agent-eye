use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The config written on first run, with `{resume}` still to fill in; `docs/config.md` explains
/// every key.
const DEFAULT_TOML_TEMPLATE: &str = r#"[agents]
enabled = ["claude"]  # the agents eye follows; the first is the default. [] turns every agent off

# [agents.claude]
# dir = "~/.claude"   # where Claude Code keeps its files (the EYE_CLAUDE_DIR variable wins)
# [agents.codex]
# dir = "~/.codex"    # where Codex CLI keeps its files (else the CODEX_HOME variable)

[summary]
enabled = true
quiet_minutes = 10
model = ""         # the model that writes them; empty = the agent's own cheap one (haiku for Claude Code, gpt-6-luna for Codex CLI)
agent = ""         # which agent writes summaries; empty = the first enabled one
concurrency = 2

[board]
enabled = false    # the web board is optional: eye board on
port = 7420
resume = "{resume}"   # how Resume/Start open a session: "auto" (iTerm2 or Terminal on a Mac, your terminal on Linux, else copy), "iterm", "terminal", "copy"
done_days = 7      # Done sessions older than this leave the board for the archive view; 0 = keep them

[user]
name = ""          # your name as older summaries wrote it, e.g. "Sam": lines starting "Sam:" show as "You:"

[exclude]
paths = []         # cwd prefixes eye ignores completely

[outputs]
jira_projects = [] # empty = learn project prefixes; or list them, e.g. ["PROJ", "ACME"]
jira_url = ""     # Jira keys link to the site seen in your links; set one for keys never linked

[hide]
trivial = true         # hide throw-away sessions ("/clear", "exit", "cd ..") from the board, archive, find and ls
max_prompts = 1
max_reply_chars = 200
commands = ["exit", "quit", "resume", "usage", "clear", "cd", "help"]  # first words that mark a trivial prompt

[backup]
enabled = true         # a daily snapshot of the database, and one before a schema upgrade; false = only `eye backup` by hand
keep_daily = 7
keep_weekly = 4
mirror = ""            # a folder that already exists (an external disk, a synced folder) to copy each snapshot into; empty = off
mirror_archive = false # also copy the transcript archive into <mirror>/archive
"#;

/// The config written on first run.
pub fn default_toml() -> String {
    DEFAULT_TOML_TEMPLATE.replace("{resume}", crate::platform::default_resume())
}

/// All settings, one field per `[section]`; every key has a default.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub agents: AgentsCfg,
    pub summary: SummaryCfg,
    pub user: UserCfg,
    pub board: BoardCfg,
    pub exclude: ExcludeCfg,
    pub outputs: OutputsCfg,
    pub hide: HideCfg,
    pub backup: BackupCfg,
}

/// Which coding agents eye follows, and where each keeps its files.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AgentsCfg {
    /// Ids of the enabled agents, in order; the first is the default. Not set: Claude Code only.
    pub enabled: Option<Vec<String>>,
    /// `[agents.<id>]` tables.
    #[serde(flatten)]
    pub each: BTreeMap<String, AgentCfg>,
}

/// Settings of one agent (`[agents.<id>]`).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AgentCfg {
    /// Where the agent keeps its files; a leading `~` is the home folder. Not set: the agent's
    /// usual place.
    pub dir: Option<String>,
    /// A folder given on the command line; it beats the environment and `dir`.
    #[serde(skip)]
    pub dir_override: Option<PathBuf>,
}

/// A leading `~` or `~/` is the home folder.
pub fn expand_tilde(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(h)) => PathBuf::from(h).join(rest),
        (None, Some(h)) if p == "~" => PathBuf::from(h),
        _ => PathBuf::from(p),
    }
}

/// Who "you" is. Summaries name the user `You`; older ones may carry a real name.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct UserCfg {
    /// A stored next line that starts with `<name>:` is shown as `You:`.
    pub name: String,
}

impl UserCfg {
    /// `Sam: review the PR` becomes `You: review the PR` when `name` is "Sam" (any case).
    pub fn as_you(&self, line: &str) -> String {
        match self.owner_rest(line) {
            Some(rest) => format!("You: {rest}"),
            None => line.to_string(),
        }
    }

    /// The text after the owner when the line is the user's (`You:` or `<name>:`), else the line.
    pub fn without_you<'a>(&self, line: &'a str) -> &'a str {
        match line.strip_prefix("You:") {
            Some(rest) => rest.trim_start(),
            None => self.owner_rest(line).unwrap_or(line),
        }
    }

    fn owner_rest<'a>(&self, line: &'a str) -> Option<&'a str> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let head = line.get(..name.len())?;
        let rest = line[name.len()..].strip_prefix(':')?;
        head.eq_ignore_ascii_case(name).then(|| rest.trim_start())
    }
}

/// A session is trivial (and hidden) when it has at most `max_prompts` prompts, no outputs or
/// children, a last reply of at most `max_reply_chars`, and a first prompt that is a slash command,
/// starts with one of `commands`, or is a single short word. `eye hide` / `eye unhide` override it.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HideCfg {
    pub trivial: bool,
    pub max_prompts: i64,
    pub max_reply_chars: usize,
    pub commands: Vec<String>,
}
impl Default for HideCfg {
    fn default() -> Self {
        Self {
            trivial: true,
            max_prompts: 1,
            max_reply_chars: 200,
            commands: ["exit", "quit", "resume", "usage", "clear", "cd", "help"]
                .map(String::from)
                .to_vec(),
        }
    }
}

/// `[backup]`: snapshots of the database, how many to keep, and the optional mirror folder.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BackupCfg {
    /// Automatic snapshots (daily, and before a schema upgrade). `eye backup` always works.
    pub enabled: bool,
    pub keep_daily: usize,
    pub keep_weekly: usize,
    /// A folder to copy each snapshot into; it must exist already. Empty is off.
    pub mirror: String,
    /// Also copy the transcript archive into `<mirror>/archive`.
    pub mirror_archive: bool,
}
impl Default for BackupCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            keep_daily: 7,
            keep_weekly: 4,
            mirror: String::new(),
            mirror_archive: false,
        }
    }
}
impl BackupCfg {
    /// The mirror folder, with a leading `~` as the home folder; None when it is off.
    pub fn mirror_dir(&self) -> Option<PathBuf> {
        let m = self.mirror.trim();
        if m.is_empty() {
            return None;
        }
        Some(expand_tilde(m))
    }
}

/// `[outputs]`: which Jira projects and site to link.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct OutputsCfg {
    pub jira_projects: Vec<String>,
    /// Jira site for keys whose site eye has not seen in a link, e.g. "https://acme.atlassian.net".
    pub jira_url: String,
}

/// `[summary]`: when, by whom and how summaries are written.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SummaryCfg {
    pub enabled: bool,
    pub quiet_minutes: u32,
    /// The model handed to the summary agent; empty is that agent's own default
    /// (`Agent::default_summary_model`), whichever agent it is.
    pub model: String,
    /// The agent that writes summaries; empty is the first enabled one.
    pub agent: String,
    pub concurrency: u32,
    /// The daemon only summarizes sessions active within this many days.
    pub auto_days: u32,
}
impl Default for SummaryCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet_minutes: 10,
            model: String::new(),
            agent: String::new(),
            concurrency: 2,
            auto_days: 3,
        }
    }
}

/// `[board]`: the optional web board and how Resume opens a session.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BoardCfg {
    /// The daemon serves the web board only when this is true.
    pub enabled: bool,
    pub port: u16,
    pub resume: String,
    /// Done sessions idle for longer than this many days leave the board (0 = no limit).
    pub done_days: u32,
}
impl Default for BoardCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 7420,
            resume: crate::platform::default_resume().into(),
            done_days: 7,
        }
    }
}

/// `[exclude]`: folders that are never indexed.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ExcludeCfg {
    pub paths: Vec<String>,
}

impl Config {
    /// Parse the text of a `config.toml`.
    pub fn parse(s: &str) -> Result<Config> {
        toml::from_str(s).context("parsing config.toml")
    }

    /// Load the config, writing the defaults first when the file is missing.
    pub fn load_or_create(path: &Path) -> Result<Config> {
        if !path.exists() {
            crate::files::write_private(path, &default_toml())
                .with_context(|| format!("writing {}", path.display()))?;
        }
        crate::files::make_private(path);
        let s =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Config::parse(&s)
    }

    /// The ids in `[agents] enabled`; Claude Code when the key is not set.
    pub fn enabled_agent_ids(&self) -> Vec<String> {
        self.agents
            .enabled
            .clone()
            .unwrap_or_else(|| vec![crate::agents::LEGACY_ID.to_string()])
    }

    /// The settings of one agent (the defaults when it has no table).
    pub fn agent_cfg(&self, id: &str) -> AgentCfg {
        self.agents.each.get(id).cloned().unwrap_or_default()
    }

    /// Point an agent at a folder for this run only (the `--claude-dir` flag).
    pub fn override_agent_dir(&mut self, id: &str, dir: PathBuf) {
        self.agents
            .each
            .entry(id.to_string())
            .or_default()
            .dir_override = Some(dir);
    }

    /// Settings for tests and embedders: Claude Code reads its files from `dir`.
    pub fn with_claude_dir(dir: &Path) -> Config {
        let mut c = Config::default();
        c.override_agent_dir("claude", dir.to_path_buf());
        c
    }

    /// Is `cwd` inside an excluded folder?
    pub fn is_excluded(&self, cwd: &str) -> bool {
        self.exclude
            .paths
            .iter()
            .any(|p| !p.is_empty() && cwd.starts_with(p.as_str()))
    }
}

/// Set `key = value` in the section with this header (`[board]`) of a config.toml text, keeping every
/// other line and comment: a key that is there is replaced (its comment stays), one that is not is
/// added under the header, and a section that is not there is added at the end.
fn set_key(text: &str, header: &str, key: &str, value: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let line = format!("{key} = {value}");
    let at = lines.iter().position(|l| l.trim() == header);
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match at {
        Some(h) => {
            let end = lines[h + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['))
                .map_or(lines.len(), |i| h + 1 + i);
            let found = (h + 1..end).find(|&i| {
                let t = lines[i].trim_start();
                t.strip_prefix(key)
                    .is_some_and(|r| r.trim_start().starts_with('='))
            });
            match found {
                Some(i) => {
                    let indent = &lines[i][..lines[i].len() - lines[i].trim_start().len()];
                    let comment = lines[i].find('#').map(|c| &lines[i][c..]);
                    out[i] = match comment {
                        Some(c) => format!("{indent}{line}    {c}"),
                        None => format!("{indent}{line}"),
                    };
                }
                None => out.insert(h + 1, line),
            }
        }
        None => {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(header.into());
            out.push(line);
        }
    }
    out.join("\n") + "\n"
}

/// Set `enabled` in the `[board]` section of a config.toml text, keeping every other line and comment.
pub fn set_board_enabled(text: &str, on: bool) -> String {
    set_key(text, "[board]", "enabled", &on.to_string())
}

/// Set `enabled` in the `[agents]` section of a config.toml text to these agent ids.
pub fn set_agents_enabled(text: &str, ids: &[String]) -> String {
    let list: Vec<String> = ids.iter().map(|i| format!("{i:?}")).collect();
    set_key(
        text,
        "[agents]",
        "enabled",
        &format!("[{}]", list.join(", ")),
    )
}

/// The text of the config file, or the defaults when it is not there.
fn text_or_default(path: &Path) -> Result<String> {
    if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    } else {
        Ok(default_toml())
    }
}

/// Write `[board] enabled` into the config file (created from the defaults when missing).
pub fn write_board_enabled(path: &Path, on: bool) -> Result<()> {
    let text = set_board_enabled(&text_or_default(path)?, on);
    crate::files::write_private(path, &text).with_context(|| format!("writing {}", path.display()))
}

/// Write `[agents] enabled` into the config file (created from the defaults when missing).
pub fn write_agents_enabled(path: &Path, ids: &[String]) -> Result<()> {
    let text = set_agents_enabled(&text_or_default(path)?, ids);
    crate::files::write_private(path, &text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_is_off_by_default() {
        assert!(!Config::default().board.enabled);
        assert!(!Config::parse(&default_toml()).unwrap().board.enabled);
        assert!(
            Config::parse("[board]\nenabled=true")
                .unwrap()
                .board
                .enabled
        );
    }

    #[test]
    fn set_enabled_key_present() {
        let t = set_board_enabled(&default_toml(), true);
        assert!(t.contains("enabled = true    # the web board is optional: eye board on\n"));
        assert_eq!(t.lines().count(), default_toml().lines().count());
        assert!(Config::parse(&t).unwrap().board.enabled);
        assert_eq!(set_board_enabled(&t, false), default_toml());
    }

    #[test]
    fn the_agents_are_set_in_the_template_and_in_a_config_without_the_key() {
        let ids = ["claude".to_string(), "codex".to_string()];
        let t = set_agents_enabled(&default_toml(), &ids);
        assert!(t.contains("enabled = [\"claude\", \"codex\"]    # the agents eye follows;"));
        assert_eq!(t.lines().count(), default_toml().lines().count());
        assert_eq!(
            Config::parse(&t).unwrap().enabled_agent_ids(),
            ["claude", "codex"]
        );
        // only that line changes
        let back = set_agents_enabled(&t, &ids[..1]);
        assert!(back.lines().skip(2).eq(default_toml().lines().skip(2)));
        // an old file with an [agents] section and no key, and one with no section
        let t = set_agents_enabled("[agents.claude]\ndir = \"/c\"\n", &ids[1..]);
        assert_eq!(
            t,
            "[agents.claude]\ndir = \"/c\"\n\n[agents]\nenabled = [\"codex\"]\n"
        );
        let t = set_agents_enabled(
            "# mine\n[agents]\n# nothing\n[board]\nport = 1\n",
            &ids[..1],
        );
        assert_eq!(
            t,
            "# mine\n[agents]\nenabled = [\"claude\"]\n# nothing\n[board]\nport = 1\n"
        );
        let c = Config::parse(&t).unwrap();
        assert_eq!(
            (c.enabled_agent_ids(), c.board.port),
            (vec!["claude".to_string()], 1)
        );
        assert!(Config::parse(&set_agents_enabled("", &[]))
            .unwrap()
            .enabled_agent_ids()
            .is_empty());
    }

    #[test]
    fn set_enabled_key_missing() {
        let t = set_board_enabled(
            "# top\n[board]\nport = 1  # mine\n[exclude]\npaths = []\n",
            true,
        );
        assert_eq!(
            t,
            "# top\n[board]\nenabled = true\nport = 1  # mine\n[exclude]\npaths = []\n"
        );
    }

    #[test]
    fn set_enabled_section_missing() {
        let t = set_board_enabled("[summary]\nenabled = false\n# end", true);
        assert_eq!(
            t,
            "[summary]\nenabled = false\n# end\n\n[board]\nenabled = true\n"
        );
        let c = Config::parse(&t).unwrap();
        assert!(c.board.enabled && !c.summary.enabled);
        assert_eq!(set_board_enabled("", false), "[board]\nenabled = false\n");
    }

    #[test]
    fn set_enabled_only_touches_the_board_section() {
        let src = "[summary]\nenabled = true\n[board]\n  enabled=false\n[hide]\nenabled = 1\n";
        assert_eq!(
            set_board_enabled(src, true),
            "[summary]\nenabled = true\n[board]\n  enabled = true\n[hide]\nenabled = 1\n"
        );
    }
    #[test]
    fn done_days_defaults_to_seven() {
        assert_eq!(Config::default().board.done_days, 7);
        assert_eq!(Config::parse(&default_toml()).unwrap().board.done_days, 7);
        assert_eq!(
            Config::parse("[board]\ndone_days = 0")
                .unwrap()
                .board
                .done_days,
            0
        );
    }

    #[test]
    fn the_user_name_is_rewritten_to_you() {
        let u = Config::parse("[user]\nname = \"Sam\"").unwrap().user;
        assert_eq!(u.as_you("Sam: review the PR"), "You: review the PR");
        assert_eq!(u.as_you("sam:review"), "You: review");
        assert_eq!(u.as_you("Samantha: review"), "Samantha: review");
        assert_eq!(u.as_you("Claude: fix it"), "Claude: fix it");
        assert_eq!(u.as_you("You: already"), "You: already");
        assert_eq!(u.without_you("Sam: review"), "review");
        assert_eq!(u.without_you("You: review"), "review");
        assert_eq!(u.without_you("Claude: fix"), "Claude: fix");
        // no name configured: nothing changes
        let none = Config::default().user;
        assert_eq!(none.as_you("Sam: review"), "Sam: review");
        assert_eq!(none.without_you("You: x"), "x");
        assert!(Config::parse(&default_toml()).unwrap().user.name.is_empty());
    }

    #[test]
    fn agents_default_to_claude_and_read_the_config() {
        let c = Config::default();
        assert_eq!(c.enabled_agent_ids(), ["claude"]);
        assert_eq!(
            Config::parse(&default_toml()).unwrap().enabled_agent_ids(),
            ["claude"]
        );
        let c = Config::parse(
            "[agents]\nenabled = [\"fake\", \"claude\"]\n[agents.claude]\ndir = \"~/c\"\n[agents.fake]\ndir = \"/f\"",
        )
        .unwrap();
        assert_eq!(c.enabled_agent_ids(), ["fake", "claude"]);
        assert_eq!(c.agent_cfg("fake").dir.as_deref(), Some("/f"));
        assert_eq!(c.agent_cfg("claude").dir.as_deref(), Some("~/c"));
        assert!(c.agent_cfg("other").dir.is_none());
        // an empty list turns every agent off
        let off = Config::parse("[agents]\nenabled = []").unwrap();
        assert!(off.enabled_agent_ids().is_empty());
        // a table without `enabled` keeps the default list
        let only_dir = Config::parse("[agents.claude]\ndir = \"/x\"").unwrap();
        assert_eq!(only_dir.enabled_agent_ids(), ["claude"]);
        assert_eq!(Config::default().summary.agent, "");
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~/c"), PathBuf::from(&home).join("c"));
        assert_eq!(expand_tilde("~"), PathBuf::from(&home));
        assert_eq!(expand_tilde("/a/~b"), PathBuf::from("/a/~b"));
    }

    #[test]
    fn defaults_parse() {
        let c = Config::parse(&default_toml()).unwrap();
        assert_eq!(c.board.port, 7420);
        assert_eq!(c.summary.model, "", "the agent's own default");
        assert!(!c.is_excluded("/x"));
        let c = Config::parse("[exclude]\npaths=[\"/x\"]").unwrap();
        assert!(c.is_excluded("/x/y"));
        assert!(c.summary.enabled);
        assert!(c.hide.trivial);
        assert_eq!(c.hide.max_reply_chars, 200);
        assert!(c.hide.commands.contains(&"cd".to_string()));
        let d = Config::default();
        assert_eq!(d.hide.commands, c.hide.commands);
        let c = Config::parse("[hide]\ntrivial=false").unwrap();
        assert!(!c.hide.trivial && c.hide.max_prompts == 1);
    }
}
