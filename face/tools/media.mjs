#!/usr/bin/env node
// Makes the board's README media from the offline sample build (npm run look):
//   board-light.png                   1440x900 at 2x
//   board-demo.gif                    open a card, search, right-click Mark done
//
//   node tools/media.mjs [outdir]     (default ../assets)
//
// Needs headless Chrome with remote debugging (see tools/cdp.mjs) and ffmpeg. Pixel art is never
// resampled: the GIF is captured at its final size, and its palette has no dithering.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { openPage, sleep } from './cdp.mjs';

const outDir = path.resolve(process.argv[2] ?? '../assets');
const board = path.resolve(import.meta.dirname, '../dist-looks/ink/index.html');
if (!fs.existsSync(board)) {
  console.error('Build the sample board first: npm run look');
  process.exit(2);
}
const url = `file://${board}`;
fs.mkdirSync(outDir, { recursive: true });

// ───────────── stills ─────────────

{
  const page = await openPage({ w: 1440, h: 900, dpr: 2, dark: false, reducedMotion: true });
  await page.goto(url, 2000);
  await page.shot(path.join(outDir, 'board-light.png'));
  await page.close();
}

// ───────────── the demo GIF ─────────────

const W = 1200;
const H = 750;
const page = await openPage({ w: W, h: H, dpr: 1, reducedMotion: true });
await page.goto(url, 2000);

// A pixel mouse pointer, since screenshots do not show the real one.
await page.evaluate(`(() => {
  const rows = ['#...........','##..........','#o#.........','#oo#........','#ooo#.......','#oooo#......','#ooooo#.....','#oooooo#....',
    '#ooooooo#...','#oooooooo#..','#ooooo#####.','#oo#oo#.....','#o#.#oo#....','##..#oo#....','#....#oo#...','.....#oo#...','......##....'];
  const rects = rows.flatMap((r, y) => [...r].map((c, x) => c === '.' ? '' : '<rect x="' + x + '" y="' + y + '" width="1" height="1" fill="' + (c === '#' ? '#161616' : '#ffffff') + '"/>')).join('');
  const el = document.createElement('div');
  el.style.cssText = 'position:fixed;left:0;top:0;z-index:99999;pointer-events:none;transform-origin:0 0';
  el.innerHTML = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 17" width="24" height="34" shape-rendering="crispEdges">' + rects + '</svg>';
  document.body.appendChild(el);
  window.__cursor = (x, y, down) => { el.style.left = x + 'px'; el.style.top = y + 'px'; el.style.transform = down ? 'scale(.85)' : 'none'; };
  window.__cursor(${W - 40}, ${H - 60}, false);
})()`);

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'eye-gif-'));
const frames = [];
let pos = { x: W - 40, y: H - 60 };

const snap = async () => {
  const file = path.join(dir, `f${String(frames.length).padStart(4, '0')}.png`);
  await page.shot(file);
  frames.push({ file, t: performance.now() });
};
/** Keeps snapping for `ms`: the frames of an animation or a typing burst. */
const watch = async (ms, every = 80) => {
  const end = performance.now() + ms;
  while (performance.now() < end) { const t = performance.now(); await snap(); await sleep(Math.max(0, every - (performance.now() - t))); }
};
/** One frame, then still for `ms` (its duration is the time until the next frame). */
const hold = async (ms) => { await snap(); await sleep(ms); };

const where = (selector, text) => page.evaluate(`(() => {
  const els = [...document.querySelectorAll(${JSON.stringify(selector)})].filter((e) => ${JSON.stringify(text ?? '')} === '' || e.textContent.includes(${JSON.stringify(text ?? '')}));
  const el = els[0];
  if (!el) return null;
  const b = el.getBoundingClientRect();
  return { x: b.left + Math.min(b.width / 2, 90), y: b.top + Math.min(b.height / 2, 28) };
})()`);

