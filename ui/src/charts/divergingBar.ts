/** Bars that grow left (negative) or right (positive) from a zero line, all on one
 * scale (mock-up economy-1's surplus and shortage bars). Pure: fractions of the
 * track, for a widget to place with `left` and `width`. */

/** Where zero sits and where a bar starts and how wide it is, as fractions of the
 * track. `negative` and `positive` are the largest magnitudes each side shows (the
 * scale); zero sits after the negative side's share. A value beyond its side's
 * extent is cut at the track's edge; without any extent nothing is drawn. */
export function divergingBar(value: number, negative: number, positive: number): { zero: number; start: number; width: number } {
  const [low, high] = [Math.max(0, negative), Math.max(0, positive)];
  const total = low + high;
  if (!(total > 0) || !Number.isFinite(value)) return { zero: total > 0 ? low / total : 0, start: total > 0 ? low / total : 0, width: 0 };
  const zero = low / total;
  const width = Math.min(Math.abs(value) / total, value < 0 ? zero : 1 - zero);
  return { zero, start: value < 0 ? zero - width : zero, width };
}

/** Where a bar's end label goes: just past the bar's outer end, on its side of zero. */
export function barEndLabel(bar: { start: number; width: number }, value: number): { at: number; side: 'left' | 'right' } {
  return value < 0 ? { at: bar.start, side: 'left' } : { at: bar.start + bar.width, side: 'right' };
}
