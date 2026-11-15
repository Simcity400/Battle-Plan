import { fillHatched } from '../hatch.ts';
/** Canvas drawing for the race: hairline value gridlines with
 * their labels under the plot, bars with a 4 px rounded tip and a square baseline,
 * segments split by a surface seam, hatched parts and the corner date. */
import { font } from '../draw.ts';
import type { ChartTokens } from '../tokens';
import type { RaceLayout, RaceRow } from './race';
import { barExtent, barParts, drawnWidths, raceTicks, rowCentre, rowExtent, valueX } from './race.ts';
import { textWidth } from '../textWidth.ts';

const crisp = (value: number) => Math.round(value) + 0.5;
const INSET_SHARE = .5;
const OUTER_ALPHA = .35;
/** The rounded tip, as style A draws it. */
export const TIP_RADIUS = 4;
/** Room between a bar's tip and its value label. */
export const VALUE_GAP = 8;
/** A flag in the label column: 21 × 14 px, 8 px from the edge. */
export const FLAG = { x: 8, width: 21, height: 14 } as const;
/** Where the name starts, after the flag. */
export const NAME_X = FLAG.x + FLAG.width + 7;
/** Room after the widest name, and the name column's least width. */
export const NAME_PAD = 12;
export const MIN_LABEL = 72;
/** Room kept right of the widest value label. */
export const VALUE_PAD = 6;

const tickLabels = new WeakMap<(value: number, ticks: readonly number[]) => string, { key: string; text: string[] }>();

/** Hairline gridlines at the value ticks, labelled under the plot. */
export function drawRaceGrid(ctx: CanvasRenderingContext2D, layout: RaceLayout, tokens: ChartTokens, min: number, max: number,
  format: (value: number, ticks: readonly number[]) => string): void {
  const ticks = raceTicks(layout, min, max);
  // The same ticks keep their labels: an easing axis rarely moves them.
  const key = ticks.join(',');
  let held = tickLabels.get(format);
  if (held?.key !== key) tickLabels.set(format, held = { key, text: ticks.map(tick => format(tick, ticks)) });
  const labels = held.text;
  ctx.save();
  ctx.lineWidth = tokens.lineWidth.grid;
  ctx.strokeStyle = tokens.grid;
  ctx.font = font(tokens, tokens.fontSize.small);
  ctx.fillStyle = tokens.muted;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  ticks.forEach((tick, at) => {
    const x = crisp(valueX(layout, min, max, tick));
    ctx.beginPath(); ctx.moveTo(x, layout.view.top); ctx.lineTo(x, layout.view.bottom); ctx.stroke();
    ctx.fillText(labels[at]!, x, layout.view.bottom + 6);
  });
  ctx.restore();
}

/** A bar's outline from its baseline at `x`: square there, a rounded tip at the
 * other end, to the right (`width` > 0) or to the left (`width` < 0). */
export function barPath(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, radius = TIP_RADIUS): void {
  const size = Math.abs(width);
  const r = Math.max(0, Math.min(radius, size / 2, height / 2));
  const tip = x + width;
  const back = Math.sign(width) || 1;
  ctx.beginPath();
  ctx.moveTo(x, y);
  ctx.lineTo(tip - back * r, y);
  ctx.arcTo(tip, y, tip, y + r, r);
  ctx.lineTo(tip, y + height - r);
  ctx.arcTo(tip, y + height, tip - back * r, y + height, r);
  ctx.lineTo(x, y + height);
  ctx.closePath();
}

export { fillHatched } from '../hatch.ts';

/** One row's bar: its segments stacked as signed values (`barParts`), positive
 * ones right of the baseline and negative ones left of it, each solid then hatched
 * away from zero; each side is clipped to one outline with a rounded tip, and a
 * surface seam divides the segments. Returns the right end of the bar (the baseline
 * when nothing lies right of it), where the value label goes. */
