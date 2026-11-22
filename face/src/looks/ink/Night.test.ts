// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import type { NightPlanInput, Source } from '../../api/types';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { Root } from './Root';
import { discardText, durationText, nightAsk, nightPanel, runStatus } from './Night';

let mounted: HTMLElement | null = null;

beforeAll(() => {
  keepPrefsLocal();
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); nightAsk.value = null; nightPanel.value = false; });
afterEach(() => {
  if (mounted) { render(null, mounted); mounted.remove(); mounted = null; }
  document.body.innerHTML = '';
});

/** The demo board, with the night calls recorded. */
async function mount(opts: { copyRun?: boolean; configIsolation?: 'copy' } = {}) {
  const base = fixtureSource(true);
  const brings: number[] = [];
  const plans: NightPlanInput[] = [];
  const discards: Array<[number, boolean]> = [];
  const settings: unknown[] = [];
  const src: Source = {
    ...base,
    nightPlan: (i) => { plans.push(i); return base.nightPlan(i); },
    nightDiscard: (id, lose) => { discards.push([id, lose]); return base.nightDiscard(id, lose); },
    nightSettings: (i) => { settings.push(i); return base.nightSettings(i); },
    nightBring: (id) => { brings.push(id); return base.nightBring(id); },
    night: async () => ({ ...(await base.night()), ...(opts.configIsolation ? { isolation: opts.configIsolation } : {}) }),
    card: async (id) => {
      const c = await base.card(id);
      return opts.copyRun && c.night_run ? { ...c, night_run: { ...c.night_run, isolation: 'copy' } } : c;
    },
  };
  const store = createStore(src);
  const root = document.createElement('div');
  document.body.appendChild(root);
  mounted = root;
  render(h(Root, { store }), root);
  await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
  await vi.waitFor(() => expect(store.night.value).not.toBeNull());
  const $ = <T extends Element = HTMLElement>(sel: string) => root.querySelector<T>(sel);
  const $$ = <T extends Element = HTMLElement>(sel: string) => [...root.querySelectorAll<T>(sel)];
  return { store, root, plans, discards, settings, brings, $, $$ };
}

const text = (e: Element | null) => e?.textContent ?? '';
const button = (root: ParentNode, label: string) => [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => text(b).trim() === label);
const type = (el: HTMLInputElement | HTMLTextAreaElement | null, v: string) => { el!.value = v; el!.dispatchEvent(new Event('input', { bubbles: true })); };

type M = Awaited<ReturnType<typeof mount>>;

async function addIdea(m: M, over: { prompt?: string; folder?: string } = {}) {
  const card = await m.store.addIdea({ title: 'Night idea', prompt: 'prompt' in over ? over.prompt : 'Add a README line', folder: over.folder ?? '~/code/demo' });
  expect(card).not.toBeNull();
  await vi.waitFor(() => expect(m.$$('.card.idea').some((c) => text(c).includes('Night idea'))).toBe(true));
  return card!;
}

describe('the words', () => {
  it('durations and statuses read plainly', () => {
    expect(durationText(30_000)).toBe('under 1 min');
    expect(durationText(41 * 60_000)).toBe('41 min');
    expect(durationText(65 * 60_000)).toBe('1 h 5 min');
    const run = { status: 'stopped', why: 'weekly limit 80% reached (81% used)' } as never;
    expect(runStatus(run)).toBe('Stopped: weekly limit 80% reached (81% used)');
    expect(runStatus({ status: 'planned', why: null } as never)).toBe('Planned');
  });
  it('the discard question names the branch and warns about uncommitted work', () => {
    const base = { branch: 'night/20261121-x', uncommitted: false } as never;
    expect(discardText(base)).toBe('Remove the worktree and the branch night/20261121-x? Commits that are only on that branch are lost.');
    expect(discardText({ ...(base as object), uncommitted: true } as never)).toContain('uncommitted changes that will be lost too');
    expect(discardText({ branch: null, uncommitted: false } as never)).toBe('Remove the copy of the folder?');
    expect(discardText({ ...(base as object), isolation: 'copy' } as never)).toContain('Remove the copy and its branch night/20261121-x?');
  });
});

