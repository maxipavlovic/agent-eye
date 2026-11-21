# Night shift

Park work during the day, let it run at night inside the plan you already pay for, and find results to review in the morning. It is **off until you switch it on**.

## The idea

- You plan a run for tonight: an **idea** (it needs a folder and a prompt) or a **session to continue** (it needs a prompt: what it should do tonight).
- Inside the window (01:00-07:00 by default) the daemon starts planned runs, headless, when no session you work in is live.
- Every run works in its own git worktree on a new branch, or in a separate copy of your folder (`[night] isolation`, see "Two ways to keep your folder safe"). It never runs in your folder. A sandboxed Claude run cannot reach a git host (see "The barrier" below), and a Codex run has no network.
- When a run ends, its session is a **Needs you** card with the next step "review the night run": status, report, what it changed, and its loose ends.

## Plan, from the command line

```
eye night on | off
eye night add idea:<n> [--limit 30%|2M|90m] [--agent claude|codex|agy] [--switch] [--isolation worktree|copy]
eye night add <session> --prompt "what it should do tonight" [--limit ...]
eye night ls                      # the default
eye night cancel <run>
eye night log <run>
eye night window 01:00-07:00      # local time; 23:00-05:00 crosses midnight
eye night parallel 2
eye night run-now <run> --yes [--wait]
eye night bring <run> [--yes]         # a repo copy: its branch into your repo
eye night discard <run> [--yes] [--lose-changes]
```

Planning after the window closes means tonight; planning inside the window means the night that is going on. A run planned for a night that has passed (the daemon was off) starts in the next window.

`run-now` starts a planned run at once, ignoring the window and the quiet rule, to try it out. Without `--wait` the daemon finishes it; with `--wait` eye stays, watches it and prints the report, so it works without a daemon.

On the board: a **TONIGHT** button on idea cards and a TONIGHT box in the idea form (with the optional limit, agent, switch and **Work in**), **CONTINUE TONIGHT** in a session's panel (prompt and optional limit), and a calm **Tonight: 2** chip in the header while runs are planned. The chip opens a list: switch on or off, window, how many at once, and for each run CHANGE LIMIT and CANCEL RUN. A finished run's session has a **night** chip, and its panel a **NIGHT RUN** section: status, start, end, time taken, tokens, the limit and whether it stopped the run, the report, the diff stat, how it worked (a worktree or a separate copy), the branch or folder, and RESUME (interactive, in the worktree), OPEN FOLDER, DISCARD and, for the copy of a repo, **BRING INTO MY REPO** (each asks first in the page). In the picker the row says `night` and the preview has the status line and the first line of the report.

## When a run starts

All of these, checked once a minute:

1. `[night] enabled = true` and the run is planned.
2. The clock is inside the window, and the run's night has come.
3. **Quiet:** no live interactive session that is not itself a night run has been busy or touched for `[night] quiet_minutes` (20). A headless run (`claude -p`, `codex exec`) does not count as a session you work in; neither does anything working inside eye's night folder.
4. Fewer than `[night] parallel` (2) runs are going. Planned runs never block each other: two planned for tonight both run, in parallel, each in its own worktree or copy. A third starts when one ends.

A run that is going at the end of the window may finish; no new one starts.

## Limits

No budget cap by default. Each planned run can set **one** optional limit:

| Limit | Looks like | Stops the run when |
|---|---|---|
| weekly plan | `30%` | the agent's weekly window is above that (from `eye usage` limit data, `limits:<agent>`; a window that has reset since is ignored) |
| tokens | `2M`, `500k` | the run's session used that many input, output and cache-write tokens, sub-agents included (cache reads are not counted) |
| time | `90m`, `2h` | that long since it started |

A limit that eye cannot check is **refused when you plan it**, with the reason: a weekly limit for Claude needs `eye statusline` to be set up (see `usage.md`), and one for Codex needs a Codex session to have reported its limits.

