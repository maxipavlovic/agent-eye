// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { h, render, type ComponentChild } from 'preact';
import { addDays, addMonths, dateValue, DatePicker, monthWeeks, parseTime, stepTime, TimeField } from './DatePick';

let root: HTMLElement | null = null;
afterEach(() => { if (root) { render(null, root); root.remove(); root = null; } });

function mount(vnode: ComponentChild) {
  root = document.createElement('div');
  document.body.appendChild(root);
  render(vnode, root);
  return root;
}

const key = (el: Element, k: string) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
const flush = () => new Promise((r) => setTimeout(r, 0));

describe('dates for the picker', () => {
  it('lays a month out in weeks from Monday, blank before the 1st and after the last day', () => {
    const weeks = monthWeeks('2026-11-20');
    expect(weeks).toHaveLength(6);
    expect(weeks[0]).toEqual([null, null, null, null, null, null, '2026-11-01']);
    expect(weeks[5]).toEqual(['2026-11-30', null, null, null, null, null, null]);
    expect(monthWeeks('2027-02-10').flat().filter(Boolean)).toHaveLength(28);
  });

  it('steps days across months and keeps a month step inside a shorter month', () => {
    expect(addDays('2026-11-30', 1)).toBe('2026-12-01');
    expect(addDays('2027-01-01', -1)).toBe('2026-12-31');
    expect(addMonths('2027-01-31', 1)).toBe('2027-02-28');
    expect(addMonths('2026-12-15', 1)).toBe('2027-01-15');
    expect(addMonths('2026-03-31', -1)).toBe('2026-02-28');
    expect(dateValue(new Date(2026, 0, 5))).toBe('2026-01-05');
  });
});

describe('times for the field', () => {
  it('reads a typed time as HH:MM, 24 h, and refuses anything else', () => {
    expect(['9', '930', '9:30', '09:30', '0930', ' 18:05 ', '0', '23:59'].map(parseTime)).toEqual(
      ['09:00', '09:30', '09:30', '09:30', '09:30', '18:05', '00:00', '23:59']);
    expect(['', '24:00', '9:60', '9.30', 'noon', '12:3', '123456'].map(parseTime)).toEqual([null, null, null, null, null, null, null]);
  });

  it('steps to the next quarter hour and stops at the ends of the day', () => {
    expect(stepTime('09:00', 1)).toBe('09:15');
    expect(stepTime('09:07', 1)).toBe('09:15');
    expect(stepTime('09:07', -1)).toBe('09:00');
    expect(stepTime('09:15', -1)).toBe('09:00');
    expect(stepTime('23:30', 1)).toBe('23:45');
    expect(stepTime('23:45', 1)).toBe('23:45');
    expect(stepTime('23:50', 1)).toBe('23:45');
    expect(stepTime('00:00', -1)).toBe('00:00');
    expect(stepTime('00:10', -1)).toBe('00:00');
    expect(stepTime('soon', 1)).toBe('09:15');
  });
});

describe('the date picker', () => {
  const today = dateValue(new Date());
  const show = (value = addDays(today, 1), onChange = vi.fn()) => {
    const el = mount(h(DatePicker, { value, min: today, onChange, labelId: 'd' }));
    const focused = () => el.querySelector<HTMLButtonElement>('.dpgrid button[tabindex="0"]')!;
    return { el, onChange, focused, month: () => el.querySelector('.dphead span')!.textContent };
  };

  it('marks today, the chosen day and the days that are past', () => {
    const { el } = show();
    const cur = el.querySelector('[aria-current="date"]')!;
    expect(cur.classList.contains('today')).toBe(true);
    expect(el.querySelectorAll('td[aria-selected="true"]')).toHaveLength(1);
    expect(el.querySelector('td[aria-selected="true"] button')!.classList.contains('on')).toBe(true);
    const past = [...el.querySelectorAll<HTMLButtonElement>('.dpgrid button')].filter((b) => b.disabled);
    expect(past).toHaveLength(new Date().getDate() - 1);
    expect(el.querySelector('.dphead button')!.hasAttribute('disabled')).toBe(true);
    expect(el.querySelector('table')!.getAttribute('aria-labelledby')).toBe('d d-month');
  });

  it('moves a day, a week and a month on the keys, and keeps them to itself', async () => {
    const board = vi.fn();
    addEventListener('keydown', board);
    const start = addDays(today, 1);
    const { el, focused, month } = show(start);
    const grid = el.querySelector('table')!;
    key(grid, 'ArrowRight');
    await flush();
    expect(focused().getAttribute('aria-label')).toBe(new Date(`${addDays(start, 1)}T12:00`).toLocaleDateString('en-GB', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }));
    await vi.waitFor(() => expect(document.activeElement).toBe(focused()));
    key(grid, 'ArrowDown');
    key(grid, 'PageDown');
    await flush();
    expect(month()).toBe(new Date(`${addMonths(addDays(start, 8), 1)}T12:00`).toLocaleDateString('en-GB', { month: 'long', year: 'numeric' }).toUpperCase());
    expect(board).not.toHaveBeenCalled();
    removeEventListener('keydown', board);
  });

  it('never moves before the first day it may pick', async () => {
    const { el, focused } = show(today);
    key(el.querySelector('table')!, 'PageUp');
    key(el.querySelector('table')!, 'ArrowLeft');
    await flush();
    expect(focused().classList.contains('today')).toBe(true);
  });

  it('picks the day that is clicked', () => {
    const { el, onChange } = show();
    const day = [...el.querySelectorAll<HTMLButtonElement>('.dpgrid button')].filter((b) => !b.disabled).at(-1)!;
    day.click();
    expect(onChange).toHaveBeenCalledWith(expect.stringMatching(/^\d{4}-\d\d-\d\d$/));
    expect(Number(onChange.mock.calls[0]![0].slice(8))).toBe(Number(day.textContent));
  });
});

describe('the time field', () => {
  it('steps a quarter hour on the arrow keys and the buttons, and sets a quick time', () => {
    const onChange = vi.fn();
    const el = mount(h(TimeField, { id: 't', value: '09:00', onChange, invalid: false }));
    key(el.querySelector('input')!, 'ArrowUp');
    key(el.querySelector('input')!, 'ArrowDown');
    el.querySelector<HTMLButtonElement>('[aria-label="15 minutes later"]')!.click();
    [...el.querySelectorAll<HTMLButtonElement>('.tchips button')].find((b) => b.textContent === '15:00')!.click();
    expect(onChange.mock.calls.map((c) => c[0])).toEqual(['09:15', '08:45', '09:15', '15:00']);
    expect(el.querySelector('.tchips [aria-pressed="true"]')!.textContent).toBe('09:00');
  });

  it('says when the time does not read', () => {
    const el = mount(h(TimeField, { id: 't', value: '25:00', onChange: () => {}, invalid: true }));
    expect(el.querySelector('input')!.getAttribute('aria-invalid')).toBe('true');
    expect(el.querySelector('input')!.getAttribute('aria-describedby')).toBe('t-err t-hint');
  });
});
