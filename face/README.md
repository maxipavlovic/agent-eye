# agent-eye face

The TypeScript front end of the agent-eye board: Preact + signals, built with Vite. The Rust engine serves the JSON API (`../docs/api.md`) and, once built, this app at `/`.

## Build

```
cd face
npm ci
npm run build          # -> dist/
cd .. && cargo build --release   # embeds face/dist in the binary
```

`cargo build` works without node or a built face: with no `face/dist/index.html` it embeds nothing and `/` serves a plain page saying the board was not built in. Rebuild cargo after changing `face/dist`.

## Dev loop

```
cd face && npm run watch                       # vite build --watch -> dist/
EYE_FACE_DIR=face/dist eye serve --port 7441   # reads dist/ on every request, no cargo rebuild
```

Open the URL `eye serve` prints (`/?t=<token>` sets the cookie). Reload after each rebuild. Use a temporary `EYE_HOME` and `EYE_CLAUDE_DIR` when experimenting.

## Layout

- `index.html`, `src/main.tsx`: the real app. `createStore(liveSource())`, then the look.
- `src/api/types.ts`: the API shapes, and `Source`, the one interface the store reads from.
- `src/data/live.ts`: `Source` over the API and `/api/events`. `src/data/fixture.ts`: `Source` over `fixtures/sample/` (synthetic, committed), or over `fixtures/real/` (a git-ignored snapshot of your own board) when that exists.
- `src/store.ts`: signals and actions. Looks never fetch. `src/format.ts`: pure helpers (ages, "next" owners, paths, tokens).
- `src/pixel/`: `sprite.ts` turns text rows into SVG; `Pix.tsx` is the component.
- `src/board/`: the board every look shares, one module per part:
  - `look.tsx`: what a look gives (`Look`: its header, project bar, lane head, welcome, loading, footer and `Art`, the drawings the shared parts ask for); `LookProvider` and `useLook()`.
  - `Board.tsx` composes the page from the look's parts. `Lanes.tsx`, `Grid.tsx`, `Paused.tsx`: the two layouts. `Card.tsx`: a card, its marks and its "next" line (the fixed frame). `Results.tsx`: search results.
  - `Detail.tsx` with `Sections.tsx`, `Conversation.tsx`, `Move.tsx`: the side panel. `CardMenu.tsx`: the right-click menu. `AccentPicker.tsx`: the look and the accent colour.
  - `IdeaForm.tsx`, `Remind.tsx`, `DatePick.tsx`, `Night.tsx`, `Usage.tsx`, `Loose.tsx`, `Forget.tsx`, `KeyHelp.tsx`, `Toasts.tsx`: forms and panels.
  - `keys.ts` (board keys), `walk.ts` (arrows step through cards), `layout.ts` (layout and folded lanes, per look), `columns.ts`, `model.ts` (projects), `folder.ts`, `worked.ts`.
- `src/looks/index.ts`: the registry and the picker's list. Each look is a folder with a `Root.tsx` (its `Look` in a `LookProvider` around `Board`), its parts and its styles:
  - `src/looks/ink/`: `look.tsx` (the parts and the pixel `Art`), `Header.tsx`, `Head.tsx`, `Eye.tsx`, `Welcome.tsx`, `art.ts` (the sprites), `tokens.css`, `eye.css`, `ink.css`.
  - `src/looks/iris/`: `look.tsx` (the parts, the welcome and the title block), `art.tsx` (hairline drawings), `Header.tsx` (cells and the project strips), `Head.tsx` (count, fact, timeline), `facts.ts`, `Eye.tsx`, `tokens.css`, `iris.css`.

## The looks

Two looks, picked in the header's colour button (INK or IRIS, shared by every browser as `eye.look`) or with `?look=`; an unknown name falls back to ink.

- `ink`: a one-bit printed board (warm paper, black ink, dithered greys, vermilion for what needs you). See `src/looks/ink/NOTES.md`.
- `iris`: the board as a technical drawing in one-pixel lines (header cells, condensed counts, a fact and a timeline under each lane, a title block). See `src/looks/iris/NOTES.md`.

Both have the Lanes and the Grid layout, the same cards, panel, forms, keys and the eight accent colours.

**Adding a look.**

1. Make `src/looks/<name>/` with a `Look` (`board/look.tsx` lists what it gives) and a `Root.tsx` that wraps `Board` in it. Build its header from `board/HeaderParts.tsx` and drive its eye with `board/gaze.ts`; the look only draws.
2. Write a style sheet for the shared parts' classes (iris.css covers all of them) that imports `board/modal.css` first, and a `tokens.css` with the eight accents, with a test like `iris/tokens.test.ts` (the checks are in `src/colourCheck.ts`).
3. Add the look to `LOOK_LIST` in `src/looks/names.ts` (the picker's label) and its loader to `looks` in `src/looks/index.ts`.
4. For the offline sample: `looks/<name>/index.html`, `src/looks/<name>/main.tsx` and a `look:<name>` script.
5. Add two Playwright projects in `e2e/playwright.config.ts`, make their baselines with `scripts/e2e.sh --update --project=…` and look at every one.
6. List any new font's license in `../licenses/`.

A look's chunk loads only when it is picked.

## Agents and the user

The board does not assume a name or an agent. Next-step owners are `You` (the user), the card's agent (`Card.agent_name`, e.g. `Claude`) or another person. The idea form shows an agent picker only when `GET /api/agents` lists more than one agent.

## Offline sample board

```
npm run look           # -> dist-looks/ink/index.html
npm run look:iris      # -> dist-looks/iris/index.html
```

One self-contained file (sample data, fonts and sprites inlined) that opens from `file://`. The sample is a developer's week across five made-up projects; its times are moved on load, so it always looks fresh. The project site serves this file as its live demo.

## Screenshots and media

```
node tools/shot.mjs "file://$PWD/dist-looks/ink/index.html" shots/board.png --w 1440 --h 900
node tools/media.mjs ../assets   # board-light.png, board-dark.png, board-demo.gif
```

`tools/shot.mjs` options: `--dark`, `--w`, `--h`, `--dpr`, `--click`, `--type '#q=text'`, `--eval`, `--full`, `--reduced-motion`. Both tools need headless Chrome with remote debugging on port 9223 (see the top of `tools/cdp.mjs`); `media.mjs` also needs `ffmpeg`.

## Tests

```
npm test               # vitest (happy-dom): the live and fixture sources, the store, the look
npm run typecheck      # strict TypeScript
npm run coverage       # coverage/lcov.info
```

## Site

The project site lives in `../site` (a small Vite project). `npm run site:dev` and `npm run site:build` run it from here; the output is `../site/dist`.
