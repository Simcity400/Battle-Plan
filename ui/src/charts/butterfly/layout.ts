/** Geometry for paired bars either side of one zero axis, laid out as the race lays
 * out its rows: flag and name on the left, the left value growing left and the right
 * value growing right of the axis with exact values at their tips, then the ratio. */
import { FLAG, MIN_LABEL, NAME_PAD, NAME_X, VALUE_GAP, VALUE_PAD } from '../race/draw.ts';
import { barThickness, MIN_SLOT, RACE_AXIS } from '../race/race.ts';
import { linearTicks } from '../valueAxis.ts';

export interface ButterflyValue { id: string; left: number | null; right: number | null }
export interface ButterflyLayout {
  /** The name column's width, the shared zero axis and how far a bar reaches either way. */
  labelWidth: number; axis: number; half: number;
  /** The ratio column's right edge. */
  ratioX: number;
  /** The column headings above the rows, one row's height and a bar's thickness in it. */
  top: number; slot: number; barHeight: number;
}

/** The column headings' row above the bars, as tall as the value axis under them. */
export const BUTTERFLY_HEADER = RACE_AXIS;
/** The name column's largest share: both halves and the ratio need the rest. */
const MAX_LABEL_SHARE = 0.32;
/** Room either side of the ratio column. */
const RATIO_GAP = VALUE_GAP * 2;
/** The least length a bar keeps before the chart scrolls sideways. */
const MIN_BAR = 24;
/** Value-axis ticks come about this far apart on each side, and never nearer than
 * their labels need. */
const TICK_SPACING = 90;
const MIN_TICK_GAP = 56;

const valueRoom = (valueWidth: number) => (valueWidth ? valueWidth + VALUE_GAP : 0) + VALUE_PAD;
const ratioRoom = (ratioWidth: number) => ratioWidth ? ratioWidth + RATIO_GAP : 0;

/** Rows share the height between the headings and the value axis, sized for `slots`
 * (the top N) as the race sizes them and never less than its least row. Both halves
 * keep the same room, with exact end labels and a separate ratio column. */
export function butterflyLayout(width: number, height: number, slots: number,
  labelWidth: number, valueWidth: number, ratioWidth: number): ButterflyLayout {
  const ratioX = width - VALUE_PAD;
  const plotRight = ratioX - ratioRoom(ratioWidth);
  const axis = (labelWidth + plotRight) / 2;
  const half = Math.max(0, axis - labelWidth - valueRoom(valueWidth));
  const top = BUTTERFLY_HEADER;
  const slot = Math.max(MIN_SLOT, (height - top - RACE_AXIS) / Math.max(1, Math.floor(slots)));
  return { labelWidth, axis, half, ratioX, top, slot, barHeight: barThickness(slot) };
}

/** The width under which the chart scrolls sideways rather than squeeze its bars. */
export function butterflyMinimumWidth(labelWidth: number, valueWidth: number, ratioWidth: number): number {
  return labelWidth + 2 * (valueRoom(valueWidth) + MIN_BAR) + ratioRoom(ratioWidth) + VALUE_PAD;
}

/** One fitted symmetric scale. Signed corrections use magnitude for geometry. */
export function butterflyScale(items: readonly ButterflyValue[]): number {
  let maximum = 0;
  for (const item of items) for (const value of [item.left, item.right]) if (value != null && Number.isFinite(value)) maximum = Math.max(maximum, Math.abs(value));
  return maximum > 0 ? maximum * 1.05 : 1;
}

/** One side's value-axis ticks from the axis out, zero first, on a scale reaching
 * `half` px; a tick nearer the one before than its label needs is left out. */
export function butterflyTicks(scale: number, half: number): number[] {
  const kept = [0];
  if (!(scale > 0) || !(half > 0)) return kept;
  for (const tick of linearTicks(0, scale, Math.max(2, Math.round(half / TICK_SPACING)))) {
    if (tick > 0 && tick <= scale && (tick - kept.at(-1)!) / scale * half >= MIN_TICK_GAP) kept.push(tick);
  }
  return kept;
}

/** Where a name starts: after its flag, as in the race, or at the flag's place without one. */
export const butterflyNameX = (showFlags = true) => showFlags ? NAME_X : FLAG.x;

/** The race's name column: the longest caller name after the flag, at most a third of the chart. */
export function butterflyLabelWidth(width: number, longestName: number, showFlags = true): number {
  return Math.min(Math.max(MIN_LABEL, butterflyNameX(showFlags) + longestName + NAME_PAD), Math.max(MIN_LABEL, width * MAX_LABEL_SHARE));
}

/** Truncate glyphs to fit, never squeeze them with Canvas fillText's maxWidth. */
export function ellipsizeLabel(label: string, width: number, measure: (text: string) => number): string {
  if (measure(label) <= width) return label;
  if (measure('…') > width) return '';
  const glyphs = Array.from(label);
  let lo = 0, hi = glyphs.length;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(glyphs.slice(0, mid).join('') + '…') <= width) lo = mid; else hi = mid - 1;
  }
  return glyphs.slice(0, lo).join('') + '…';
}

/** The right/left ratio, rounded to two decimals; a zero or missing denominator has none. */
export function formatRatio(right: number | bigint | null, left: number | bigint | null): string {
  if (right == null || left == null || left === 0 || left === 0n) return '—';
  if (typeof right === 'bigint' && typeof left === 'bigint') {
    const numerator = right < 0n ? -right : right, denominator = left < 0n ? -left : left;
    const hundredths = (numerator * 200n + denominator) / (denominator * 2n);
    const sign = (right < 0n) !== (left < 0n) && hundredths !== 0n ? '-' : '';
    return `${sign}${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')} : 1`;
  }
  const ratio = Number(right) / Number(left);
  return Number.isFinite(ratio) ? `${ratio.toFixed(2)} : 1` : '—';
}

/** The list fills the body until its rows need scrolling at the race's least row height. */
export function butterflyMinimumHeight(count: number): number {
  return BUTTERFLY_HEADER + Math.max(1, Math.floor(count)) * MIN_SLOT + RACE_AXIS;
}

/** The complete list scrolls, while the backing canvas stays viewport-sized. */
export function butterflySurface(count: number, viewportHeight: number) {
  return { canvasHeight: viewportHeight, contentHeight: Math.max(viewportHeight, butterflyMinimumHeight(count)) };
}

/** Visible list coordinates between the fixed column headings and the fixed value axis. */
export function butterflyRowViewport(top: number, viewportHeight: number, scrollTop: number) {
  const height = Math.max(0, viewportHeight - top - RACE_AXIS);
  return { start: scrollTop + top, end: scrollTop + top + height, height };
}

/** Measure caller text without spreading an unbounded item list into arguments. A
 * missing value's text is drawn even with values off, so it keeps its room. */
export function butterflyTextWidths(items: Iterable<{ left?: number | null; right?: number | null; leftText: string; rightText: string; ratioText: string }>,
  measure: (text: string) => number, ratioLabel: string, showValues: boolean, showRatio: boolean) {
  let valueWidth = 0, ratioWidth = showRatio ? measure(ratioLabel) : 0;
  for (const item of items) {
    if (showValues || item.left === null) valueWidth = Math.max(valueWidth, measure(item.leftText));
    if (showValues || item.right === null) valueWidth = Math.max(valueWidth, measure(item.rightText));
    if (showRatio) ratioWidth = Math.max(ratioWidth, measure(item.ratioText));
  }
  return { valueWidth, ratioWidth };
}