const glide = async (to, steps = 7) => {
  const from = pos;
  for (let i = 1; i <= steps; i++) {
    const e = i / steps;
    const x = from.x + (to.x - from.x) * (e * e * (3 - 2 * e));
    const y = from.y + (to.y - from.y) * (e * e * (3 - 2 * e));
    await page.move(x, y);
    await page.evaluate(`window.__cursor(${x}, ${y}, false)`);
    await snap();
  }
  pos = to;
};
/** Scrolls the element's lane (a few frames) when it sits below the visible part of the board. */
const reveal = async (selector, text) => {
  const hidden = await page.evaluate(`(() => {
    const el = [...document.querySelectorAll(${JSON.stringify(selector)})].find((e) => e.textContent.includes(${JSON.stringify(text ?? '')}));
    const lane = el?.closest('.cbody');
    return el && lane ? Math.round(el.getBoundingClientRect().bottom - (innerHeight - 70)) : 0;
  })()`);
  if (hidden <= 0) return;
  for (let i = 1; i <= 6; i++) {
    await page.evaluate(`(() => {
      const el = [...document.querySelectorAll(${JSON.stringify(selector)})].find((e) => e.textContent.includes(${JSON.stringify(text ?? '')}));
      el.closest('.cbody').scrollTop += ${Math.ceil(hidden / 6)};
    })()`);
    await snap();
  }
};
const target = async (selector, text) => {
  await reveal(selector, text);
  const at = await where(selector, text);
  if (!at) throw new Error(`nothing for ${selector} ${text ?? ''}`);
  return at;
};
const press = async (kind) => {
  await page.evaluate(`window.__cursor(${pos.x}, ${pos.y}, true)`);
  await snap();
  await kind();
  await page.evaluate(`window.__cursor(${pos.x}, ${pos.y}, false)`);
};
const click = async (selector, text) => { await glide(await target(selector, text)); await press(() => page.click(pos)); };
const rightClick = async (selector, text) => { await glide(await target(selector, text)); await press(() => page.rightClick(pos)); };

// 1. the board at rest
await hold(1500);

// 2. open a card: the session stopped on a permission prompt
await click('.c-needs_you .card', 'date library');
await watch(500);
await hold(2200);
await page.key('Escape');
await watch(350);

// 3. search: words are marked in the results, and again in the panel
await click('#q');
for (const ch of 'retry') { await page.type(ch); await sleep(40); await snap(); await sleep(60); }
await watch(500);
await hold(1500);
await click('.results .card', 'webhooks');
await watch(500);
await hold(2400);
await page.key('Escape');
await watch(300);
await page.key('Escape');
await watch(500);
await hold(700);

// 4. right-click, Mark done, then Undo so the loop starts from the same board
await rightClick('.c-needs_you .card', 'flaky login');
await watch(400);
await hold(1000);
await click('.ctx button', 'MARK DONE');
await watch(500);
await hold(1600);
await click('.toast .tact');
await watch(400);
// back to the first frame, so the loop is seamless
for (let i = 5; i >= 0; i--) {
  await page.evaluate(`document.querySelector('.c-needs_you .cbody').scrollTop = Math.round(document.querySelector('.c-needs_you .cbody').scrollTop * ${i} / 6)`);
  await snap();
}
await glide({ x: W - 40, y: H - 60 });
await hold(800);

await page.close();

// frames to GIF: real timing, one shared palette, no dithering
const list = frames.map((f, i) => {
  const next = frames[i + 1]?.t ?? f.t + 1000;
  return `file '${f.file}'\nduration ${Math.max(0.03, (next - f.t) / 1000).toFixed(3)}`;
}).join('\n') + `\nfile '${frames.at(-1).file}'\n`;
fs.writeFileSync(path.join(dir, 'frames.txt'), list);
const gif = path.join(outDir, 'board-demo.gif');
execFileSync('ffmpeg', [
  '-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', path.join(dir, 'frames.txt'),
  '-filter_complex', '[0:v]fps=12,split[a][b];[a]palettegen=max_colors=96:stats_mode=full[p];[b][p]paletteuse=dither=none:diff_mode=rectangle',
  '-loop', '0', gif,
]);
fs.rmSync(dir, { recursive: true, force: true });
console.log(`${frames.length} frames -> ${gif} (${(fs.statSync(gif).size / 1e6).toFixed(2)} MB)`);
