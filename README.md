<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/banner-dark.svg">
    <img src="assets/banner-light.svg" alt="agent-eye" width="720">
  </picture>
</p>

<p align="center">
  <b>Find, sort and resume your coding-agent sessions.</b><br>
  A local index, a terminal picker and an optional pixel-art board. macOS and Linux.
</p>

<p align="center">
  <a href="https://github.com/maxipavlovic/agent-eye/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/maxipavlovic/agent-eye/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://sonarcloud.io/summary/new_code?id=maxipavlovic_agent-eye"><img alt="Quality gate" src="https://sonarcloud.io/api/project_badges/measure?project=maxipavlovic_agent-eye&metric=alert_status"></a>
  <a href="https://sonarcloud.io/summary/new_code?id=maxipavlovic_agent-eye"><img alt="Coverage" src="https://sonarcloud.io/api/project_badges/measure?project=maxipavlovic_agent-eye&metric=coverage"></a>
  <a href="https://github.com/maxipavlovic/agent-eye/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/maxipavlovic/agent-eye"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <img alt="Platform: macOS | Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey">
</p>

<p align="center">
  <img src="assets/picker-demo.gif" alt="The terminal picker: search by words, preview a session, read its conversation, press Ctrl-G to put the resume command on the prompt" width="800">
</p>

## Why

- **Sessions disappear.** Claude Code deletes transcripts after 30 days by default. agent-eye keeps a compressed copy and puts it back when you resume.
- **"Which session was that?"** Search titles, prompts, replies, PR links and folders in one place. Ctrl-G puts the resume command on your prompt.
- **"What needs me now?"** Live status from hooks plus a short summary of each session: where it stands, the next step, who acts.

## Features

<table>
<tr><td width="48"><img src="assets/icon-search.svg" width="32" alt=""></td><td><b>Search that finds it.</b> Words match titles, prompts, replies, PR links, ticket keys and folders, with fuzzy matching on titles. Recent and active sessions come first.</td></tr>
<tr><td><img src="assets/icon-key.svg" width="32" alt=""></td><td><b>Resume from your prompt.</b> Press Ctrl-G, type a few words, press Enter: the resume command lands on your command line. Ctrl-F forks the session instead.</td></tr>
<tr><td><img src="assets/icon-keep.svg" width="32" alt=""></td><td><b>Sessions you keep.</b> A compressed copy of every transcript. When the agent has deleted the original, resume restores the copy first.</td></tr>
<tr><td><img src="assets/icon-status.svg" width="32" alt=""></td><td><b>Know what needs you.</b> Each session gets a state, a next step and an owner (you, the agent, or someone else), so you see what is blocked on you without opening it.</td></tr>
<tr><td><img src="assets/icon-board.svg" width="32" alt=""></td><td><b>A board, if you want one.</b> Needs you, Running, Waiting on others, Paused, Done and Ideas, in your browser. Off until you turn it on.</td></tr>
<tr><td><img src="assets/icon-local.svg" width="32" alt=""></td><td><b>Local only.</b> No account and no telemetry. The index lives in <code>~/.agent-eye</code> on your machine.</td></tr>
</table>

## Requirements

