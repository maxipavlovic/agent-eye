import { useEffect, useId, useRef } from 'preact/hooks';
import { needsLane } from '../../board/gaze';
import { followEye, PUPIL_R } from './eyeFollow';

/** The almond the eye is drawn in: two hairline arcs. */
const ALMOND = 'M1.5 17C14 2.5 46 2.5 58.5 17C46 31.5 14 31.5 1.5 17Z';

/** The almond's two arcs, drawn from the inner corner, and how far outside them the lids rest. */
const UPPER = 'C14 2.5 46 2.5 58.5 17', LOWER = 'C14 31.5 46 31.5 58.5 17', REST = 1.5;
/** A lid: the paper beyond one arc, out to well past the almond. */
const LID_UP = `M1.5 17${UPPER}H62V-24H-2V17Z`, LID_DOWN = `M1.5 17${LOWER}H62V58H-2V17Z`;
/** Three lashes over the upper arc, only while something needs you (as ink's eye). */
const LASHES = 'M30 4.4V1.2M21.6 6.4L19.9 3.3M38.4 6.4L40.1 3.3';

/** Fibres of the iris: sparse radial hairlines, long and short in turn. */
const FIBRES = (() => {
  const R = 11.5, p = PUPIL_R, n = 36;
  let d = '';
  for (let i = 0; i < n; i++) {
    const a = (i / n) * Math.PI * 2, r0 = p + 1.2, r1 = i % 2 ? R - 2.4 : R - 1;
    d += `M${(Math.cos(a) * r0).toFixed(2)} ${(Math.sin(a) * r0).toFixed(2)}L${(Math.cos(a) * r1).toFixed(2)} ${(Math.sin(a) * r1).toFixed(2)}`;
  }
  return d;
})();

/**
 * The brand eye in hairlines (eyeFollow.ts moves it): the iris follows the pointer and its pupil narrows when
 * the pointer comes close; resting, it looks at Needs you while something waits there (`alert`, which also
 * raises three lashes), else it drifts; now and then it blinks, two paper lids closing into one curved line.
 */
export function Eye({ alert }: { alert: boolean }) {
  const svg = useRef<SVGSVGElement>(null);
  const alertRef = useRef(alert);
  alertRef.current = alert;
  // one clip per eye: the welcome page shows two
  const clip = `almond-${useId()}`;

  useEffect(() => followEye(svg.current!, () => (alertRef.current ? needsLane() : null)), []);

  return (
    <svg ref={svg} class={`eye${alert ? ' alert' : ''}`} viewBox="0 0 60 34" width="56" height="32" aria-hidden="true">
      <defs><clipPath id={clip}><path d={ALMOND} /></clipPath></defs>
      <path class="lash" d={LASHES} />
      <g clip-path={`url(#${clip})`}>
        <g class="look">
          <g transform="translate(30 17)">
            <path class="fib" d={FIBRES} />
            <circle class="ir" r="11.5" />
            <circle class="pup" r={PUPIL_R} />
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
