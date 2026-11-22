// @vitest-environment happy-dom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import { fixtureSource } from '../../data/fixture';
import { keepPrefsLocal } from '../../prefs';
import { createStore } from '../../store';
import { forgetFor, forgetText } from './Forget';
import { Root } from './Root';

let mounted: HTMLElement | null = null;

async function mount() {
  const src = fixtureSource();
  const forget = vi.fn(src.forget);
  const store = createStore({ ...src, forget });
  const root = document.createElement('div');
  document.body.appendChild(root);
  mounted = root;
  render(h(Root, { store }), root);
  await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBeGreaterThan(5));
  return { store, root, forget };
}

const contextMenu = (el: Element) => el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 40, clientY: 40 }));
const item = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('.ctx button')].find((b) => b.textContent === label);

beforeAll(() => {
  keepPrefsLocal();
  Element.prototype.scrollIntoView ??= () => {};
  Element.prototype.scrollTo ??= () => {};
});
beforeEach(() => { localStorage.clear(); });
afterEach(() => {
  if (mounted) { render(null, mounted); mounted.remove(); mounted = null; }
  forgetFor.value = null;
});

describe('FORGET…', () => {
  it('says exactly what goes, and the transcript only when the box is ticked', () => {
    expect(forgetText('Claude Code', true)).toBe('Remove this session from eye, and its transcript from Claude Code? This cannot be undone.');
    expect(forgetText('Claude Code', false)).toBe('Remove this session from eye? This cannot be undone.');
  });

  it('asks first: nothing is sent until FORGET is pressed, and the box starts off', async () => {
    const { root, forget } = await mount();
    contextMenu(root.querySelector<HTMLElement>('.card:not(.idea)')!);
    await vi.waitFor(() => expect(item('FORGET…')).toBeTruthy());
    item('FORGET…')!.click();
    await vi.waitFor(() => expect(root.querySelector('.forget')).toBeTruthy());
    expect(forget).not.toHaveBeenCalled();
    expect(root.querySelector<HTMLInputElement>('.forget input[type=checkbox]')!.checked).toBe(false);
    expect(root.querySelector('#forget-text')!.textContent).toBe('Remove this session from eye? This cannot be undone.');
    root.querySelector<HTMLButtonElement>('.forget .ghost')!.click();
    await vi.waitFor(() => expect(root.querySelector('.forget')).toBeNull());
    expect(forget).not.toHaveBeenCalled();
  });

  it('sends the confirm with the transcript choice, and the card leaves the board', async () => {
    const { root, forget } = await mount();
    const before = root.querySelectorAll('.card').length;
    contextMenu(root.querySelector<HTMLElement>('.card:not(.idea)')!);
    await vi.waitFor(() => expect(item('FORGET…')).toBeTruthy());
    item('FORGET…')!.click();
    await vi.waitFor(() => expect(root.querySelector('.forget')).toBeTruthy());
    const box = root.querySelector<HTMLInputElement>('.forget input[type=checkbox]')!;
    box.checked = true;
    box.dispatchEvent(new Event('change', { bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('#forget-text')!.textContent).toContain('and its transcript from'));
    root.querySelector<HTMLFormElement>('.forget')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(forget).toHaveBeenCalledTimes(1));
    expect(forget.mock.calls[0]![1]).toBe(true);
    await vi.waitFor(() => expect(root.querySelectorAll('.card').length).toBe(before - 1));
    await vi.waitFor(() => expect(root.querySelector('.forget')).toBeNull());
  });

  it('opens from the panel with X, and Esc closes it', async () => {
    const { store, root, forget } = await mount();
    root.querySelector<HTMLElement>('.card:not(.idea)')!.click();
    await vi.waitFor(() => expect(store.detail.value).toBeTruthy());
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'x', bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.forget')).toBeTruthy());
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await vi.waitFor(() => expect(root.querySelector('.forget')).toBeNull());
    expect(forget).not.toHaveBeenCalled();
    expect(store.selected.value).toBeTruthy();
  });
});
