import { LABEL_WEIGHT } from '../../charts/draw.ts';
import { ellipsiseCanvasText, measureCanvasText, type Canvas2D } from '../../charts/textWidth.ts';
import { drawBar, drawRowFocusRing, drawRowHover, drawRowSelected, resolveKitTokens } from '../../draw/kit.ts';
import { auditCell, type FamilyAuditRow } from '../auditRow.ts';
import { drawMediaSlot } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import type { FamilyTheme } from '../contract.ts';
import type { FamilyTooltipPayload } from '../host/tooltips.ts';
import { BARS_PAD_TOP, PICK_DIVIDER_GAP, PICK_ROW_HEIGHT, type BarsLayout } from './layout.ts';
import type { BarsCaptionSide, BarsPickList, BarsPickRow } from './model.ts';

export const PICK_PREFIX = 'pick:';
const DUEL_WIDTH = 110;
const DUEL_HEIGHT = 6;
const LABEL_INSET = 10;

export const pickKey = (key: string) => `${PICK_PREFIX}${key}`;
export const pickedKey = (hit: string | null | undefined): string | null => (hit?.startsWith(PICK_PREFIX) ? hit.slice(PICK_PREFIX.length) : null);

export function pickRowTops(list: BarsPickList): readonly number[] {
  const tops: number[] = [];
  let y = BARS_PAD_TOP;
  for (const row of list.rows) {
    if (row.divided) y += PICK_DIVIDER_GAP;
    tops.push(y);
    y += PICK_ROW_HEIGHT;
  }
  return tops;
}

export function drawPickList(
  ctx: Canvas2D,
  list: BarsPickList,
  layout: BarsLayout,
  theme: FamilyTheme,
  inspectedKey: string | null,
  focusedKey: string | null,
): void {
  if (layout.pickWidth <= 0) return;
  const tokens = resolveKitTokens(theme);
  const tops = pickRowTops(list);
  const width = layout.pickWidth;
  ctx.save();
  ctx.fillStyle = theme.surface;
  ctx.fillRect(0, 0, layout.bodyLeft, layout.height);
  ctx.fillStyle = tokens.hairlineStrong;
  ctx.fillRect(Math.round(width + (layout.bodyLeft - width) / 2), 0, 1, layout.height);
  const rowFont = `${LABEL_WEIGHT} ${theme.fontSize.body}px ${theme.font}`;
  ctx.font = rowFont;
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  for (let i = 0; i < list.rows.length; i++) {
    const row = list.rows[i]!;
    const top = tops[i]!;
    const key = pickKey(row.key);
    if (row.divided) {
      ctx.fillStyle = tokens.hairline;
      ctx.fillRect(LABEL_INSET, top - PICK_DIVIDER_GAP / 2, width - LABEL_INSET * 2, 1);
    }
    if (list.selectedKey === row.key) drawRowSelected(ctx, 4, top, width - 8, PICK_ROW_HEIGHT, theme);
    else if (inspectedKey === key) drawRowHover(ctx, 4, top, width - 8, PICK_ROW_HEIGHT, theme);
    if (focusedKey === key) drawRowFocusRing(ctx, 4, top, width - 8, PICK_ROW_HEIGHT, theme);
    const middle = top + PICK_ROW_HEIGHT / 2;
    const duelX = width - LABEL_INSET - DUEL_WIDTH;
    ctx.fillStyle = theme.text;
    ctx.fillText(ellipsiseCanvasText(ctx, row.label, Math.max(0, duelX - LABEL_INSET * 2), rowFont), LABEL_INSET, middle);
    drawDuel(ctx, row, duelX, Math.round(middle - DUEL_HEIGHT / 2), tokens.barTrack, theme.grey);
  }
  ctx.restore();
}

function drawDuel(ctx: Canvas2D, row: BarsPickRow, x: number, y: number, track: string, unknown: string): void {
  const left = row.left.fraction;
  const right = row.right.fraction;
  if (left === null || right === null || left + right <= 0) {
    drawBar(ctx, x, y, DUEL_WIDTH, DUEL_HEIGHT, track, 'right');
    return;
  }
  const total = left + right;
  const leftWidth = Math.round(DUEL_WIDTH * (left / total));
  if (leftWidth > 0) drawBar(ctx, x, y, leftWidth, DUEL_HEIGHT, row.left.color ?? unknown, 'left');
  if (DUEL_WIDTH - leftWidth > 0) drawBar(ctx, x + leftWidth, y, DUEL_WIDTH - leftWidth, DUEL_HEIGHT, row.right.color ?? unknown, 'right');
}

export function drawCaptionSides(ctx: Canvas2D, sides: readonly BarsCaptionSide[], right: number, middle: number, theme: FamilyTheme, artCache?: LaneArtCache): void {
  const flagW = 20;
  const flagH = 13;
  const label = `${LABEL_WEIGHT} ${theme.fontSize.small}px ${theme.font}`;
  const value = `600 ${theme.fontSize.small}px ${theme.font}`;
  let x = right;
  ctx.save();
  ctx.textBaseline = 'middle';
  ctx.textAlign = 'left';
  const pieces = sides.map((side, at) => {
    const labelW = measureCanvasText(ctx, side.label, label);
    const valueW = measureCanvasText(ctx, side.value, value);
    return { side, at, labelW, valueW, width: flagW + 6 + labelW + 5 + valueW };
  });
  for (let p = pieces.length - 1; p >= 0; p--) {
    const piece = pieces[p]!;
    x -= piece.width;
    const flagX = piece.at === pieces.length - 1 ? x + piece.labelW + 5 + piece.valueW + 6 : x;
    const textX = piece.at === pieces.length - 1 ? x : x + flagW + 6;
    drawMediaSlot(ctx, flagX, middle, flagW, flagH, piece.side.media, theme, { artCache, radius: 2 });
    ctx.font = label;
    ctx.fillStyle = theme.muted;
    ctx.fillText(piece.side.label, textX, middle);
    ctx.font = value;
    ctx.fillStyle = theme.text;
    ctx.fillText(piece.side.value, textX + piece.labelW + 5, middle);
    x -= 16;
  }
  ctx.restore();
}

export function hitPickList(list: BarsPickList, layout: BarsLayout, point: { readonly x: number; readonly y: number }): string | null {
  if (layout.pickWidth <= 0 || point.x >= layout.pickWidth) return null;
  const tops = pickRowTops(list);
  for (let i = 0; i < list.rows.length; i++) {
    const top = tops[i]!;
    if (point.y >= top && point.y < top + PICK_ROW_HEIGHT) return pickKey(list.rows[i]!.key);
  }
  return null;
}

export function pickNav(list: BarsPickList): readonly (readonly [string, string, number, number, string, unknown, number, number])[] {
  const tops = pickRowTops(list);
  return list.rows.map((row, at) => [pickKey(row.key), row.label, at, -1, '', { pick: row.key }, 0, tops[at]! + PICK_ROW_HEIGHT] as const);
}

export function pickTooltip(list: BarsPickList, key: string, hour: number): FamilyTooltipPayload | null {
  const row = list.rows.find(item => item.key === key);
  if (!row) return null;
  return row.tooltip ? (row.tooltip.hour !== undefined ? row.tooltip : { ...row.tooltip, hour }) : { title: row.label, fullText: row.label, hour };
}

export function pickAudit(list: BarsPickList): FamilyAuditRow[] {
  return list.rows.map(row => auditCell(pickKey(row.key), 'pick', JSON.stringify([row.label, row.left.fraction, row.right.fraction, list.selectedKey === row.key, row.divided === true])));
}