export function drawRaceBar(ctx: CanvasRenderingContext2D, layout: RaceLayout, tokens: ChartTokens, row: RaceRow, min: number, max: number): number {
  const centre = rowCentre(layout, row.y);
  const top = centre - layout.thickness / 2;
  const x = (value: number) => valueX(layout, min, max, value);
  const base = x(0);
  const parts = barParts(drawnWidths(row));
  const extent = barExtent(drawnWidths(row));
  // One solid part from the baseline: its outline filled, the same pixels as the clipped path.
  if (parts.length === 1 && !parts[0]!.hatched && !row.item.inset && (parts[0]!.low === 0 || parts[0]!.high === 0)) {
    const end = x(parts[0]!.low === 0 ? parts[0]!.high : parts[0]!.low);
    if (Math.abs(end - base) >= 0.01) {
      ctx.fillStyle = row.item.segments[parts[0]!.segment].color;
      barPath(ctx, base, top, end - base, layout.thickness);
      ctx.fill();
    }
    return x(rowExtent(row).high);
  }
  for (const side of [1, -1]) {
    const end = x(side > 0 ? extent.high : extent.low);
    if (Math.abs(end - base) < 0.01) continue;
    ctx.save();
    if (row.item.inset) ctx.globalAlpha *= OUTER_ALPHA;
    barPath(ctx, base, top, end - base, layout.thickness);
    ctx.clip();
    for (const part of parts.filter(item => (side > 0 ? item.high > 0 : item.low < 0))) {
      const from = x(part.low);
      const width = x(part.high) - from;
      const color = row.item.segments[part.segment].color;
      if (part.hatched) fillHatched(ctx, from, top, width, layout.thickness, color, 4, 1.5);
      else { ctx.fillStyle = color; ctx.fillRect(from, top, width, layout.thickness); }
      // A seam where one segment meets another, never at the baseline.
      const seam = side > 0 ? from : from + width;
      if (Math.abs(seam - base) > 0.5 && !(part.hatched && parts.some(other => other.segment === part.segment && !other.hatched))) {
        ctx.fillStyle = tokens.surface;
        ctx.fillRect(seam - 0.5, top, 1, layout.thickness);
      }
    }
    ctx.restore();
  }
  if (row.item.inset && row.inset !== undefined && row.inset !== 0) {
    const height = layout.thickness * INSET_SHARE;
    ctx.save();
    barPath(ctx, base, centre - height / 2, x(row.inset) - base, height);
    ctx.fillStyle = row.item.inset.color;
    ctx.fill();
    ctx.strokeStyle = tokens.surface;
    ctx.lineWidth = tokens.lineWidth.grid;
    ctx.stroke();
    ctx.restore();
  }
  return x(rowExtent(row).high);
}

/** The moment shown as a large, faint date in the plot's lower right corner, under
 * the bars (the race widgets' date overlay, off by default). */
export function drawDateOverlay(ctx: CanvasRenderingContext2D, layout: RaceLayout, tokens: ChartTokens, text: string): void {
  const size = Math.max(tokens.fontSize.title, Math.min(64, (layout.right - layout.left) / 6, (layout.view.bottom - layout.view.top) / 3));
  ctx.save();
  ctx.font = font(tokens, size, 800);
  ctx.fillStyle = tokens.text;
  ctx.globalAlpha = 0.08;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'alphabetic';
  ctx.fillText(text, layout.right, layout.view.bottom - 8, layout.right - layout.left);
  ctx.restore();
}

/** `text` broken at spaces into lines no wider than `width` by `measure`; a word wider than `width` is a line of its own. */
export function wrapText(text: string, width: number, measure: (text: string) => number): string[] {
  const lines: string[] = [];
  let line = '';
  for (const word of text.split(/\s+/)) {
    if (!word) continue;
    const next = line ? `${line} ${word}` : word;
    if (line && measure(next) > width) { lines.push(line); line = word; } else line = next;
  }
  if (line) lines.push(line);
  return lines;
}

/** Why a race has no bar, centred in its view and wrapped to its width, never squeezed onto one line. */
export function drawRaceEmpty(ctx: CanvasRenderingContext2D, layout: RaceLayout, tokens: ChartTokens, text: string): void {
  const width = layout.right - layout.left, step = Math.round(tokens.fontSize.body * 1.4);
  ctx.save();
  ctx.font = font(tokens, tokens.fontSize.body);
  ctx.fillStyle = tokens.muted;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  const lines = wrapText(text, width, line => textWidth(ctx, line));
  const top = (layout.view.top + layout.view.bottom) / 2 - (lines.length - 1) * step / 2;
  lines.forEach((line, at) => ctx.fillText(line, (layout.left + layout.right) / 2, top + at * step, width));
  ctx.restore();
}
