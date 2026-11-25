// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { aim, watchGaze, type Gaze } from './gaze';

const motion = (reduce: boolean) => vi.spyOn(globalThis, 'matchMedia').mockImplementation((q: string) => ({ matches: reduce && q.includes('reduce') }) as MediaQueryList);

describe('where an eye aims', () => {
  it('reaches its side 160 px away and its top 70 px away, never further', () => {
    expect(aim(160, 0)).toEqual({ x: 1, y: 0 });
    expect(aim(-1000, 0).x).toBe(-1);
    expect(aim(0, 35).y).toBeCloseTo(0.5, 6);
    expect(aim(0, -500).y).toBe(-1);
    expect(aim(0, 0)).toEqual({ x: 0, y: 0 });
  });
});

describe('an eye watching the pointer', () => {
  beforeEach(() => vi.useFakeTimers({ toFake: ['requestAnimationFrame', 'cancelAnimationFrame', 'performance', 'setTimeout'] }));
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

  const watch = (blinks = { ms: 200, gap: 2000 }) => {
    const seen: Gaze[] = [];
    const stop = watchGaze(document.body, (g) => seen.push(g), blinks);
    return { seen, stop };
  };

  it('blinks after a while, then opens again', () => {
    motion(false);
    const { seen, stop } = watch();
    vi.advanceTimersByTime(1700);
    expect(seen.every((g) => g.blink === -1)).toBe(true);
    // the first blink comes at 1.8 s and takes 200 ms; the next one is at least 2 s later
    vi.advanceTimersByTime(600);
    expect(seen.some((g) => g.blink > 0 && g.blink < 1)).toBe(true);
    expect(seen.at(-1)!.blink).toBe(-1);
    stop();
  });

  it('never blinks with reduced motion', () => {
    motion(true);
    const { seen, stop } = watch();
    vi.advanceTimersByTime(10_000);
    expect(seen.length).toBeGreaterThan(100);
    expect(seen.every((g) => g.blink === -1 && g.reduced)).toBe(true);
    stop();
  });

  it('stops: no more frames and no pointer listener once stopped', () => {
    motion(false);
    const off = vi.spyOn(globalThis, 'removeEventListener');
    const { seen, stop } = watch();
    vi.advanceTimersByTime(100);
    stop();
    const n = seen.length;
    vi.advanceTimersByTime(1000);
    expect(seen.length).toBe(n);
    expect(off).toHaveBeenCalledWith('pointermove', expect.any(Function));
  });
});
