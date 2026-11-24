# Testing

Four layers, from fastest to most realistic.

| Layer | Where | Run it | Covers |
|---|---|---|---|
| Rust unit and integration tests | `src/**/tests.rs`, `tests/*.rs` | `cargo test` | the engine, adapters, API routes |
| CLI end to end | `tests/cli_e2e.rs` | `cargo test --test cli_e2e` | the real binary on temporary homes: `usage`, `loose` (stop and remove on things the test made), `night`, `find`, `ls`, `show`, `pick`, `idea`, export and import through `-` |
| Face unit tests | `face/src/**/*.test.ts` (happy-dom) | `cd face && npm test` | components and the store, not the real page |
| Browser end to end | `e2e/` (Playwright) | `scripts/e2e.sh` | the real `agent-eye serve` and the real board in Chromium and WebKit, in both looks (ink and iris): flows and screenshots |

The face tests run in happy-dom, which has no layout. A wrapped card line, a broken form row or a
misaligned bar passes all of them. The browser suite exists for that.

## Browser suite

```
scripts/e2e.sh                 # everything, about a minute when the Rust build is warm
scripts/e2e.sh -g usage        # anything else goes to `playwright test`
scripts/e2e.sh --update        # make the screenshot baselines again
```

Needs docker (and node when `face/dist` is missing). The script runs two containers and removes both:
a Rust one builds the Linux binary with `--features fixed-clock`, and the pinned Playwright image
(the version in `e2e/package.json`) runs the tests. Fonts and rendering are the same on every
machine because every baseline is made in that image. On an Apple-silicon Mac that is arm64 Linux, so
CI runs the job on an arm64 runner; the same image on another CPU may draw a few pixels differently.

What a run does, per test: `examples/fixture_home.rs` makes a fresh made-up eye home, then a real
`agent-eye serve --port 0` starts on it. Nothing touches `~/.claude`, `~/.codex`, `~/.gemini` or
`~/.agent-eye`: `HOME`, `EYE_HOME` and `EYE_CLAUDE_DIR` are temporary folders, and `PATH` holds fake
agents that only exit. Tests that change things (a reminder, a new idea, a stopped process) cannot
affect each other.

The fixture has sessions in every lane, two agents, a Remote session, a very long project name, due
and later reminders, ideas, usage over ten days with plan limits, leftovers, a night run in
each status and a session waiting on a permission.

**Time is fixed.** The binary is built with the `fixed-clock` feature, which makes `EYE_FIXED_NOW_MS`
replace the clock (2026-11-20 12:00 UTC); the browser clock is pinned to the same instant and the
time zone is UTC. Ages, due reminders and usage windows come out the same every day. The feature
is never enabled in a release.

Flows: board loads with no console problems; search (words, a typo, a pasted link); the panel and
its sections; D in the panel; right-click menus; Remind me on the card and in the panel; the idea
form with TONIGHT; the usage panel; LEFTOVERS (Ignore asks first, Stop ends a process the test
started); the Tonight list; export (from MORE) downloads one HTML page with the secret masked; the
look picker (the engine remembers the look, each look keeps its own layout, an unknown `?look=` is
ignored); in iris, each lane's fact and timeline. Every flow runs in both looks: the projects
`chromium` and `webkit` open the board in ink, `chromium-iris` and `webkit-iris` in iris, and each test
checks the page really is in its look. The
worst-case card (every mark, a long project name and title) is screenshot at 260, 310 and 360 px in
light and dark: the card's frame must hold whatever a card carries.

### Screenshots

`tests/visual.spec.ts` takes the board (Lanes and Grid), the panel, the menu, the idea form and the
usage panel, light and dark, at 1440 and 1000 px, and the worst-case card: 34 per project, in
`e2e/__screenshots__/<project>/` (`chromium`, `webkit`, `chromium-iris`, `webkit-iris`).

WebKit draws at 2x and the screenshots are kept at CSS size, so a line one device pixel wide (iris
on a Retina screen) can drop out of a WebKit baseline even though the browser draws it. Check such
lines at device scale (`scale: 'device'`) when one seems to be missing.

- **When a screenshot fails** the page changed. Look at the diff (`e2e/test-results/*/…-diff.png`;
  CI uploads them as the `e2e-diffs` artifact). A bug: fix it. A change you meant: update.
- **To update**: `scripts/e2e.sh --update`, then read every changed image (`git diff --stat
  e2e/__screenshots__`) before committing them with the change that caused them. Never update to
  make a red run green without looking.
- **When to update**: after a change to board layout, styles, copy or the fixture. Do not update
  for a different machine; a baseline made outside the container is wrong by definition.
- Selectors use roles and visible text, so a restyle does not break a flow. A screenshot of a view
  that scrolls sideways is taken with the target fully in view, so the click does not move the page.
