/** Canvas drawing for the race (style A · Clean): hairline value gridlines with
 * their labels under the plot, bars with a 4 px rounded tip and a square baseline,
 * segments split by a surface seam, hatched parts and the corner date. */
import { font } from '../draw.ts';
import type { ChartTokens } from '../tokens';
import type { RaceLayout, RaceRow } from './race';
import { barExtent, barParts, drawnWidths, raceTicks, rowCentre, rowExtent, valueX } from './race.ts';

const crisp = (value: number) => Math.round(value) + 0.5;
const INSET_SHARE = .5;
const OUTER_ALPHA = .35;
/** The rounded tip, as style A draws it. */
export const TIP_RADIUS = 4;
/** Room between a bar's tip and its value label. */
export const VALUE_GAP = 8;
/** A flag in the label column: 21 × 14 px, 8 px from the edge (mock-up style-1). */
export const FLAG = { x: 8, width: 21, height: 14 } as const;
/** Where the name starts, after the flag. */
export const NAME_X = FLAG.x + FLAG.width + 7;

/** Hairline gridlines at the value ticks, labelled under the plot. */
export function drawRaceGrid(ctx: CanvasRenderingContext2D, layout: RaceLayout, tokens: ChartTokens, min: number, max: number,
  format: (value: number, ticks: readonly number[]) => string): void {
  const ticks = raceTicks(layout, min, max);
  ctx.save();
  ctx.lineWidth = tokens.lineWidth.grid;
  ctx.strokeStyle = tokens.grid;
  ctx.font = font(tokens, tokens.fontSize.small);
  ctx.fillStyle = tokens.muted;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  for (const tick of ticks) {
    const x = crisp(valueX(layout, min, max, tick));
    ctx.beginPath(); ctx.moveTo(x, layout.top); ctx.lineTo(x, layout.bottom); ctx.stroke();
    ctx.fillText(format(tick, ticks), x, layout.bottom + 6);
  }
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

/** Diagonal hatching over a rectangle in `color`, on a faint wash of it: a part of
 * the bar that is counted apart, such as damaged factories. */
export function fillHatched(ctx: CanvasRenderingContext2D, x: number, y: number, width: number, height: number, color: string, spacing = 4): void {
  if (width <= 0 || height <= 0) return;
  ctx.save();
  ctx.beginPath(); ctx.rect(x, y, width, height); ctx.clip();
  ctx.globalAlpha *= 0.25;
  ctx.fillStyle = color;
  ctx.fillRect(x, y, width, height);
  ctx.globalAlpha /= 0.25;
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  for (let start = x - height; start < x + width; start += spacing) {
    ctx.moveTo(start, y + height);
    ctx.lineTo(start + height, y);
  }
  ctx.stroke();
  ctx.restore();
}

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
      if (part.hatched) fillHatched(ctx, from, top, width, layout.thickness, color);
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
  const size = Math.max(tokens.fontSize.title, Math.min(64, (layout.right - layout.left) / 6, (layout.bottom - layout.top) / 3));
  ctx.save();
  ctx.font = font(tokens, size, 800);
  ctx.fillStyle = tokens.text;
  ctx.globalAlpha = 0.08;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'alphabetic';
  ctx.fillText(text, layout.right, layout.bottom - 8, layout.right - layout.left);
  ctx.restore();
}
