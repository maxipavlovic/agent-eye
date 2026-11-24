import { useEffect, useId, useRef } from 'preact/hooks';

/** The almond the eye is drawn in: two hairline arcs. */
const ALMOND = 'M1.5 17C14 2.5 46 2.5 58.5 17C46 31.5 14 31.5 1.5 17Z';

/** Fibres of the iris: sparse radial hairlines, long and short in turn. */
const FIBRES = (() => {
  const R = 11.5, p = 4.2, n = 36;
  let d = '';
  for (let i = 0; i < n; i++) {
    const a = (i / n) * Math.PI * 2, r0 = p + 1.2, r1 = i % 2 ? R - 2.4 : R - 1;
    d += `M${(Math.cos(a) * r0).toFixed(2)} ${(Math.sin(a) * r0).toFixed(2)}L${(Math.cos(a) * r1).toFixed(2)} ${(Math.sin(a) * r1).toFixed(2)}`;
  }
  return d;
})();

/** The almond's two arcs, drawn from the inner corner, and how far outside them the lids rest. */
const UPPER = 'C14 2.5 46 2.5 58.5 17', LOWER = 'C14 31.5 46 31.5 58.5 17', REST = 1.5;
/** A lid: the paper beyond one arc, out to well past the almond. */
const LID_UP = 'M1.5 17C14 2.5 46 2.5 58.5 17H62V-24H-2V17Z', LID_DOWN = 'M1.5 17C14 31.5 46 31.5 58.5 17H62V58H-2V17Z';

/** How far the iris may move inside the almond, in drawing units. */
const REACH_X = 9, REACH_Y = 3.2;

/**
 * The brand eye in hairlines: the iris follows the pointer; when something needs you (`alert`) and the
 * pointer has not moved yet, it looks at the Needs you lane. Now and then it blinks: two lids, paper with a
 * hairline edge shaped like the almond, close into one curved line (iris.css).
 */
export function Eye({ alert }: { alert: boolean }) {
  const svg = useRef<SVGSVGElement>(null);
  const look = useRef<SVGGElement>(null);
  const moved = useRef(false);
  // one clip per eye: the welcome page shows two
  const clip = `almond-${useId()}`;

  const aim = (x: number, y: number) => {
    const box = svg.current?.getBoundingClientRect();
    if (!box || !look.current) return;
    const dx = x - (box.left + box.width / 2), dy = y - (box.top + box.height / 2);
    const d = Math.hypot(dx, dy) || 1, k = Math.min(1, d / 420);
    look.current.style.transform = `translate(${((dx / d) * REACH_X * k).toFixed(2)}px, ${((dy / d) * REACH_Y * k).toFixed(2)}px)`;
  };

  useEffect(() => {
    const on = (e: PointerEvent) => { moved.current = true; aim(e.clientX, e.clientY); };
    addEventListener('pointermove', on);
    return () => removeEventListener('pointermove', on);
  }, []);

  useEffect(() => {
    if (!alert || moved.current) return;
    const lane = document.querySelector('[data-col="needs_you"]')?.getBoundingClientRect();
    if (lane) aim(lane.left + lane.width / 2, lane.top + 120);
  }, [alert]);

  return (
    <svg ref={svg} class={`eye${alert ? ' alert' : ''}`} viewBox="0 0 60 34" width="56" height="32" aria-hidden="true">
      <defs><clipPath id={clip}><path d={ALMOND} /></clipPath></defs>
      <g clip-path={`url(#${clip})`}>
        <g ref={look} class="look">
          <g transform="translate(30 17)">
            <path class="fib" d={FIBRES} />
            <circle class="ir" r="11.5" />
            <circle class="pup" r="4.2" />
            <circle class="gl" cx="-1.7" cy="-1.9" r="1.05" />
          </g>
        </g>
        <g class="lid up"><path class="lidfill" d={LID_UP} transform={`translate(0 ${-REST})`} /><path class="s" d={`M1.5 17${UPPER}`} transform={`translate(0 ${-REST})`} /></g>
        <g class="lid down"><path class="lidfill" d={LID_DOWN} transform={`translate(0 ${REST})`} /><path class="s" d={`M1.5 17${LOWER}`} transform={`translate(0 ${REST})`} /></g>
      </g>
      <path class="s" d={ALMOND} />
    </svg>
  );
}