describe('the Tonight chip and list', () => {
  it('no chip while nothing is planned', async () => {
    const m = await mount();
    expect(m.$('.hd .tonight')).toBeNull();
  });

  it('planning an idea from its panel shows "Tonight: 1", and the list opens, changes the limit, cancels and switches on', async () => {
    const m = await mount();
    const idea = await addIdea(m);
    m.$$('.card.idea').find((c) => text(c).includes('Night idea'))!.querySelector<HTMLButtonElement>('.btn.ghost')!.click();
    await vi.waitFor(() => expect(m.$('.nplan')).not.toBeNull());
    expect(text(m.$('.nplan'))).toContain('Nothing is pushed');
    type(m.$<HTMLInputElement>('.nplan input:not([type=checkbox])'), '2M');
    button(m.root, 'PLAN FOR TONIGHT')!.click();
    await vi.waitFor(() => expect(m.plans).toEqual([{ idea_id: idea.idea_id, limit: '2M', agent: undefined, switch: false, isolation: 'worktree' }]));
    await vi.waitFor(() => expect(text(m.$('.hd .tonight'))).toBe('Tonight: 1'));
    // the panel now says it is planned
    await vi.waitFor(() => expect(text(m.$('.page'))).toContain('Planned for tonight, limit 2M'));
    // the list
    m.$<HTMLButtonElement>('.hd .tonight')!.click();
    await vi.waitFor(() => expect(m.$('[aria-label="Tonight"]')).not.toBeNull());
    expect(text(button(m.root, 'NIGHT SHIFT OFF') ?? null)).toBe('NIGHT SHIFT OFF');
    expect(text(m.$('.trow'))).toContain('limit 2M');
    button(m.root, 'CHANGE LIMIT')!.click();
    await vi.waitFor(() => expect(m.$('.nlim')).not.toBeNull());
    type(m.$<HTMLInputElement>('.nlim input'), '90m');
    button(m.root, 'SAVE')!.click();
    await vi.waitFor(() => expect(text(m.$('.trow .ns'))).toContain('limit 90m'));
    button(m.root, 'NIGHT SHIFT OFF')!.click();
    await vi.waitFor(() => expect(button(m.root, 'NIGHT SHIFT ON')).toBeDefined());
    expect(m.settings).toEqual([{ enabled: true }]);
    button(m.root, 'CANCEL RUN')!.click();
    await vi.waitFor(() => expect(m.$('.hd .tonight')).toBeNull());
    expect(text(m.$('.nlist, .page.night .dim'))).toContain('Nothing planned');
  });

  it('a bad window is refused with the reason, and a good one is saved', async () => {
    const m = await mount();
    await addIdea(m);
    await m.store.nightPlan({ idea_id: 1 }).catch(() => {});
    await m.store.nightPlan({ idea_id: (await m.store.addIdea({ title: 'x', prompt: 'p', folder: '~/code/x' }))!.idea_id! });
    await vi.waitFor(() => expect(m.$('.hd .tonight')).not.toBeNull());
    m.$<HTMLButtonElement>('.hd .tonight')!.click();
    await vi.waitFor(() => expect(m.$('.nset input')).not.toBeNull());
    type(m.$<HTMLInputElement>('.nset input'), '7-9');
    button(m.root, 'SET WINDOW')!.click();
    await vi.waitFor(() => expect(m.$$('.toast, [role=status]').map(text).join(' ')).toContain('is not a window'));
    type(m.$<HTMLInputElement>('.nset input'), '23:00-05:00');
    button(m.root, 'SET WINDOW')!.click();
    await vi.waitFor(() => expect(m.settings.at(-1)).toEqual({ window: '23:00-05:00' }));
    await vi.waitFor(() => expect(m.store.night.value?.window).toBe('23:00-05:00'));
  });

  it('an idea without a prompt is refused in words, and nothing is planned', async () => {
    const m = await mount();
    const idea = await addIdea(m, { prompt: '' });
    await m.store.nightPlan({ idea_id: idea.idea_id! });
    await vi.waitFor(() => expect(m.$$('.toast, [role=status]').map(text).join(' ')).toContain('has no prompt'));
    expect(m.$('.hd .tonight')).toBeNull();
  });
});

describe('the TONIGHT switch in the idea form', () => {
  it('saving with TONIGHT checked plans the new idea with its limit', async () => {
    const m = await mount();
    button(m.root, '+ IDEA')!.click();
    await vi.waitFor(() => expect(m.$('form.modal')).not.toBeNull());
    type(m.$<HTMLInputElement>('input[name=title]'), 'From the form');
    type(m.$<HTMLTextAreaElement>('textarea[name=prompt]'), 'Do the thing');
    type(m.$<HTMLInputElement>('input[name=folder]'), '~/code/demo');
    [...m.$$('form.modal .pick button')].find((b) => text(b) === 'Run it tonight')!.click();
    await vi.waitFor(() => expect(m.$('input[name=limit]')).not.toBeNull());
    type(m.$<HTMLInputElement>('input[name=limit]'), '45m');
    m.$<HTMLFormElement>('form.modal')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(m.plans.length).toBe(1));
    expect(m.plans[0]).toMatchObject({ limit: '45m', switch: false });
    await vi.waitFor(() => expect(text(m.$('.hd .tonight'))).toBe('Tonight: 1'));
  });
});

