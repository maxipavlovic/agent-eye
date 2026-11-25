import { aim, watchGaze } from '../../board/gaze';

/** How far the iris may move inside the almond, in drawing units. */
export const REACH_X = 9, REACH_Y = 3.2;
/** The pupil's radius at rest, in drawing units; it narrows to 0.68 of it when the pointer comes close. */
export const PUPIL_R = 4.2;

/** Where the iris sits, in drawing units, for a gaze point `dx`, `dy` CSS px from the eye's centre. */
export function aimAt(dx: number, dy: number): { x: number; y: number } {
  const a = aim(dx, dy);
  return { x: a.x * REACH_X, y: a.y * REACH_Y };
}

/**
 * Brings an iris `svg.eye` to life (watchGaze says where it looks and when it blinks): the iris eases toward
 * the gaze point a fifth of the way each frame, its pupil narrows when the pointer comes close, and a blink
 * closes two paper lids into one curved line. With reduced motion it jumps to where it looks. Returns the stop.
 */
export function followEye(svg: SVGSVGElement, idleTarget: () => Element | null = () => null): () => void {
  const look = svg.querySelector<SVGGElement>('.look')!;
  const pupil = svg.querySelector<SVGCircleElement>('.pup')!;
  const up = svg.querySelector<SVGGElement>('.lid.up')!;
  const down = svg.querySelector<SVGGElement>('.lid.down')!;
  let x = 0, y = 0, size = 1, drawn = '';
  return watchGaze(svg, (g) => {
    const k = g.reduced ? 1 : 0.2;
    x += (g.x * REACH_X - x) * k;
    y += (g.y * REACH_Y - y) * k;
    size += ((g.near ? 0.68 : 1) - size) * k;
    const shut = g.blink < 0 ? 0 : Math.sin(g.blink * Math.PI);
    const key = `${x.toFixed(2)} ${y.toFixed(2)} ${size.toFixed(3)} ${shut.toFixed(3)}`;
    if (key === drawn) return;
    drawn = key;
    look.setAttribute('transform', `translate(${x.toFixed(2)} ${y.toFixed(2)})`);
    pupil.setAttribute('r', (PUPIL_R * size).toFixed(2));
    // the upper lid does most of the work and slips under the lower one: the closed eye is one curved line
    up.setAttribute('transform', `translate(0 ${(shut * 17).toFixed(2)})`);
    down.setAttribute('transform', `translate(0 ${(-shut * 8.5).toFixed(2)})`);
  }, { ms: 230, gap: 2400, twice: 0.2 }, idleTarget);
}
