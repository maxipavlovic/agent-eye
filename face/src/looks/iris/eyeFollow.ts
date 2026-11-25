/** How far the iris may move inside the almond, in drawing units. */
export const REACH_X = 9, REACH_Y = 3.2;
/** How long the pointer rests before the eye looks somewhere else, in ms. */
const IDLE_MS = 2500;
/** One blink, closing and opening, in ms. */
const BLINK_MS = 230;

/**
 * Where the iris sits for a gaze point `dx`, `dy` CSS px from the eye's centre: full reach 160 px to the side
 * or 70 px up or down, as ink's eye.
 */
export function aimAt(dx: number, dy: number): { x: number; y: number } {
  const clamp = (v: number) => Math.max(-1, Math.min(1, v));
  return { x: clamp(dx / 160) * REACH_X, y: clamp(dy / 70) * REACH_Y };
}

/**
 * Brings an iris `svg.eye` to life: the iris eases toward the pointer, and its pupil narrows when the pointer
 * comes close; when the pointer rests, it looks at `idleTarget()` (Needs you while something waits) or drifts;
 * now and then it blinks, sometimes twice. With reduced motion it jumps to where it looks, never drifts and never
 * blinks. Returns the stop.
 */
export function followEye(svg: SVGSVGElement, idleTarget: () => Element | null = () => null): () => void {
  const look = svg.querySelector<SVGGElement>('.look')!;
  const pupil = svg.querySelector<SVGCircleElement>('.pup')!;
  const up = svg.querySelector<SVGGElement>('.lid.up')!;
  const down = svg.querySelector<SVGGElement>('.lid.down')!;
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let tx = 0, ty = 0, moved = -1e9, x = 0, y = 0, size = 1, shut = 0, raf = 0;
  let blinkAt = performance.now() + 1800, blinkStart = -1, twice = false;
  let drawn = '';
  const onMove = (e: PointerEvent) => { tx = e.clientX; ty = e.clientY; moved = performance.now(); };
  addEventListener('pointermove', onMove);

  const tick = (t: number) => {
    const r = svg.getBoundingClientRect();
    const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    const resting = t - moved > IDLE_MS;
    let gx = tx, gy = ty;
    if (resting) {
      const target = idleTarget();
      if (target) { const b = target.getBoundingClientRect(); gx = b.left + b.width / 2; gy = b.top + 80; }
      else if (reduced) { gx = cx; gy = cy; }
      else { gx = cx + Math.sin(t / 2300) * 220; gy = cy + Math.sin(t / 3100) * 30; }
    }
    const aim = aimAt(gx - cx, gy - cy);
    // the pupil narrows when the pointer comes close, as in bright light
    const near = !resting && Math.hypot(tx - cx, ty - cy) < 90;
    // eased: a fifth of the way each frame
    const k = reduced ? 1 : 0.2;
    x += (aim.x - x) * k;
    y += (aim.y - y) * k;
    size += ((near ? 0.68 : 1) - size) * k;

    if (!reduced) {
      if (blinkStart < 0 && t >= blinkAt) blinkStart = t;
      if (blinkStart >= 0) {
        const p = (t - blinkStart) / BLINK_MS;
        if (p >= 1) {
          blinkStart = -1;
          shut = 0;
          // now and then a second blink right after the first
          if (twice) { twice = false; blinkAt = t + 2400 + Math.random() * 4200; } else if (Math.random() < 0.2) { twice = true; blinkAt = t + 120; } else blinkAt = t + 2400 + Math.random() * 4200;
        } else shut = Math.sin(p * Math.PI);
      }
    }

    const key = `${x.toFixed(2)} ${y.toFixed(2)} ${size.toFixed(3)} ${shut.toFixed(3)}`;
    if (key !== drawn) {
      drawn = key;
      look.setAttribute('transform', `translate(${x.toFixed(2)} ${y.toFixed(2)})`);
      pupil.setAttribute('r', (4.2 * size).toFixed(2));
      // the upper lid does most of the work and slips under the lower one: the closed eye is one curved line
      up.setAttribute('transform', `translate(0 ${(shut * 17).toFixed(2)})`);
      down.setAttribute('transform', `translate(0 ${(-shut * 8.5).toFixed(2)})`);
    }
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
  return () => { cancelAnimationFrame(raf); removeEventListener('pointermove', onMove); };
}
