// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { h, render } from 'preact';
import { aimAt, REACH_X, REACH_Y } from './eyeFollow';
import { Eye } from './Eye';

describe('where the iris looks', () => {
  it('reaches the side of the almond 160 px away and the top 70 px away, never further', () => {
    expect(aimAt(160, 0)).toEqual({ x: REACH_X, y: 0 });
    expect(aimAt(-1000, 0).x).toBe(-REACH_X);
    expect(aimAt(0, 35).y).toBeCloseTo(REACH_Y / 2, 6);
    expect(aimAt(0, -500).y).toBe(-REACH_Y);
  });

  it('looks straight ahead at a point on its centre', () => {
    expect(aimAt(0, 0)).toEqual({ x: 0, y: 0 });
  });
});

describe('the eye in the header', () => {
  let root: HTMLElement | null = null;
  afterEach(() => { if (root) { render(null, root); root.remove(); root = null; } });

  it('turns toward the pointer, eased over a few frames, and has lashes for when something needs you', async () => {
    root = document.createElement('div');
    document.body.appendChild(root);
    render(h(Eye, { alert: true }), root);
    const svg = root.querySelector('svg.eye')!;
    expect(svg.classList.contains('alert')).toBe(true);
    expect(svg.querySelector('.lash')).not.toBeNull();
    dispatchEvent(new PointerEvent('pointermove', { clientX: 400, clientY: 0 }));
    const x = () => Number(/translate\(([-\d.]+)/.exec(svg.querySelector('.look')!.getAttribute('transform') ?? '')?.[1] ?? 0);
    await vi.waitFor(() => expect(x()).toBeGreaterThan(REACH_X / 2));
    expect(x()).toBeLessThanOrEqual(REACH_X);
  });
});