describe('CONTINUE TONIGHT', () => {
  async function openSession(m: M) {
    const card = m.$$('.card').find((c) => !c.classList.contains('idea') && !c.querySelector('.mk.night'))!;
    card.click();
    await vi.waitFor(() => expect(m.$('.page')).not.toBeNull());
    await vi.waitFor(() => expect(button(m.root, 'CONTINUE TONIGHT…')).toBeDefined());
  }

  it('needs a prompt, then plans the session', async () => {
    const m = await mount();
    await openSession(m);
    button(m.$('.page')!, 'CONTINUE TONIGHT…')!.click();
    await vi.waitFor(() => expect(m.$('form.nplan textarea')).not.toBeNull());
    m.$<HTMLFormElement>('form.nplan')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(m.$$('.toast, [role=status]').map(text).join(' ')).toContain('needs a prompt'));
    type(m.$<HTMLTextAreaElement>('form.nplan textarea'), 'Finish the failing tests');
    type(m.$<HTMLInputElement>('form.nplan input:not([type=checkbox])'), '30m');
    m.$<HTMLFormElement>('form.nplan')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(m.plans.at(-1)).toMatchObject({ prompt: 'Finish the failing tests', limit: '30m' }));
    expect(m.plans.at(-1)!.session_id).toBeTruthy();
    await vi.waitFor(() => expect(text(m.$('.hd .tonight'))).toBe('Tonight: 1'));
  });

  it('opens at the top of the panel, above every section that loads later', async () => {
    const m = await mount();
    await openSession(m);
    button(m.$('.page')!, 'CONTINUE TONIGHT…')!.click();
    await vi.waitFor(() => expect(m.$('form.nplan')).not.toBeNull());
    const secs = m.$$('.page section.sec');
    expect(secs[0]!.querySelector('h4')!.textContent).toBe('CONTINUE TONIGHT');
    expect(secs[0]!.contains(m.$('form.nplan'))).toBe(true);
  });

  it('a weekly limit eye cannot check is refused with the reason', async () => {
    const m = await mount();
    await openSession(m);
    button(m.$('.page')!, 'CONTINUE TONIGHT…')!.click();
    await vi.waitFor(() => expect(m.$('form.nplan textarea')).not.toBeNull());
    type(m.$<HTMLTextAreaElement>('form.nplan textarea'), 'go');
    type(m.$<HTMLInputElement>('form.nplan input:not([type=checkbox])'), '80%');
    m.$<HTMLFormElement>('form.nplan')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(m.$$('.toast, [role=status]').map(text).join(' ')).toContain('no weekly plan data'));
    expect(m.$('.hd .tonight')).toBeNull();
  });
});

describe('the morning card and the NIGHT RUN section', () => {
  async function openNight(m: M) {
    const card = m.$$('.card').find((c) => c.querySelector('.mk.night'))!;
    expect(card).toBeDefined();
    card.click();
    await vi.waitFor(() => expect(m.$('.nrun')).not.toBeNull());
  }

  it('a card made by a night run has a calm NIGHT chip, and no other card does', async () => {
    const m = await mount();
    expect(m.$$('.card .mk.night').map(text)).toEqual(['night']);
  });

  it('shows status, times, tokens, limit, report, diff stat, branch and the three buttons', async () => {
    const m = await mount();
    await openNight(m);
    const dl = text(m.$('.nrun'));
    for (const w of ['Done', 'Started', 'Ended', '41 min', 'Tokens', '90 minutes: not reached']) expect(dl).toContain(w);
    expect(text(m.$('.nreport'))).toContain('Added the README line');
    expect(text(m.$$('.nreport')[1]!)).toContain('2 files changed');
    expect(text(m.$('.nwhere'))).toContain('night/20261121-add-a-readme-line');
    const labels = [...m.$('.page')!.querySelectorAll('button')].map(text);
    expect(labels).toEqual(expect.arrayContaining(['RESUME', 'OPEN FOLDER', 'DISCARD']));
  });

  it('the report is shown as text, never as markup', async () => {
    const m = await mount();
    await openNight(m);
    const pre = m.$('.nreport')!;
    pre.textContent = '<img src=x onerror=alert(1)> <b>bold</b>';
    expect(pre.querySelector('img, b')).toBeNull();
  });

  it('DISCARD asks first in the page, a cancel changes nothing, a yes calls the route', async () => {
    const m = await mount();
    await openNight(m);
    button(m.$('.page')!, 'DISCARD')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).not.toBeNull());
    expect(text(m.$('.lask p'))).toContain('Remove the worktree and the branch night/20261121-add-a-readme-line');
    expect(m.discards).toEqual([]);
    button(m.$('.lask')!, 'CANCEL')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).toBeNull());
    expect(m.discards).toEqual([]);
    button(m.$('.page')!, 'DISCARD')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).not.toBeNull());
    button(m.$('.lask')!, 'YES')!.click();
    await vi.waitFor(() => expect(m.discards).toEqual([[1, false]]));
    await vi.waitFor(() => expect(text(m.$('.page'))).toContain('Discarded'));
  });
});

