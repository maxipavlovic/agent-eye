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
- `src/looks/index.ts`: the registry. `src/looks/ink/`: the look, one module per part:
  - `Root.tsx` composes the page; `Header.tsx` has the brand, search, switches and the project bar.
  - `Lanes.tsx`, `Grid.tsx`, `Head.tsx`, `Paused.tsx`: the two layouts. `Welcome.tsx`: what a board with nothing on it shows. `Card.tsx`: a card and its "next" line. `Results.tsx`: search results.
  - `Detail.tsx` with `Sections.tsx`, `Conversation.tsx`, `Move.tsx`: the side panel. `CardMenu.tsx`: the right-click menu.
  - `IdeaForm.tsx`, `keys.ts` (board keys), `walk.ts` (arrows step through cards), `layout.ts` (layout and folded lanes).
  - `art.ts`: the pixel art. `tokens.css`, `eye.css`, `ink.css`: styles.

## The look

One look is published: `ink`, a one-bit printed board (warm paper, black ink, dithered greys, vermilion for what needs you), with a Lanes and a Grid layout. See `src/looks/ink/NOTES.md`. An unknown `?look=` or saved `eye.look` falls back to ink.

## Agents and the user

The board does not assume a name or an agent. Next-step owners are `You` (the user), the card's agent (`Card.agent_name`, e.g. `Claude`) or another person. The idea form shows an agent picker only when `GET /api/agents` lists more than one agent.

## Offline sample board

```
npm run look           # -> dist-looks/ink/index.html
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
