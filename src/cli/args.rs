//! The command line: every subcommand and flag.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Index, search and resume coding-agent sessions",
    after_help = "Run with no arguments to open the interactive picker: type to search, Enter to resume.\nPut `eval \"$(<this-command> init zsh)\"` in ~/.zshrc for the Ctrl-G key (see docs/terminal.md)."
)]
pub struct Cli {
    /// Claude Code's folder for this run (default ~/.claude; also `[agents.claude] dir`)
    #[arg(long, global = true, env = "EYE_CLAUDE_DIR", hide = true)]
    pub claude_dir: Option<PathBuf>,
    /// eye home (default ~/.agent-eye)
    #[arg(long, global = true, env = "EYE_HOME")]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Args)]
pub struct PickArgs {
    /// Start the search with these words
    pub words: Vec<String>,
    /// Print the command (`cd <dir> && <agent> --resume <id>`) instead of running it
    #[arg(long)]
    pub print: bool,
    /// No screen: print the ranked list for these words
    #[arg(long)]
    pub filter: Option<String>,
    /// No screen: take the first hit (with --print, print its command; without, run it)
    #[arg(long)]
    pub first: bool,
    /// Include hidden sessions and the archive
    #[arg(long)]
    pub all: bool,
    /// Rows to print without a screen
    #[arg(long, default_value_t = 30)]
    pub limit: usize,
    /// Skip the quick rescan before searching
    #[arg(long)]
    pub no_scan: bool,
    /// With --first: fork the session (resume it as a new one that branches from it)
    #[arg(long)]
    pub fork: bool,
}

#[derive(Subcommand)]
pub enum IdeaCmd {
    /// Park an idea in the Ideas column
    Add {
        title: String,
        /// Your notes; never sent to the agent
        #[arg(long)]
        notes: Option<String>,
        /// First message the agent gets when the idea starts (none: the agent opens and waits for you)
        #[arg(long)]
        prompt: Option<String>,
        /// Folder to start it in (default: the current folder)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Session this idea came from (id or prefix)
        #[arg(long)]
        from_session: Option<String>,
        /// Agent to start it in (default: the first enabled one)
        #[arg(long)]
        agent: Option<String>,
    },
    /// Change an idea; an empty value clears notes or prompt
    Edit {
        id: i64,
        #[arg(long)]
        title: Option<String>,
        /// Your notes; never sent to the agent
        #[arg(long)]
        notes: Option<String>,
        /// First message the agent gets when the idea starts
        #[arg(long)]
        prompt: Option<String>,
        /// Folder to start it in
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Agent to start it in (an empty value goes back to the default)
        #[arg(long)]
        agent: Option<String>,
    },
    /// List ideas
    Ls {
        #[arg(long)]
        json: bool,
    },
    /// Start an idea in a new session
    Start {
        id: i64,
        /// Run in the background
        #[arg(long)]
        bg: bool,
        /// Make the idea's folder first when it does not exist (without it, a terminal asks)
        #[arg(long)]
        create_folder: bool,
        /// Show what would run, start nothing
        #[arg(long)]
        dry_run: bool,
    },
}

/// Where `install` and `uninstall` work, and whether they change anything.
#[derive(Args)]
pub struct SetupArgs {
    /// Show what would change, write nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Show the changes to each file (a dry run always does)
    #[arg(long)]
    pub verbose: bool,
    /// Where the binary goes (default ~/.local/bin)
    #[arg(long)]
    pub bin_dir: Option<PathBuf>,
    /// Where the service file goes (default ~/Library/LaunchAgents on macOS, ~/.config/systemd/user on Linux)
    #[arg(long, alias = "launch-agents-dir")]
    pub service_dir: Option<PathBuf>,
    /// Never run launchctl or systemctl; print the commands instead
    #[arg(long, alias = "no-launchctl")]
    pub no_service: bool,
}

#[derive(Subcommand)]
pub enum BackupCmd {
    /// List the snapshots, newest first
    Ls,
    /// Put a snapshot back as the database (`latest`, or a name from `eye backup ls`)
    Restore {
        name: String,
        /// Do not ask first (needed in a script)
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum BoardCmd {
    /// Turn the web board on and restart the daemon
    On {
        #[arg(long)]
        port: Option<u16>,
        /// Open it in the browser
        #[arg(long)]
        open: bool,
        /// Do not restart the daemon through launchctl or systemctl
        #[arg(long, alias = "no-launchctl")]
        no_service: bool,
    },
    /// Turn the web board off and restart the daemon
    Off {
        /// Do not restart the daemon through launchctl or systemctl
        #[arg(long, alias = "no-launchctl")]
        no_service: bool,
    },
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Interactive picker: type to search, Enter to resume (also what running with no arguments does)
    Pick(PickArgs),
    /// Print the shell widget (Ctrl-G picker): eval "$(<this-command> init zsh)"
    Init {
        /// zsh or bash
        shell: String,
        /// Key that opens the picker
        #[arg(long, default_value = "ctrl-g")]
        key: String,
    },
    /// Index transcripts, history, jobs and live sessions
    Scan {
        /// Re-read everything from the start
        #[arg(long)]
        full: bool,
        #[arg(long)]
        json: bool,
    },
    /// Search sessions
    Find {
        #[arg(required = true)]
        words: Vec<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Include archived and hidden sessions
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// List sessions
    Ls {
        #[arg(long)]
        column: Option<String>,
        /// Include hidden (trivial) sessions
        #[arg(long)]
        all: bool,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Hide a session from the board, archive, find and ls (until `eye unhide`)
    Hide { id: String },
    /// Show a hidden session again, and keep it shown even if it looks trivial
    Unhide { id: String },
    /// Show one session
    Show {
        id: String,
        #[arg(long)]
        json: bool,
        /// Print the conversation (your prompts and the agent's replies) instead
        #[arg(long)]
        conversation: bool,
        /// With --conversation: how many of the latest messages
        #[arg(short = 'n', default_value_t = 10)]
        n: usize,
    },
    /// Resume a session in a new terminal tab or window, or copy its command (`[board] resume`)
    Resume {
        id: String,
        /// Only print the command
        #[arg(long)]
        dry_run: bool,
        /// Branch a new session from this one (the original stays untouched)
        #[arg(long)]
        fork: bool,
    },
    /// Ask the summary agent where a session stands (status, state, next step, topics)
    Summarize {
        ids: Vec<String>,
        #[arg(long)]
        all: bool,
        /// Show what would run, call nothing
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        max: Option<usize>,
        /// Ignore the unchanged-input check
        #[arg(long)]
        force: bool,
        /// Print the masked excerpt that is sent
        #[arg(long)]
        print_excerpt: bool,
        #[arg(long)]
        json: bool,
    },
    /// Run the board on 127.0.0.1
    Serve {
        /// Port (default from config.toml, 7420)
        #[arg(long)]
        port: Option<u16>,
        /// Open the board in the browser once it is up
        #[arg(long)]
        open: bool,
    },
    /// Print the web board URL with its token (the board is optional: `board on` / `board off`)
    Board {
        #[command(subcommand)]
        sub: Option<BoardCmd>,
        #[arg(long)]
        port: Option<u16>,
        /// Open it in the browser
        #[arg(long)]
        open: bool,
    },
    /// Ideas: add, edit, list, start
    #[command(subcommand)]
    Idea(IdeaCmd),
    /// Put a session in a column: mark <id> done | mark --current done
    Mark {
        /// `<id> <status>`, or just `<status>` with --current
        #[arg(required = true, num_args = 1..=2)]
        args: Vec<String>,
        /// The session this command runs in
        #[arg(long)]
        current: bool,
        #[arg(long)]
        next: Option<String>,
    },
    /// Check on a session or an idea later: remind <id> <when>, remind <id> --clear, remind ls
    Remind {
        /// `<id> <when>`, or `<id> --clear`, or `ls`. The id is a session id (or its first 6
        /// characters) or `idea:<n>`; `<when>` is tomorrow, mon..sun, 30m, 4h, 3d, 1w, 2026-11-20
        /// or "2026-11-20 14:00" (a day alone means 9:00)
        #[arg(required = true, num_args = 1..)]
        args: Vec<String>,
        /// What to check on, shown with the reminder
        #[arg(long)]
        note: Option<String>,
        /// Remove the reminder
        #[arg(long)]
        clear: bool,
    },
    /// Hook entry point (called by the agent, reads the event on stdin)
    Hook {
        event: String,
        /// Which agent calls (default: Claude Code, the form installed before there were several)
        #[arg(long)]
        agent: Option<String>,
    },
    /// Background process: hooks, live status, summaries, archive, board
    Daemon {
        #[arg(long)]
        port: Option<u16>,
        /// Do not serve the board
        #[arg(long)]
        no_board: bool,
    },
    /// Install the binary, the agents' hooks and skill, and the background service
    Install {
        #[command(flatten)]
        a: SetupArgs,
        /// Also turn the optional web board on
        #[arg(long)]
        board: bool,
    },
    /// Remove what install added
    Uninstall {
        #[command(flatten)]
        a: SetupArgs,
        /// Also delete eye's own data (index, archive, config)
        #[arg(long)]
        purge: bool,
    },
    /// Check that everything works
    Doctor,
    /// Keep a compressed copy of transcripts
    Archive {
        id: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Put an archived transcript back where its agent expects it
    Restore { id: String },
    /// Snapshot the database now (`eye backup ls` lists them, `eye backup restore` puts one back)
    Backup {
        #[command(subcommand)]
        sub: Option<BackupCmd>,
    },
}

impl Default for PickArgs {
    fn default() -> Self {
        PickArgs {
            words: vec![],
            print: false,
            filter: None,
            first: false,
            all: false,
            limit: 30,
            no_scan: false,
            fork: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parses(args: &[&str]) -> bool {
        Cli::try_parse_from(std::iter::once("eye").chain(args.iter().copied())).is_ok()
    }

    fn help(name: &str) -> String {
        Cli::command()
            .find_subcommand(name)
            .unwrap()
            .clone()
            .render_long_help()
            .to_string()
    }

    #[test]
    fn the_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn install_and_uninstall_each_list_only_their_own_flags() {
        assert!(parses(&["install", "--board", "--dry-run"]));
        assert!(parses(&[
            "uninstall",
            "--purge",
            "--dry-run",
            "--no-service"
        ]));
        assert!(!parses(&["uninstall", "--board"]));
        assert!(!parses(&["install", "--purge"]));
        let (install, uninstall) = (help("install"), help("uninstall"));
        assert!(install.contains("--board") && !install.contains("--purge"));
        assert!(uninstall.contains("--purge") && !uninstall.contains("--board"));
        assert!(!uninstall.contains("(install)"));
    }

    #[test]
    fn the_old_service_flags_still_work_and_are_not_advertised() {
        assert!(parses(&["install", "--no-launchctl"]));
        assert!(parses(&["uninstall", "--launch-agents-dir", "/tmp/x"]));
        assert!(parses(&["board", "on", "--no-launchctl"]));
        assert!(parses(&["board", "off", "--no-launchctl"]));
        assert!(parses(&[
            "install",
            "--service-dir",
            "/tmp/x",
            "--no-service"
        ]));
        for name in ["install", "uninstall"] {
            let h = help(name);
            assert!(
                h.contains("--no-service") && h.contains("--service-dir"),
                "{h}"
            );
            assert!(
                !h.contains("launchctl --") && !h.contains("--no-launchctl"),
                "{h}"
            );
            assert!(!h.contains("--launch-agents-dir"), "{h}");
        }
    }
}
