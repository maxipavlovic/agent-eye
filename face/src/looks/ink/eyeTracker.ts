/**
 * Moves the pupil of an `.eye` element toward the pointer and blinks it now and then. While the
 * pointer is idle the eye looks at `idleTarget()` when that gives an element, else it drifts.
 * Returns the function that stops it.
 */
export function trackEye(el: HTMLElement, idleTarget: () => Element | null = () => null): () => void {
  const pupil = el.querySelector<HTMLElement>('.pupil')!;
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  let tx = 0, ty = 0, moved = -1e9, px = 0, py = 0, frame = 0, blinkAt = performance.now() + 1800, blinkStart = -1, raf = 0;
  const onMove = (e: PointerEvent) => { tx = e.clientX; ty = e.clientY; moved = performance.now(); };
  addEventListener('pointermove', onMove);
  const clamp = (v: number) => Math.max(-1, Math.min(1, v));
  const tick = (t: number) => {
    const r = el.getBoundingClientRect();
    const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    let gx = tx, gy = ty;
    if (t - moved > 2500) {
      const target = idleTarget();
      if (target) { const b = target.getBoundingClientRect(); gx = b.left + b.width / 2; gy = b.top + 80; }
      else if (reduced) { gx = cx; gy = cy; }
      else { gx = cx + Math.sin(t / 2300) * 220; gy = cy + Math.sin(t / 3100) * 30; }
    }
    const nx = Math.round(clamp((gx - cx) / 160) * 3);
    const ny = Math.round(clamp((gy - cy) / 70));
    if (nx !== px || ny !== py) { px = nx; py = ny; pupil.style.transform = `translate(${px * 4}px,${py * 4}px)`; }
    let f = 0;
    if (!reduced) {
      if (blinkStart < 0 && t >= blinkAt) blinkStart = t;
      if (blinkStart >= 0) {
        const i = Math.floor((t - blinkStart) / 80);
        if (i >= 4) { blinkStart = -1; blinkAt = t + 2200 + Math.random() * 4200; } else f = [1, 2, 2, 1][i]!;
      }
    }
    if (f !== frame) { frame = f; el.dataset.f = String(f); }
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
  return () => { cancelAnimationFrame(raf); removeEventListener('pointermove', onMove); };
}