When a limit is passed the run gets SIGTERM to its process group (after the same pid and start time check `eye loose` makes, never SIGKILL) and is marked `stopped` with why, for example `weekly limit 80% reached (81% used)`. It is finished as soon as its process is gone.

### Switching agents

With `--switch` an idea run starts again on the other enabled agent, with the same prompt in a fresh worktree or copy, when its agent's plan is out: a limit window at 100% when it is about to start, or the CLI exits with a rate-limit message. Not for continued sessions, which stay on their agent. A weekly limit is dropped when the run moves, because it was about the other plan.

## Two ways to keep your folder safe

`[night] isolation` picks how a run works; each plan can choose its own (`--isolation`, or **Work in** in the TONIGHT and CONTINUE TONIGHT forms, which start on the config value). The run remembers which one it used, and the NIGHT RUN section shows it.

- **`worktree` (the default).** A git worktree of your repo on a new `night/...` branch. It is fast and small, and the barrier below keeps the run out of your folder: every existing file and folder is locked, and `git push` and the network are closed. One gap, in plain words: your repo's `.git` folder has to stay writable for the worktree's commits, and Claude treats the main checkout of a worktree as part of the run's own area. So a shell command that builds a path in an unusual way can still add a *new* file at the top of your repo folder. It cannot change anything that is already there.
- **`copy`.** A separate local clone of your repo (`git clone --local`, objects are hardlinked, so it is quick and small), checked out on the night branch at the commit your repo had when you planned, with its `origin` removed so nothing points back. Your own folder is locked completely, `.git` included, and the run may write only inside the copy. Nothing in your folder can change, not even a new file. In the morning, **BRING INTO MY REPO** (or `eye night bring <run>`) fetches the night branch into your repo as a new branch, after you confirm; it refuses if your repo already has a branch of that name, and it never touches the branch you have checked out. DISCARD removes the copy.

Pick `copy` when the repo matters more than speed or disk, when the session you continue remembers many absolute paths, or when you want the guarantee rather than the net. A plain folder (not a repo) is always a copy.

## How runs are made

Each run has a folder `<eye home>/night/<run id>/` with `log` (stdout and stderr) and `exit` (the exit code), and the worktree or copy inside it.

- **Repo, mode `copy`:** `git clone --local --no-checkout <repo> <run dir>/work`, then `git checkout -b night/<YYYYMMDD>-<slug> <base>` and `git remote remove origin`; the base is your repo's HEAD at planning time. A sub-folder run starts in the same sub-folder of the copy.
- **Repo, mode `worktree`:** `git worktree add <run dir>/wt -b night/<YYYYMMDD>-<slug> HEAD`. The base is the current HEAD and is recorded. A second run with the same slug gets `-<run id>`. A run planned for a sub-folder starts in the same sub-folder of its worktree.
- **Plain folder:** copied into `<run dir>/work`, leaving out `node_modules`, `.git`, `target`, `.venv`, `dist` and symlinks. Over 100 MB (not counting those) is refused when you plan it.
- **Claimed:** before anything is made, the daemon sets the run to `starting` (only from `planned`), so two starters or a cancel cannot start it twice; `eye night cancel` works on a planned run only. A run whose process start time cannot be read is set to `lost` (its pid and folder are kept, eye never signals it); a run stuck in `starting` for 30 minutes is set to `failed`.
- **Detached:** every run is started as `sh -c '"$@"; echo $? > exit'` in its own session, so the daemon finds it again by pid and start time after a restart or an upgrade of its own binary.

### Claude Code

`claude -p <prompt> --session-id <uuid> --permission-mode auto --disallowedTools ...`, in the worktree or copy. The transcript is kept, so it is a normal session that eye indexes. Headless permission modes the installed CLI offers (`claude --help`): `acceptEdits`, `auto`, `bypassPermissions`, `manual`, `dontAsk`, `plan`. eye uses `auto`. Set `[night] claude_permission = "acceptEdits"` to use that instead; eye then also passes `--allowedTools` for git status/diff/log/add/commit and these commands only: `npm test`, `npm run`, `npm ci`, `npm install`, `cargo build|test|check|fmt|clippy`, `python -m pytest`, `pytest`, `go test|build`, `make test`, `node`, `ls`. No bare `npm *`, `cargo *`, `npx *` or `python*`. `bypassPermissions` is never chosen by eye.

