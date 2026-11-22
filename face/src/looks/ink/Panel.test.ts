// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { Source } from '../../api/types';
import { demoUsage } from '../../data/usageSample';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { IDEA_KEY_LIST, keyHelp, KEY_LIST } from './KeyHelp';
import { closeForget } from './Forget';
import { ideaForm } from './IdeaForm';
import { nightAsk, tonightOpen } from './Night';
import { remindForm } from './Remind';
import { usagePanel } from './Usage';
import { resetFolds } from './Sections';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  globalThis.matchMedia ??= ((q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as typeof matchMedia;
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); resetFolds(); closeForget(); tonightOpen.value = null; nightAsk.value = null; ideaForm.value = null; keyHelp.value = false; remindForm.value = null; usagePanel.value = false; });
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
const session = (m: Awaited<ReturnType<typeof mount>>) => m.$$('.card').find((c) => !c.classList.contains('idea') && !c.classList.contains('row') && !c.querySelector('.tab.night'))!;
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
    expect(text(m.$('.page section.sec h4'))).toBe('CONTINUE TONIGHT');
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

  it('shows each key on its button, the way RESUME shows R, and beside each action in MORE', async () => {
    const m = await open();
    const keyed = (sel: string) => text(m.$(`${sel} u`));
    expect(keyed('.qmove .resume')).toBe('R');
    expect(m.$$('.qmove button[aria-keyshortcuts] u').map(text)).toEqual(['N', 'W', 'P', 'D', 'R']);
    [...m.$$<HTMLButtonElement>('.page .ptop button')].find((b) => text(b) === 'MORE ▾')!.click();
    await vi.waitFor(() => expect(m.$('.page [role=menu]')).not.toBeNull());
    expect(m.$$('.page [role=menuitem] kbd').map(text)).toEqual(['F', 'C', 'E', 'T', 'M', 'H', 'X']);
  });

  it('MORE holds the other actions; a choice runs and closes it, and Esc closes it without closing the panel', async () => {
    const m = await open();
    const more = () => [...m.$$<HTMLButtonElement>('.page .ptop button')].find((b) => text(b) === 'MORE ▾')!;
    more().click();
    await vi.waitFor(() => expect(m.$('.page [role=menu]')).not.toBeNull());
    expect(m.$$('.page [role=menuitem] span').map(text)).toEqual(['FORK', 'COPY RESUME COMMAND', 'EXPORT', 'CONTINUE TONIGHT…', 'REMIND ME…', 'HIDE', 'FORGET…', 'ARCHIVE']);
    press('Escape');
    await vi.waitFor(() => expect(m.$('.page [role=menu]')).toBeNull());
    expect(m.store.selected.value).not.toBeNull();
    more().click();
    await vi.waitFor(() => expect(m.$('.page [role=menu]')).not.toBeNull());
    [...m.$$<HTMLButtonElement>('.page [role=menuitem]')].find((b) => text(b.querySelector('span')) === 'CONTINUE TONIGHT…')!.click();
    await vi.waitFor(() => expect(m.$('.page form.nplan')).not.toBeNull());
    expect(m.$('.page [role=menu]')).toBeNull();
  });

  it('folds only the long or seldom needed parts, ends with the receipt and the actions, and keeps a section you open open', async () => {
    const m = await open();
    expect(m.$$('.page .details .fsec').map((x) => x.id)).toEqual(['conversation', 'tokens', 'reminder']);
    expect(m.$$('.page .details .fsec.open').length).toBe(0);
    // WHERE and FIRST PROMPT stay open; the receipt and the action row end the panel
    expect(m.$('.page dl.where')).not.toBeNull();
    const last = [...m.$('.page')!.children].slice(-2).map((e) => e.className);
    expect(last).toEqual(['receipt', 'acts end']);
    m.$<HTMLButtonElement>('#tokens .fhead')!.click();
    await vi.waitFor(() => expect(m.$('#tokens.open')).not.toBeNull());
    m.store.close();
    await vi.waitFor(() => expect(m.$('.page')).toBeNull());
    session(m).click();
    await vi.waitFor(() => expect(m.$('.page #tokens')).not.toBeNull());
    expect(m.$('#tokens')!.classList.contains('open')).toBe(true);
  });

  it('? opens the list of keys, and ?, Esc or CLOSE closes it without closing the panel', async () => {
    const m = await open();
    press('?', { shiftKey: true });
    await vi.waitFor(() => expect(m.$('.keyhelp')).not.toBeNull());
    const shown = m.$$('.keyhelp dt').map(text);
    // a session's panel lists every key, X (forget) included
    for (const [k] of KEY_LIST) expect(shown).toContain(k);
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

describe("an idea's panel", () => {
  const openIdea = async (title: string, over: Partial<Source> = {}) => {
    const m = await mount(over);
    m.$$('.card.idea').find((c) => text(c).includes(title))!.click();
    await vi.waitFor(() => expect(m.$('.page .acts')).not.toBeNull());
    return m;
  };
  const started = () => {
    const calls: Array<[number, boolean]> = [];
    const startIdea: Source['startIdea'] = async (id, background) => {
      calls.push([id, background]);
      return { dry_run: true, mode: 'bg', command: 'claude', note: '', session_id: null, job: null, creates_folder: false };
    };
    return { calls, startIdea };
  };

  it('has START, IN BACKGROUND, TONIGHT and EDIT in one row, and REMIND ME and ARCHIVE in MORE', async () => {
    const m = await openIdea('Shortcut list');
    const rows = m.$$('.page > .acts');
    expect(rows.length).toBe(1);
    expect(m.$$('.page > .acts .btn').map(text)).toEqual(['START', 'IN BACKGROUND', 'TONIGHT', 'EDIT']);
    m.$$('.page .ptop button').find((b) => text(b) === 'MORE ▾')!.click();
    await vi.waitFor(() => expect(m.$('.page .more .menu')).not.toBeNull());
    expect(m.$$('.page .more [role=menuitem] span').map(text)).toEqual(['REMIND ME…', 'ARCHIVE']);
  });

  it('S starts it, B starts it in the background, E edits it and T opens the plan for tonight', async () => {
    const s = started();
    const m = await openIdea('Shortcut list', { startIdea: s.startIdea });
    const id = m.store.detail.value!.idea_id!;
    press('s');
    await vi.waitFor(() => expect(s.calls).toEqual([[id, false]]));
    await tick();
    press('b');
    await vi.waitFor(() => expect(s.calls).toEqual([[id, false], [id, true]]));
    press('t');
    await vi.waitFor(() => expect(m.$('.page form.nplan')).not.toBeNull());
    (document.activeElement as HTMLElement | null)?.blur();
    press('e');
    await vi.waitFor(() => expect(ideaForm.value).toEqual({ edit: m.store.detail.value!.id }));
  });

  it('B on an idea with no prompt opens the form on the prompt instead', async () => {
    const s = started();
    const m = await openIdea('Migration guide', { startIdea: s.startIdea });
    press('b');
    await vi.waitFor(() => expect(ideaForm.value).toEqual({ edit: m.store.detail.value!.id, focus: 'prompt' }));
    expect(s.calls).toEqual([]);
  });

  it('? lists the keys of an idea', async () => {
    const m = await openIdea('Shortcut list');
    press('?', { shiftKey: true });
    await vi.waitFor(() => expect(m.$('.keyhelp')).not.toBeNull());
    expect(m.$$('.keyhelp dt').map(text)).toEqual(IDEA_KEY_LIST.map(([k]) => k));
    expect(text(m.$('.keyhelp .hint'))).toContain("an idea's panel");
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
    // in the status row: right under the action bar (the moves and MORE), before the state line
    expect(m.$('.page .dbar')!.nextElementSibling).toBe(row);
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