describe('the two ways to keep your folder safe', () => {
  it('the form offers both, starts on the config value and sends the choice', async () => {
    const m = await mount();
    const idea = await addIdea(m);
    m.$$('.card.idea').find((c) => text(c).includes('Night idea'))!.querySelector<HTMLButtonElement>('.btn.ghost')!.click();
    await vi.waitFor(() => expect(m.$('.nplan')).not.toBeNull());
    const choices = [...m.$('.nplan')!.querySelectorAll<HTMLButtonElement>('fieldset.pick button')].filter((b) => /worktree|copy/.test(text(b)));
    expect(choices.map(text)).toEqual(['A worktree of your repo (default)', 'A separate copy (your folder fully locked)']);
    expect(choices.map((b) => b.getAttribute('aria-pressed'))).toEqual(['true', 'false']);
    choices[1]!.click();
    await vi.waitFor(() => expect(choices[1]!.getAttribute('aria-pressed')).toBe('true'));
    button(m.root, 'PLAN FOR TONIGHT')!.click();
    await vi.waitFor(() => expect(m.plans.at(-1)).toMatchObject({ idea_id: idea.idea_id, isolation: 'copy' }));
    await vi.waitFor(() => expect(m.store.night.value?.open[0]?.isolation).toBe('copy'));
  });

  it('the config value is the starting choice', async () => {
    const m = await mount({ configIsolation: 'copy' });
    await addIdea(m);
    m.$$('.card.idea').find((c) => text(c).includes('Night idea'))!.querySelector<HTMLButtonElement>('.btn.ghost')!.click();
    await vi.waitFor(() => expect(m.$('.nplan')).not.toBeNull());
    const pressed = [...m.$('.nplan')!.querySelectorAll<HTMLButtonElement>('fieldset.pick button[aria-pressed=true]')].map(text);
    expect(pressed).toContain('A separate copy (your folder fully locked)');
    button(m.root, 'PLAN FOR TONIGHT')!.click();
    await vi.waitFor(() => expect(m.plans.at(-1)).toMatchObject({ isolation: 'copy' }));
  });

  async function openRun(m: M) {
    m.$$('.card').find((c) => c.querySelector('.mk.night'))!.click();
    await vi.waitFor(() => expect(m.$('.nrun')).not.toBeNull());
  }

  it('the NIGHT RUN section says which one a run used; a worktree run has no BRING button', async () => {
    const m = await mount();
    await openRun(m);
    expect(text(m.$('.nrun'))).toContain('Worked in');
    expect(text(m.$('.nrun'))).toContain('A worktree of your repo');
    expect(button(m.$('.page')!, 'BRING INTO MY REPO')).toBeUndefined();
  });

  it('BRING INTO MY REPO asks in the page first, and a yes calls the route', async () => {
    const m = await mount({ copyRun: true });
    await openRun(m);
    expect(text(m.$('.nrun'))).toContain('A separate copy (your folder was fully locked)');
    button(m.$('.page')!, 'BRING INTO MY REPO')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).not.toBeNull());
    expect(text(m.$('.lask p'))).toContain('night/20261121-add-a-readme-line');
    expect(m.brings).toEqual([]);
    button(m.$('.lask')!, 'CANCEL')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).toBeNull());
    expect(m.brings).toEqual([]);
    button(m.$('.page')!, 'BRING INTO MY REPO')!.click();
    await vi.waitFor(() => expect(m.$('.lask')).not.toBeNull());
    button(m.$('.lask')!, 'YES')!.click();
    await vi.waitFor(() => expect(m.brings).toEqual([1]));
  });
});
