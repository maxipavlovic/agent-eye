// A tiny Chrome DevTools Protocol client for the screenshot tools.
// It talks to a running headless Chrome (CDP on 127.0.0.1:9223 by default, or $CDP_PORT):
//   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new \
//     --disable-gpu --no-first-run --user-data-dir=<tmp> --remote-debugging-port=9223 about:blank
// Every page gets its own throwaway browser context, so runs share no storage or tabs.
import fs from 'node:fs';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export { sleep };

/** Opens a page. Options: w, h (CSS px), dpr, dark, reducedMotion, mobile. */
export async function openPage(opt = {}) {
  const view = { w: 1440, h: 900, dpr: 1, dark: false, reducedMotion: false, mobile: false, ...opt };
  const port = process.env.CDP_PORT || 9223;
  let version;
  try {
    version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  } catch {
    throw new Error(`No Chrome on 127.0.0.1:${port}. Start it (see the header of tools/cdp.mjs).`);
  }
  const ws = new WebSocket(version.webSocketDebuggerUrl);
  let seq = 0;
  const pending = new Map();
  const waiters = [];
  const logs = [];
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const { res, rej, method } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) rej(new Error(`${method}: ${msg.error.message}`)); else res(msg.result);
      return;
    }
    const p = msg.params || {};
    if (msg.method === 'Runtime.consoleAPICalled' && ['error', 'warning', 'warn'].includes(p.type)) {
      logs.push(`console.${p.type}: ` + p.args.map((x) => x.value ?? x.description).join(' '));
    }
    if (msg.method === 'Runtime.exceptionThrown') logs.push('EXCEPTION: ' + (p.exceptionDetails.exception?.description || p.exceptionDetails.text));
    if (msg.method === 'Log.entryAdded' && !['verbose', 'info'].includes(p.entry.level)) logs.push(`log.${p.entry.level}: ${p.entry.text} ${p.entry.url || ''}`);
    for (const wt of [...waiters]) if (wt.method === msg.method) { waiters.splice(waiters.indexOf(wt), 1); wt.res(p); }
  };
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  const send = (method, params = {}, sessionId) => new Promise((res, rej) => {
    const id = ++seq;
    pending.set(id, { res, rej, method });
    ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  });
  const once = (method, ms) => new Promise((res) => {
    const wt = { method, res };
    waiters.push(wt);
    setTimeout(() => { const i = waiters.indexOf(wt); if (i >= 0) { waiters.splice(i, 1); res(null); } }, ms);
  });

  const { browserContextId } = await send('Target.createBrowserContext', { disposeOnDetach: true });
  const { targetId } = await send('Target.createTarget', { url: 'about:blank', browserContextId });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  const cdp = (method, params) => send(method, params, sessionId);
  await cdp('Runtime.enable');
  await cdp('Log.enable');
  await cdp('Page.enable');

  const emulate = async (o) => {
    await cdp('Emulation.setDeviceMetricsOverride', { width: o.w, height: o.h, deviceScaleFactor: o.dpr, mobile: o.mobile });
    await cdp('Emulation.setEmulatedMedia', {
      features: [
        { name: 'prefers-color-scheme', value: o.dark ? 'dark' : 'light' },
        { name: 'prefers-reduced-motion', value: o.reducedMotion ? 'reduce' : 'no-preference' },
      ],
    });
  };
  await emulate(view);

  const evaluate = async (expression) => {
    const r = await cdp('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  };
  /** Centre of the first element matching `sel`, scrolled into view. */
  const centre = async (sel) => {
    const r = await evaluate(`(() => { const el = document.querySelector(${JSON.stringify(sel)}); if (!el) return null; el.scrollIntoView({block:'center',inline:'center'}); const b = el.getBoundingClientRect(); return {x:b.left+b.width/2,y:b.top+b.height/2}; })()`);
    if (!r) throw new Error(`no element for ${sel}`);
    return r;
  };
  const at = (target) => (typeof target === 'string' ? centre(target) : target);
  const mouse = (type, x, y, extra = {}) => cdp('Input.dispatchMouseEvent', { type, x, y, ...extra });

  const page = {
    logs,
    cdp,
    evaluate,
    /** Loads a URL and waits `wait` ms. */
    async goto(url, wait = 800) {
      const loaded = once('Page.loadEventFired', 15000);
      await cdp('Page.navigate', { url });
      await loaded;
      await sleep(wait);
    },
    /** Changes the viewport and colour scheme without reloading. */
    async setView(o) { Object.assign(view, o); await emulate(view); },
    /** PNG as a Buffer; `clip` is {x,y,width,height} in CSS px. */
    async png(o = {}) {
      const p = { format: 'png', ...(o.full || o.clip ? { captureBeyondViewport: true } : {}) };
      if (o.full) {
        const m = await cdp('Page.getLayoutMetrics');
        const { width, height } = m.cssContentSize || m.contentSize;
        p.clip = { x: 0, y: 0, width, height, scale: 1 };
      } else if (o.clip) p.clip = { ...o.clip, scale: 1 };
      const shot = await cdp('Page.captureScreenshot', p);
      return Buffer.from(shot.data, 'base64');
    },
    async shot(file, o = {}) { fs.writeFileSync(file, await page.png(o)); },
    async hover(target) { const { x, y } = await at(target); await mouse('mouseMoved', x, y); return { x, y }; },
    async click(target) {
      const { x, y } = await at(target);
      await mouse('mouseMoved', x, y);
      await mouse('mousePressed', x, y, { button: 'left', clickCount: 1 });
      await mouse('mouseReleased', x, y, { button: 'left', clickCount: 1 });
      return { x, y };
    },
    async rightClick(target) {
      const { x, y } = await at(target);
      await mouse('mouseMoved', x, y);
      await mouse('mousePressed', x, y, { button: 'right', clickCount: 1 });
      await mouse('mouseReleased', x, y, { button: 'right', clickCount: 1 });
      return { x, y };
    },
    async move(x, y) { await mouse('mouseMoved', x, y); },
    /** Presses a key by its `key` value, e.g. 'Escape', '/', 'ArrowRight'. */
    async key(key, modifiers = 0) {
      const text = key.length === 1 ? key : undefined;
      await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key, text, modifiers, windowsVirtualKeyCode: key.length === 1 ? key.toUpperCase().charCodeAt(0) : ({ Escape: 27, Enter: 13, ArrowRight: 39, ArrowLeft: 37 }[key] ?? 0) });
      await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key, modifiers });
    },
    /** Types text into the focused element, one character at a time. */
    async type(text, delay = 0) {
      for (const ch of text) {
        await cdp('Input.dispatchKeyEvent', { type: 'keyDown', key: ch, text: ch });
        await cdp('Input.dispatchKeyEvent', { type: 'keyUp', key: ch });
        if (delay) await sleep(delay);
      }
    },
    async close() {
      await send('Target.closeTarget', { targetId }).catch(() => {});
      await send('Target.disposeBrowserContext', { browserContextId }).catch(() => {});
      ws.close();
    },
  };
  return page;
}
