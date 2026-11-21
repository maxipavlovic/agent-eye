// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Source } from '../../api/types';
import { demoUsage } from '../../data/usageSample';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { keyHelp, KEY_LIST } from './KeyHelp';
import { tonightOpen } from './Night';
import { remindForm } from './Remind';
import { usagePanel } from './Usage';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  globalThis.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as typeof matchMedia;
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); tonightOpen.value = null; keyHelp.value = false; remindForm.value = null; usagePanel.value = false; });
afterEach(() => {
  if (mounted) { render(null, mounted); mounted.remove(); mounted = null; }
  document.body.innerHTML = '';
});

async function mount(over: Partial<Source> = {}) {
  const store = createStore({ ...fixtureSource(true), ...over });
  const root = document.createElement('div');
  document.body.appendChild(root);
  mounted = root;
  render(h(Root, { store }), root);
  await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
  await vi.waitFor(() => expect(store.night.value).not.toBeNull());
  const $ = <T extends Element = HTMLElement>(sel: string) => root.querySelector<T>(sel);
  const $$ = <T extends Element = HTMLElement>(sel: string) => [...root.querySelectorAll<T>(sel)];
  return { store, root, $, $$ };
}

const tick = () => new Promise((r) => setTimeout(r, 40));
const text = (e: Element | null | undefined) => e?.textContent ?? '';
const press = (k: string, init: KeyboardEventInit = {}) => document.body.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }));
const session = (m: Awaited<ReturnType<typeof mount>>) => m.$$('.card').find((c) => !c.classList.contains('idea') && !c.classList.contains('row') && !c.querySelector('.mk.night'))!;
const rightClick = (el: Element) => el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 200, clientY: 200 }));

describe('CONTINUE TONIGHT in the card menu', () => {
  it('is in a session\'s menu, and opens the panel with the form open and its prompt focused', async () => {
    const m = await mount();
    rightClick(session(m));
    await vi.waitFor(() => expect(m.$('.ctx')).not.toBeNull());
    const labels = m.$$('.ctx button').map(text);
    expect(labels).toContain('CONTINUE TONIGHT…');
    expect(labels.indexOf('CONTINUE TONIGHT…')).toBeLessThan(labels.indexOf('OPEN DETAILS'));
    m.$$('.ctx button').find((b) => text(b) === 'CONTINUE TONIGHT…')!.click();
    await vi.waitFor(() => expect(m.$('.page form.nplan textarea')).not.toBeNull());
    await vi.waitFor(() => expect(document.activeElement).toBe(m.$('.page form.nplan textarea')));
    expect(m.$('.ctx')).toBeNull();
  });

  it('is not in an idea\'s menu', async () => {
    const m = await mount();
    rightClick(m.$$('.card.idea')[0]!);
    await vi.waitFor(() => expect(m.$('.ctx')).not.toBeNull());
    expect(m.$$('.ctx button').map(text).join('|')).not.toContain('CONTINUE TONIGHT');
    // the menu is a module signal: close it so it does not outlive the test
    press('Escape');
    await vi.waitFor(() => expect(m.$('.ctx')).toBeNull());
  });
});

describe('the panel keys and the keys list', () => {
  async function open() {
    const m = await mount();
    session(m).click();
    await vi.waitFor(() => expect(m.$('.page .qmove')).not.toBeNull());
    return m;
  }

  it('T opens CONTINUE TONIGHT and M the reminder picker', async () => {
    const m = await open();
    expect(m.$('.page form.nplan')).toBeNull();
    press('t');
    await vi.waitFor(() => expect(m.$('.page form.nplan')).not.toBeNull());
    expect(m.$('.page .acts.end .btn[aria-keyshortcuts=T]')!.getAttribute('aria-expanded')).toBe('true');
    expect(remindForm.value).toBeNull();
    press('m');
    await vi.waitFor(() => expect(m.$('form.modal[aria-label="Remind me"]')).not.toBeNull());
  });

  it('F forks, H hides, and a key does nothing while a field has the focus or with a modifier', async () => {
    const resume = vi.fn(async () => {});
    const setHidden = vi.fn(async (_id: string, _hide: boolean) => ({}) as never);
    const m = await mount({ resume: resume as never, setHidden });
    m.$$('.card').find((c) => text(c).includes('Add dark mode'))!.click();
    await vi.waitFor(() => expect(m.$('.page .qmove')).not.toBeNull());
    const id = m.store.selected.value!;
    const input = m.$<HTMLInputElement>('#q')!;
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'h', bubbles: true }));
    press('h', { metaKey: true });
    press('f', { ctrlKey: true });
    await new Promise((r) => setTimeout(r, 30));
    expect(setHidden).not.toHaveBeenCalled();
    expect(resume).not.toHaveBeenCalled();
    press('f');
    await vi.waitFor(() => expect(resume.mock.calls.at(-1)).toEqual([id, false, true]));
    press('h');
    await vi.waitFor(() => expect(setHidden).toHaveBeenCalledWith(id, true));
  });

  it('shows each key on its button, the way RESUME shows R', async () => {
    const m = await open();
    const keyed = (sel: string) => text(m.$(`${sel} u`));
    expect(keyed('.qmove .resume')).toBe('R');
    expect(m.$$('.page .acts.end .btn u').map(text)).toEqual(['F', 'C', 'E', 'T', 'H']);
    expect(m.$$('.qmove button[aria-keyshortcuts] u').map(text)).toEqual(['N', 'W', 'P', 'D', 'R']);
  });

  it('? opens the list of keys, and ?, Esc or CLOSE closes it without closing the panel', async () => {
    const m = await open();
    press('?', { shiftKey: true });
    await vi.waitFor(() => expect(m.$('.keyhelp')).not.toBeNull());
    const shown = m.$$('.keyhelp dt').map(text);
    for (const [k] of KEY_LIST.filter(([k]) => k !== 'X')) expect(shown).toContain(k);
    press('Escape');
    await tick();
    expect(m.$('.keyhelp')).toBeNull();
    expect(m.store.selected.value).not.toBeNull();
    m.$<HTMLButtonElement>('.page .qk')!.click();
    await vi.waitFor(() => expect(m.$('.keyhelp')).not.toBeNull());
    press('?', { shiftKey: true });
    await tick();
    expect(m.$('.keyhelp')).toBeNull();
    press('?', { shiftKey: true });
    await vi.waitFor(() => expect(m.$('.keyhelp')).not.toBeNull());
    m.$$('.keyhelp button').find((b) => text(b) === 'CLOSE')!.click();
    await tick();
    expect(m.$('.keyhelp')).toBeNull();
  });

  it('N still moves the open session to Needs you, as it always did', async () => {
    const m = await open();
    press('n');
    await vi.waitFor(() => expect(m.$('.qmove button[aria-pressed="true"]')?.textContent).toBe('NEEDS YOU'));
  });

  it('X opens the FORGET… confirm for the open session', async () => {
    const m = await open();
    expect(m.$('.forget')).toBeNull();
    press('x');
    await vi.waitFor(() => expect(m.$('.forget')).not.toBeNull());
  });
});

