import { useEffect, useRef } from 'preact/hooks';
import { Pix } from '../../pixel/Pix';
import { EYE_FRAMES, INK, IRIS, IRIS_ALERT, RAYS, only } from './art';
import { trackEye } from './eyeTracker';

/** The brand eye: follows the pointer, blinks now and then, and looks at Needs you when something waits (`alert`). */
export function Eye({ alert }: { alert: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const alertRef = useRef(alert);
  alertRef.current = alert;
  useEffect(() => trackEye(ref.current!, () => (alertRef.current ? document.querySelector('[data-col="needs_you"]') : null)), []);
  return (
    <div class={`eye${alert ? ' alert' : ''}`} ref={ref} data-f="0" title="eye">
      <div class="rays"><Pix rows={RAYS} pal={INK} scale={4} /></div>
      {EYE_FRAMES.map((f, i) => <div class={`lay f${i}`} key={i}><Pix rows={only(f, 'o')} pal={INK} scale={4} /></div>)}
      <div class="pclip"><div class="pupil"><Pix rows={alert ? IRIS_ALERT : IRIS} pal={INK} scale={4} /></div></div>
      {EYE_FRAMES.map((f, i) => <div class={`lay f${i}`} key={i}><Pix rows={only(f, '#')} pal={INK} scale={4} /></div>)}
    </div>
  );
}
