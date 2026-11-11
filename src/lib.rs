/// Coding agents behind one trait; Claude Code is the first adapter.
pub mod agents;
/// Compressed copies of transcripts, and restoring them.
pub mod archive;
/// Snapshots of the database, restoring them, and the optional mirror.
pub mod backup;
/// The command line.
pub mod cli;
/// Which board column a session is in.
pub mod columns;
/// The settings in `config.toml`.
pub mod config;
/// A session's conversation as messages, from its transcript or archive copy.
pub mod conversation;
/// The background process: hook events, scans, summaries, archive, board.
pub mod daemon;
/// The SQLite database.
pub mod db;
/// Safe edits of files other programs own, and private files of eye's own.
pub mod files;
/// Whether an idea's folder exists and may be created.
pub mod folder;
/// Sessions that are left out of lists.
pub mod hide;
/// The hook entry point.
pub mod hook;
/// Parked ideas and starting them.
pub mod ideas;
/// Install, uninstall and doctor.
pub mod install;
/// Putting a session in a column by hand.
pub mod mark;
/// The data types: sessions, streams, outputs, ideas, columns.
pub mod model;
/// Where eye keeps its data.
pub mod paths;
/// The picker's index, ranking and launching.
pub mod pick;
/// What differs between macOS and Linux: the background service, opening URLs, the clipboard.
pub mod platform;
/// "Check on this" dates for sessions and ideas: the time parser, the rule and the notice.
pub mod remind;
/// What install, uninstall and board on|off print.
pub mod report;
/// Opening a command in an iTerm2 tab (macOS) or copying it.
pub mod resume;
/// Reading the agents' files into the database.
pub mod scan;
/// Full-text search and ranking.
pub mod search;
/// The board's JSON API and web files.
pub mod server;
/// Summaries of where each session stands.
pub mod summarize;
/// The terminal picker screen.
pub mod tui;
/// Small helpers.
pub mod util;