describe('the reminder row at the top of the panel', () => {
  it('shows the accent clock, when and the note, with CHANGE and CLEAR', async () => {
    const remind = vi.fn(async (_id: string, _i: unknown) => ({}) as never);
    const m = await mount({ remind });
    m.$$('.card').find((c) => text(c).includes('Add dark mode'))!.click();
    await vi.waitFor(() => expect(m.$('.page .rrow')).not.toBeNull());
    const row = m.$('.page .rrow')!;
    expect(text(row)).toContain('Tomorrow 9:00');
    expect(text(row)).toContain('the contrast review');
    expect(row.querySelector('.rclock svg')).not.toBeNull();
    // in the status row: right under the quick moves, before the state line
    expect(m.$('.page .qmove')!.nextElementSibling).toBe(row);
    [...row.querySelectorAll('button')].find((b) => text(b) === 'CHANGE')!.click();
    await vi.waitFor(() => expect(remindForm.value).toBe(m.store.selected.value));
    remindForm.value = null;
    [...row.querySelectorAll('button')].find((b) => text(b) === 'CLEAR')!.click();
    await vi.waitFor(() => expect(remind).toHaveBeenCalledWith(m.store.selected.value, { clear: true }));
  });

  it('says why when the reminder is due, as the card does', async () => {
    const base = fixtureSource(true);
    const m = await mount({
      card: async (id) => {
        const d = await base.card(id);
        return d.remind_at ? { ...d, remind_due: true, remind_note: 'the contrast review' } : d;
      },
    });
    m.$$('.card').find((c) => text(c).includes('Add dark mode'))!.click();
    await vi.waitFor(() => expect(m.$('.page .rrow.due')).not.toBeNull());
    expect(text(m.$('.page .rrow'))).toContain('Due');
    expect(text(m.$('.page .rrow'))).toContain('check on this: the contrast review');
  });

  it('is absent when no reminder is set', async () => {
    const m = await mount();
    m.$$('.card').find((c) => text(c).includes('Fix the flaky login'))!.click();
    await vi.waitFor(() => expect(m.$('.page .qmove')).not.toBeNull());
    expect(m.$('.page .rrow')).toBeNull();
  });
});

describe('the COST row', () => {
  it('is a label and value row of WHERE, with the note in the small hint style', async () => {
    const m = await mount();
    session(m).click();
    await vi.waitFor(() => expect(m.$('.page dl.where')).not.toBeNull());
    const dts = m.$$('.page dl.where dt');
    const cost = dts.find((d) => text(d) === 'COST')!;
    expect(cost).toBeDefined();
    expect(text(cost.nextElementSibling)).toMatch(/^\$[\d,.]+ · [\d.]+[kM]? tokens$/);
    expect(text(m.$('.page dl.where + .rhint'))).toBe('API-equivalent estimate, not your bill');
    expect(m.$('.page .ucost')).toBeNull();
    expect(m.$$('.page .sec h4').map(text)).not.toContain('COST');
  });
});

describe('the USAGE rows', () => {
  it('are one grid: name, bar, cost and tokens in every row, so every bar is as wide as the next', async () => {
    const m = await mount({ usage: async (d) => demoUsage(d) });
    [...m.root.querySelectorAll<HTMLButtonElement>('header.hd button')].find((b) => text(b) === 'USAGE')!.click();
    await vi.waitFor(() => expect(m.$('.usage .ubars')).not.toBeNull());
    const lis = m.$$('.usage .ubars li');
    expect(lis.length).toBeGreaterThan(10);
    for (const li of lis) expect([...li.children].map((c) => c.className.split(' ')[0])).toEqual([expect.stringMatching(/uname/), 'utrack', 'uc', 'ut']);
    // projects are the board's names, not folders, and the small ones are one row
    const projects = [...m.$$('.usage .ubars')[0]!.querySelectorAll('.uname')].map(text);
    expect(projects).toContain('agentic-layer-stack');
    expect(projects.at(-1)).toBe('other (6)');
    expect(projects.some((p) => p.startsWith('/') || p.startsWith('~/'))).toBe(false);
  });
});
