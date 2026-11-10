#!/usr/bin/env node
// Screenshot a page through a running headless Chrome (CDP on 127.0.0.1:9223).
// Each run gets its own throwaway browser context, so runs don't share storage or tabs.
//
//   node tools/shot.mjs <url> <out.png> [options]
//     --w 1600 --h 1000      viewport (CSS px)
//     --dpr 1                device pixel ratio
//     --dark | --light       prefers-color-scheme (default light)
//     --reduced-motion       prefers-reduced-motion: reduce
//     --wait 800             ms to wait after load (and after each action)
//     --click <selector>     click an element (repeatable, runs in order)
//     --type <selector>=<text>  set an input's value and fire "input" (repeatable)
//     --eval <js>            print the JSON value of an expression at the end
//     --full                 capture the full scrollable page
//
// Start Chrome once:  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new \
//   --disable-gpu --no-first-run --user-data-dir=<tmp> --remote-debugging-port=9223 about:blank
// Prints console errors and exceptions; exits 3 when the page threw.
import fs from 'node:fs';

const args = process.argv.slice(2);
const pos = [];
const opt = { w: 1600, h: 1000, dpr: 1, dark: false, reduced: false, wait: 800, actions: [], evals: [], full: false };
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  if (a === '--w') opt.w = +args[++i];
  else if (a === '--h') opt.h = +args[++i];
  else if (a === '--dpr') opt.dpr = +args[++i];
  else if (a === '--dark') opt.dark = true;
  else if (a === '--light') opt.dark = false;
  else if (a === '--reduced-motion') opt.reduced = true;
  else if (a === '--wait') opt.wait = +args[++i];
  else if (a === '--click') opt.actions.push({ click: args[++i] });
  else if (a === '--type') opt.actions.push({ type: args[++i] });
  else if (a === '--eval') opt.evals.push(args[++i]);
  else if (a === '--full') opt.full = true;
  else pos.push(a);
}
const [url, out] = pos;
if (!url || !out) {
  console.error('usage: node tools/shot.mjs <url> <out.png> [--w --h --dpr --dark --reduced-motion --wait --click --type --eval --full]');
  process.exit(2);
}
const port = process.env.CDP_PORT || 9223;
let ver;
try {
  ver = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
} catch {
  console.error(`No Chrome on 127.0.0.1:${port}. Start it (see the header of this file).`);
  process.exit(2);
}

const ws = new WebSocket(ver.webSocketDebuggerUrl);
let seq = 0;
const pending = new Map();
const waiters = [];
const logs = [];
let threw = 0;
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    const { res, rej, method } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? rej(new Error(`${method}: ${msg.error.message}`)) : res(msg.result);
    return;
  }
  const p = msg.params || {};
  if (msg.method === 'Runtime.consoleAPICalled' && (p.type === 'error' || p.type === 'warning' || p.type === 'warn')) {
    logs.push(`console.${p.type}: ` + p.args.map((x) => x.value ?? x.description).join(' '));
  }
  if (msg.method === 'Runtime.exceptionThrown') {
    threw++;
    logs.push('EXCEPTION: ' + (p.exceptionDetails.exception?.description || p.exceptionDetails.text));
  }
  if (msg.method === 'Log.entryAdded' && p.entry.level !== 'verbose' && p.entry.level !== 'info') {
    logs.push(`log.${p.entry.level}: ${p.entry.text} ${p.entry.url || ''}`);
  }
  for (const w of [...waiters]) if (w.method === msg.method) { waiters.splice(waiters.indexOf(w), 1); w.res(p); }
};
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });

const send = (method, params = {}, sessionId) => new Promise((res, rej) => {
  const id = ++seq;
  pending.set(id, { res, rej, method });
  ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
});
const once = (method, ms) => new Promise((res) => {
  const w = { method, res };
  waiters.push(w);
  setTimeout(() => { const i = waiters.indexOf(w); if (i >= 0) { waiters.splice(i, 1); res(null); } }, ms);
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const { browserContextId } = await send('Target.createBrowserContext', { disposeOnDetach: true });
const { targetId } = await send('Target.createTarget', { url: 'about:blank', browserContextId });
const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
const page = (method, params) => send(method, params, sessionId);
const evaluate = async (expression) => {
  const r = await page('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
};

let code = 0;
try {
  await page('Runtime.enable');
  await page('Log.enable');
  await page('Page.enable');
  await page('Emulation.setDeviceMetricsOverride', { width: opt.w, height: opt.h, deviceScaleFactor: opt.dpr, mobile: false });
  await page('Emulation.setEmulatedMedia', {
    features: [
      { name: 'prefers-color-scheme', value: opt.dark ? 'dark' : 'light' },
      { name: 'prefers-reduced-motion', value: opt.reduced ? 'reduce' : 'no-preference' },
    ],
  });
  const loaded = once('Page.loadEventFired', 15000);
  await page('Page.navigate', { url });
  await loaded;
  await sleep(opt.wait);
  for (const a of opt.actions) {
    if (a.click) {
      const ok = await evaluate(`(() => { const el = document.querySelector(${JSON.stringify(a.click)}); if (!el) return false; el.scrollIntoView({block:'center'}); el.click(); return true; })()`);
      if (!ok) logs.push(`ACTION: no element for --click ${a.click}`);
    } else if (a.type) {
      const i = a.type.indexOf('=');
      const sel = a.type.slice(0, i), text = a.type.slice(i + 1);
      const ok = await evaluate(`(() => { const el = document.querySelector(${JSON.stringify(sel)}); if (!el) return false; el.focus(); el.value = ${JSON.stringify(text)}; el.dispatchEvent(new Event('input', {bubbles:true})); return true; })()`);
      if (!ok) logs.push(`ACTION: no element for --type ${sel}`);
    }
    await sleep(opt.wait);
  }
  for (const e of opt.evals) console.log('EVAL', e, '=>', JSON.stringify(await evaluate(e)));
  let shot;
  if (opt.full) {
    const m = await page('Page.getLayoutMetrics');
    const { width, height } = m.cssContentSize || m.contentSize;
    shot = await page('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true, clip: { x: 0, y: 0, width, height, scale: 1 } });
  } else {
    shot = await page('Page.captureScreenshot', { format: 'png' });
  }
  fs.writeFileSync(out, Buffer.from(shot.data, 'base64'));
  console.log(`png written: ${out}`);
} catch (e) {
  logs.push('SHOT FAILED: ' + e.message);
  code = 1;
} finally {
  console.log(logs.length ? logs.join('\n') : 'no console errors');
  if (threw && !code) code = 3;
  await send('Target.closeTarget', { targetId }).catch(() => {});
  await send('Target.disposeBrowserContext', { browserContextId }).catch(() => {});
  ws.close();
  process.exit(code);
}
