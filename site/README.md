# agent-eye site

The project site, served on GitHub Pages at `https://maxipavlovic.github.io/agent-eye/` (base path `/agent-eye/`). A small Vite project: a landing page, a live demo of the board, and the docs.

```
cd face && npm ci                 # the demo is the board's offline build
cd ../site && npm ci
npm run build                     # -> site/dist
npm run dev                       # http://localhost:5173/agent-eye/
```

From `face/`, `npm run site:build` and `npm run site:dev` do the same (they install the site's packages first when they are missing). Node 22.18 or newer.

## How it is built

- `pages/landing.html` is the landing page's body. `{{sprite:name:scale}}` draws a pixel icon, `{{eye}}` the brand eye, `{{base}}` the base path.
- Every `../docs/*.md` becomes a page at `docs/<name>/`, with a menu and an "on this page" list. Links between docs files become page links; links to other repo files open on GitHub. Page order: `plugin/docs.ts`.
- `npm run demo` builds the board's offline sample build (`face/dist-looks/ink/index.html`); the plugin copies it to `demo/index.html`, and the landing page shows it in a frame.
- Colours, fonts, the eye and the pixel sprites come from `face/` (`tokens.css`, `eye.css`, `art.ts`), so the site and the board look the same. Fonts are bundled; nothing loads from a CDN.
- `plugin/site.ts` is the Vite plugin that does all of this, in dev and in the build.
