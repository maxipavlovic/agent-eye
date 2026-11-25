import { watchGaze } from '../../board/gaze';

/**
 * Moves the pupil of an `.eye` element toward the pointer in whole pixels and blinks it now and then, in four
 * frames (watchGaze says where and when). While the pointer is idle the eye looks at `idleTarget()` when that
 * gives an element, else it drifts. Returns the function that stops it.
 */
export function trackEye(el: HTMLElement, idleTarget: () => Element | null = () => null): () => void {
  const pupil = el.querySelector<HTMLElement>('.pupil')!;
  let px = 0, py = 0, frame = 0;
  return watchGaze(el, (g) => {
    const nx = Math.round(g.x * 3), ny = Math.round(g.y);
    if (nx !== px || ny !== py) { px = nx; py = ny; pupil.style.transform = `translate(${px * 4}px,${py * 4}px)`; }
    const f = g.blink < 0 ? 0 : [1, 2, 2, 1][Math.floor(g.blink * 4)]!;
    if (f !== frame) { frame = f; el.dataset.f = String(f); }
  }, { ms: 320, gap: 2200 }, idleTarget);
}
