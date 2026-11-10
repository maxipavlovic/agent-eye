import type { SVGAttributes } from 'preact';
import { rects, size, type Palette, type Rows } from './sprite';

interface PixProps extends Omit<SVGAttributes<SVGSVGElement>, 'scale'> {
  rows: Rows;
  pal: Palette;
  /** CSS px per art pixel. */
  scale?: number;
  label?: string;
}

/** Inline SVG sprite; palette colours may be CSS variables, so it follows the theme. */
export function Pix({ rows, pal, scale = 2, label, ...rest }: PixProps) {
  const { w, h } = size(rows);
  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      width={w * scale}
      height={h * scale}
      shape-rendering="crispEdges"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      dangerouslySetInnerHTML={{ __html: rects(rows, pal) }}
      {...rest}
    />
  );
}