- **macOS 11 or newer** (Apple silicon or Intel), or **Linux** x86_64/aarch64 with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later). Tested on macOS 27, and on Linux in Debian 12 and 13 containers (including systemd).
- **At least one agent:** [Claude Code](https://github.com/anthropics/claude-code) (tested with 2.1.311) or [Codex CLI](https://github.com/openai/codex) (tested with 0.167). Older versions may lack the hooks or the headless options eye uses; `agent-eye doctor` says what is missing.
- For the shell key, zsh or bash. For the board, a current browser (installing it as an app needs Chrome or Edge).
- On Linux, a systemd user session for the background process (or run `agent-eye daemon` yourself), and `wl-copy`, `xclip` or `xsel` for copying.

## Install

```sh
brew install maxipavlovic/tap/agent-eye
```

(Homebrew works on macOS and Linux) or with the installer script, or from source (Rust 1.89 or newer; the crate includes the built board, so no Node is needed):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/maxipavlovic/agent-eye/releases/latest/download/agent-eye-installer.sh | sh
cargo install agent-eye
```

Then run it once:

```sh
agent-eye install        # add --dry-run to see what it would change
```

This sets up the hooks, the background process and the short command `eye`, for the agents it finds on your machine (Claude Code, Codex CLI, or both). Codex runs hooks only after you trust them once: start Codex and approve eye's hooks with `/hooks`. The background process is a launch agent on macOS and a systemd user service on Linux ([more on Linux](#linux)). If your shell cannot find `eye`, add `~/.local/bin` to your `PATH` (`agent-eye doctor` says so). After an upgrade, run `agent-eye install` again to refresh the installed copy. Releases are built on GitHub with attestations: `gh attestation verify <file> --repo maxipavlovic/agent-eye`.

## Quick start

```sh
eye                       # the picker: type to search, Enter resumes
eye find webhook retry    # search without a screen
eval "$(eye init zsh)"    # in ~/.zshrc: Ctrl-G opens the picker from your prompt
eye board on --open       # turn the optional board on and open it
```

In the picker, Enter resumes, Ctrl-F forks, Ctrl-T shows the conversation, Ctrl-Y copies the command, Ctrl-X hides a session and Esc quits. All keys are in [docs/terminal.md](docs/terminal.md).

## The board

<p align="center">
  <a href="assets/board-demo.gif">
    <img src="assets/board-light.png" alt="The board with the columns Needs you, Running, Waiting on others, Paused, Done and Ideas" width="860">
  </a>
</p>

One card per session. Cards move by themselves as sessions change. Move one by hand (right-click or long-press it, or use the buttons and the keys N, W, P, D in its details) and it stays where you put it until the session has new activity. Park an idea as a card and start it later as a new session. Press `/` or ⌘K to search. The board is off by default and listens on `127.0.0.1` only.

The board can also be installed as an app: open it in Chrome or Edge and choose **Install agent-eye** in the address bar. It gets its own window and an icon in the Dock or the app menu, and the window title shows how many sessions need you. This works on Linux too.

## Linux

agent-eye works the same on Linux, with these differences:

- **The background process is a systemd user service.** `agent-eye install` writes `~/.config/systemd/user/agent-eye.service` and starts it with `systemctl --user`; its log is `journalctl --user -u agent-eye`. It runs while you are logged in; `loginctl enable-linger $USER` keeps it running after you log out. Where there is no systemd, run `agent-eye daemon` yourself (`install --no-service` skips the service).
- **Resume and Start copy the command** instead of opening a terminal tab (`[board] resume = "copy"` is the default). In the picker, Enter and Ctrl-G run the command in your current terminal as usual.
- **Copying** uses `wl-copy`, `xclip` or `xsel`, the first that works. With none, the command is printed instead. The background process copies for the board, and it may have no display; then the board shows the command.
- `eye board on --open` opens the board with `xdg-open`.

## Privacy and safety

- **Local only.** There is no telemetry. The only network use is the summary call below.
- **Summaries use your agent's plan.** After a session goes quiet, agent-eye runs an agent's own CLI in headless mode (`claude -p` or `codex exec`) on a short excerpt: the title, the first prompt, the last messages and one line per tool call. Obvious secrets are masked, best effort. This counts against the plan of the agent you choose (`agent` under `[summary]`; the first enabled agent by default) like any other call. Turn it off with `enabled = false` under `[summary]` in `~/.agent-eye/config.toml`.
- **What is stored.** Its own data lives in `~/.agent-eye`: the index, compressed copies of transcripts, `config.toml` and the board token. Your agent's files are only read, never changed (except the hooks that `install` adds, below).
- **What `install` changes.** It copies the binary to `~/.local/bin` and links `eye` beside it, adds eye's hooks to `~/.claude/settings.json` and `~/.codex/hooks.json` for the agents it finds (after a backup; other hooks and Codex's `notify` setting stay untouched), registers the background service (a launch agent or a systemd user unit), and adds a `/eye` skill. `agent-eye uninstall` reverts all of it; `--purge` also deletes the stored data.
- **The board** needs a token, checks the `Host` and `Origin` headers and binds to `127.0.0.1`.

## Works with

- **Claude Code**, the default.
- **Codex CLI**: sessions, search, resume and fork, live status, summaries and the board. Codex has no background sessions, so an idea starts in the foreground, and agent-eye links it to the session that begins in its folder.

`agent-eye install` follows the agents it finds on your machine and writes them to `[agents] enabled` in `config.toml`. The code has an agent layer, so other coding agents can be added ([adding an agent](docs/agents.md)).

agent-eye is unofficial and not affiliated with Anthropic or OpenAI. It reads the agents' own files, and those are not a public interface: an update can change them. `agent-eye doctor` checks that everything still works.

## Docs

- Site: <https://maxipavlovic.github.io/agent-eye/>
- [Terminal picker and shell key](docs/terminal.md), [configuration](docs/config.md), [board API](docs/api.md)
- [Security and what is stored](docs/security.md) (also how to report a problem), [architecture](docs/architecture.md), [adding an agent](docs/agents.md)
- [Changelog](CHANGELOG.md)

## License

[MIT](LICENSE). The board embeds three fonts under the SIL Open Font License; see [licenses/](licenses/).
