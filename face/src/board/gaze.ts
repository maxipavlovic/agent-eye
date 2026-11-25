/** How long the pointer rests before an eye looks somewhere else, in ms. */
const IDLE_MS = 2500;

/** The Needs you lane on the page: the header's button brings it into view, and the eyes look at it while it waits. */
export const needsLane = (): Element | null => document.querySelector('[data-col="needs_you"]');

/**
 * Where an eye aims for a gaze point `dx`, `dy` CSS px from its centre, as -1..1 of its reach: the full reach
 * 160 px to the side or 70 px up or down.
 */
export function aim(dx: number, dy: number): { x: number; y: number } {
  const clamp = (v: number) => Math.max(-1, Math.min(1, v));
  return { x: clamp(dx / 160), y: clamp(dy / 70) };
}

/** One frame of an eye: where it aims (`aim`), whether the pointer is close, and how far into a blink it is. */
export interface Gaze {
  x: number;
  y: number;
  near: boolean;
  /** 0 to 1 through a blink, -1 between blinks. */
  blink: number;
  reduced: boolean;
}

/** A look's blink: how long one takes, the shortest gap to the next (up to 4.2 s more at random), and the chance of a second one right after. */
export interface Blinks {
  ms: number;
  gap: number;
  twice?: number;
}

/**
 * Watches the pointer for the eye `el` and hands `draw` a `Gaze` every frame; the look draws it its own way.
 * The eye follows the pointer; when the pointer rests it looks at `idleTarget()` (Needs you while something
 * waits there), else it drifts; now and then it blinks. With reduced motion it never drifts or blinks.
 * Returns the stop.
 */
export function watchGaze(el: Element, draw: (g: Gaze) => void, blinks: Blinks, idleTarget: () => Element | null = () => null): () => void {
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let tx = 0, ty = 0, moved = -1e9, raf = 0;
  let blinkAt = performance.now() + 1800, blinkStart = -1, again = false;
  const onMove = (e: PointerEvent) => { tx = e.clientX; ty = e.clientY; moved = performance.now(); };
  addEventListener('pointermove', onMove);

  const tick = (t: number) => {
    const r = el.getBoundingClientRect();
    const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    const resting = t - moved > IDLE_MS;
    let gx = tx, gy = ty;
    if (resting) {
      const target = idleTarget();
      if (target) { const b = target.getBoundingClientRect(); gx = b.left + b.width / 2; gy = b.top + 80; }
      else if (reduced) { gx = cx; gy = cy; }
      else { gx = cx + Math.sin(t / 2300) * 220; gy = cy + Math.sin(t / 3100) * 30; }
    }
    let blink = -1;
    if (!reduced) {
      if (blinkStart < 0 && t >= blinkAt) blinkStart = t;
      if (blinkStart >= 0) {
        blink = (t - blinkStart) / blinks.ms;
        if (blink >= 1) {
          blink = -1;
          blinkStart = -1;
          // now and then a second blink right after the first
          if (!again && Math.random() < (blinks.twice ?? 0)) { again = true; blinkAt = t + 120; } else { again = false; blinkAt = t + blinks.gap + Math.random() * 4200; }
        }
      }
    }
    // the pupil may narrow when the pointer comes close, as in bright light
    draw({ ...aim(gx - cx, gy - cy), near: !resting && Math.hypot(tx - cx, ty - cy) < 90, blink, reduced });
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
  return () => { cancelAnimationFrame(raf); removeEventListener('pointermove', onMove); };
}
