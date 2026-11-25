import { describe, expect, it } from 'vitest';
import { placeMenu } from './CardMenu';

describe('where a card menu opens', () => {
  const W = 300, H = 600, VW = 1440, VH = 900;

  it('right of and below the pointer when it fits', () => {
    expect(placeMenu(100, 200, W, H, VW, VH)).toEqual({ left: 100, top: 200 });
  });

  it('moves up only as far as it must when the room below is short, staying by the card', () => {
    // 600 px tall at y 400: it ends 8 px above the bottom edge, not at the top of the window
    expect(placeMenu(100, 400, W, H, VW, VH).top).toBe(VH - H - 8);
    expect(placeMenu(100, 880, W, H, VW, VH).top).toBe(VH - H - 8);
  });

  it('starts at the top when it is taller than the window', () => {
    expect(placeMenu(100, 400, W, 1000, VW, VH).top).toBe(8);
  });

  it('opens to the left near the right edge, and never past the left edge', () => {
    expect(placeMenu(1300, 100, W, H, VW, VH).left).toBe(1000);
    // a narrow window with room on neither side: against the right edge, or the left one when wider still
    expect(placeMenu(250, 100, W, H, 400, VH).left).toBe(92);
    expect(placeMenu(100, 100, 500, H, 400, VH).left).toBe(8);
  });
});