Always denied by rule, in either mode: `git push`, `git -C ... push`, `git remote`, `gh` (all), `ssh`, `scp`, `npm publish`, `cargo publish`, `twine upload`, `gem push`, `docker push`. A rule matches the text of the command, so `git -c x=y push` or `sh -c 'git push'` is not matched by it; the network allowlist below is what stops those.

#### The barrier around your own folders

Every Claude run also gets `--settings '<json>'` (made per run, see `src/night/barrier.rs`), so that a session that remembers the absolute paths of your folder cannot write there:

- **Sandbox for Bash.** `sandbox.enabled`, `failIfUnavailable` (Claude refuses to start rather than run unsandboxed) and `allowUnsandboxedCommands = false` (no retry outside the sandbox). Claude's sandbox allows writes in the working directory and the temp folder; eye adds the git folders a commit in a worktree writes: `objects`, `refs/heads/night`, `logs/refs/heads/night` and the worktree's own folder under `.git/worktrees`.
- **Network for Bash.** `sandbox.network.allowedDomains` is the package registries `registry.npmjs.org`, `pypi.org`, `files.pythonhosted.org`, `crates.io`, `index.crates.io`, `static.crates.io` and `proxy.golang.org`, and nothing else; `[night] network = [...]` replaces that list. `deniedDomains` names `github.com`, `gitlab.com` and `bitbucket.org`, and `strictAllowlist = true` denies every other host instead of asking. Claude's docs: "Connections go through a proxy on your machine that checks each host against your allowed domains"; plain `ssh` "can't connect, even to an allowed host". So a `git push` over HTTPS or ssh to a host that is not listed fails, however the command is written. Only while the sandbox is on: with `allow_unsandboxed` Bash has no network limit.
- **Deny rules.** `sandbox.filesystem.denyWrite` and `Edit(//<path>/**)` rules for the planned folder, for the whole repo when the run started in a sub-folder, and for the continued session's original working folder. A deny rule is checked before any allow rule in every settings scope, so an allow rule or `additionalDirectories` in your own settings cannot open these folders for a night run. The Edit rules cover the Edit, Write and NotebookEdit tools.
- **Mode `copy`:** the sandbox may write only in the copy; `denyWrite` and the `Edit` rules cover the whole original repo (and a continued session's original folder), `.git` included.
- **In a repo (mode `worktree`)** the original `.git` must stay writable for the worktree's commits, and Claude's sandbox lets a deny rule beat an allow rule on the same path, so the folder cannot be denied whole. eye denies every entry of it except `.git`, and inside `.git` the `hooks` folder and the `config`, `HEAD` and `index` files. A plain folder (a copy run) is denied whole.
- **Two hooks** (`PreToolUse`) cover what no rule names, a *new* file directly in the original folder: the file tools are refused when `file_path` is in a guarded folder or climbs out with `..`, and Bash is refused when the command contains the folder's path. The hooks are a net for the mistake this exists for, a remembered absolute path. A command that builds the path some other way gets past the hook and, in a repo, can still create a new entry directly in the original folder; it cannot change anything that exists there.
- **No sandbox, no run.** macOS has the sandbox built in. On Linux it needs `bubblewrap` (`bwrap`) and `socat` on PATH (`apt-get install bubblewrap socat`); elsewhere it is not available. A Claude run that cannot get it fails at start with the reason and nothing is created. `[night] allow_unsandboxed = true` starts it anyway: the Edit rules and the hooks stay, Bash is then not confined. Default `false`.
- **What it does not do.** Reads are not blocked (the run can read the original folder and anything else Claude's sandbox can read). The network allowlist covers commands run by Bash; Claude's own tools (WebFetch, MCP servers) follow their own permission rules and are not limited by it. A rule in a repo's own committed `.claude/settings.json` is not checked. Codex runs are not changed: they use `--sandbox workspace-write`, which limits writes to the working directory and temp folders, with `.git` read-only, and has no network.

### Codex

`codex exec --sandbox workspace-write --cd <worktree or copy> --skip-git-repo-check <prompt>`. The sandbox has no network by default, so nothing can be pushed. Its rollout is picked up like any other session: eye treats a `codex exec` rollout as a session only when it ran in a night run's folder (`<eye home>/night/<n>/wt` or `work`); every other `exec` rollout stays hidden.

### Antigravity (agy)

`agy -p <prompt> --mode accept-edits --sandbox --settings <json> [--model <m>]`, started in the worktree or copy. A headless agy answers every permission request with a denial (checked on agy 1.5.14: "a tool required the command permission that headless mode cannot prompt for, so it was auto-denied"), so `--settings` carries the build and test commands as `permissions.allow` rules (`command(cargo test)`, the same list as Claude's) and the push and publish commands as `permissions.deny`. `[night] agy_model` picks the model. The conversation is picked up like any other session, by its folder under `<eye home>/night/`.

**An agy run does not start by default.** `--sandbox` is agy's "terminal sandbox": commands run confined, and a command that would leave it needs approval. eye could not show that this keeps a run's writes inside its own folder (the only headless run with `--sandbox` was denied every command, so nothing was written anywhere), so it does not use it as a barrier. Planning an agy run says so and stops; `[night] allow_unsandboxed = true` plans and starts it anyway, with the worktree or copy as the only protection (the deny rules are advisory). agy cannot fork a conversation, so "continue tonight" is refused for it; only ideas run.

### Continue a session tonight

- **Claude:** eye copies the session's transcript (restoring it from the archive when Claude deleted it) into the project folder Claude uses for the worktree or the copy, then runs `claude -p <prompt> --resume <id> --fork-session --session-id <new uuid>`. `--session-id` names the fork, so the new session is known at once. The original session is untouched.
- **agy:** not possible, agy has no fork.
- **Codex:** `codex exec fork <id> <prompt>` (the installed CLI has it) with `-c sandbox_mode="workspace-write"`, started in the worktree or copy.

The prompt of a continued run starts with a note: it works in a copy at `<worktree>`, the earlier conversation mentions paths of the original folder, use the same files under the worktree and change nothing in the original. The note is only advice; the barrier above is what holds. See `security.md`.

## What a run leaves

When its process ends, eye stores the report (the run's last assistant message, secrets masked, at most 6000 characters), the diff stat (`git diff --stat <base>` plus the files git does not know yet; for a copy the list of changed and new files) and the loose ends of its session (the same list `eye loose` shows), marks its session **Needs you** with the next step "review the night run", and shows a desktop notice ("Night shift: 2 runs finished") when notices are on (`[remind] notify`). Status is `done` (exit 0), `failed` (anything else, with the last line of the log), `stopped` (a limit) or `skipped` (cancelled).

**BRING INTO MY REPO** (a repo copy only) runs `git fetch <copy> night/<...>:night/<...>` in your repo after a confirm, and refuses when your repo already has that branch.

**DISCARD** removes the worktree and its branch (or the copy) after a confirm. It refuses while the worktree has uncommitted changes unless the confirm says they will be lost. It never passes `--force` to git: it deletes the folder, runs `git worktree prune` and `git branch -D`. Commits that exist only on that branch are lost with it, and the confirm says so. A copy has its own repo, which goes with its folder; a branch you already brought into your repo stays there.

## Settings

See `config.md` (`[night]`). `window`, `parallel` and `enabled` can be changed from the CLI and the board; both write `config.toml`, and the daemon reads it again every minute.
