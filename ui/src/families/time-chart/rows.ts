import { fitMeasuredLine, type Canvas2D, type MeasureText } from '../../charts/textWidth.ts';
import { computeTimeAxisTicks, drawArtSlot, drawTimeAxis, resolveKitTokens, type TimeAxisTick } from '../../draw/kit.ts';
import { resolveCachedArt, toArtSlotState } from '../art/adapters.ts';
import type { LaneArtCache } from '../art/laneCache.ts';
import type { FamilyTheme } from '../contract.ts';
import type { FamilyTooltipEntry } from '../host/tooltips.ts';
import { drawHatchRect } from '../valueCanvas.ts';
import { formatDateOnly, notRecordedLabel } from '../valueFormat.ts';
import type { ChartRowModel, ChartRowMonth, TimeChartModel } from './model.ts';

const ICON = 22;
const BAR_SHARE = 0.38;
const AXIS_HEIGHT = 24;

export interface PlacedChartRow {
  readonly row: ChartRowModel;
  readonly index: number;
  readonly top: number;
  readonly height: number;
  readonly zeroY: number;
  readonly scale: number;
  readonly name: string;
  readonly nameTruncated: boolean;
}

export interface ChartRowsLayout {
  readonly nameLeft: number;
  readonly nameWidth: number;
  readonly valueRight: number;
  readonly rows: readonly PlacedChartRow[];
}

export interface RowsFrame {
  readonly left: number;
  readonly plotWidth: number;
  readonly top: number;
  readonly axisY: number;
  readonly xDomain: readonly [number, number];
  readonly toX: (hour: number) => number;
  readonly axisTicks: readonly TimeAxisTick[];
  readonly rows: ChartRowsLayout;
}

export function layoutChartRows(model: TimeChartModel, width: number, height: number, small: boolean, measure: MeasureText, fontFamily: string): RowsFrame {
  const rows = model.rows ?? [];
  const nameWidth = small ? 92 : 140;
  const valueWidth = small ? 40 : 64;
  const left = 12 + nameWidth + 8;
  const right = valueWidth + 20;
  const top = 6;
  const axisY = Math.max(top + 1, height - AXIS_HEIGHT);
  const plotWidth = Math.max(1, width - left - right);
  const xDomain: readonly [number, number] = [Math.min(model.extent[0], model.hour), Math.max(model.extent[0] + 1, model.hour)];
  const span = Math.max(1, xDomain[1] - xDomain[0]);
  const toX = (hour: number) => left + ((hour - xDomain[0]) / span) * plotWidth;
  const rowHeight = rows.length ? Math.max(18, Math.min(48, (axisY - top) / rows.length)) : 0;
  const nameFont = `500 12px ${fontFamily}`;
  const placed = rows.map((row, index) => {
    const rowTop = top + index * rowHeight;
    const scale = row.months.reduce((most, month) => Math.max(most, Math.abs(month.value ?? 0)), 0);
    const fitted = fitMeasuredLine(row.name, nameWidth - ICON - 8, nameFont, measure);
    return { row, index, top: rowTop, height: rowHeight, zeroY: rowTop + rowHeight / 2, scale, name: fitted.text, nameTruncated: fitted.truncated };
  });
  return {
    left, plotWidth, top, axisY, xDomain, toX,
    axisTicks: computeTimeAxisTicks(xDomain[0], xDomain[1], plotWidth),
    rows: { nameLeft: 12, nameWidth, valueRight: width - 12, rows: placed },
  };
}

function monthBox(frame: RowsFrame, month: ChartRowMonth): readonly [number, number] {
  const x0 = frame.toX(month.firstHour);
  const x1 = frame.toX(month.lastHour + 1);
  const width = x1 - x0;
  return width >= 4 ? [x0 + 0.5, width - 1] : [x0, Math.max(1, width)];
}

export function drawChartRows(ctx: Canvas2D, model: TimeChartModel, frame: RowsFrame, theme: FamilyTheme, inspect: { readonly row: number; readonly month: number } | null, artCache?: LaneArtCache): void {
  const tokens = resolveKitTokens(theme);
  const negative = tokens.shortFill;
  const nameFont = `500 ${theme.fontSize.body}px ${theme.font}`;
  const valueFont = `600 13px ${theme.font}`;
  for (const placed of frame.rows.rows) {
    const { row, top, height, zeroY, scale } = placed;
    const iconY = zeroY - ICON / 2;
    if (row.icon) drawArtSlot(ctx, frame.rows.nameLeft, iconY, ICON, ICON, toArtSlotState(resolveCachedArt(row.icon.art, artCache), row.icon.label), theme, { radius: 0 });
    ctx.save();
    ctx.font = nameFont;
    ctx.fillStyle = theme.text;
    ctx.textBaseline = 'middle';
    ctx.fillText(placed.name, frame.rows.nameLeft + ICON + 8, zeroY);
    ctx.font = valueFont;
    ctx.textAlign = 'right';
    ctx.fillStyle = row.valueMissing ? tokens.textSecondary : theme.text;
    ctx.fillText(row.valueMissing ? notRecordedLabel() : row.valueText, frame.rows.valueRight, zeroY);
    ctx.restore();
    const reach = height * BAR_SHARE;
    row.months.forEach((month, index) => {
      const [x, width] = monthBox(frame, month);
      if (month.value === null) {
        drawHatchRect(ctx, x, zeroY - reach, width, reach * 2, 0, theme);
        return;
      }
      const length = scale > 0 ? Math.max(month.value === 0 ? 0 : 1, (Math.abs(month.value) / scale) * reach) : 0;
      ctx.save();
      ctx.globalAlpha = inspect && inspect.row === placed.index && inspect.month === index ? 1 : 0.92;
      ctx.fillStyle = month.value < 0 ? negative : row.color;
      ctx.fillRect(x, month.value < 0 ? zeroY : zeroY - length, width, length);
      ctx.restore();
    });
    ctx.save();
    ctx.strokeStyle = tokens.zeroLine;
    ctx.beginPath();
    ctx.moveTo(frame.left, Math.floor(zeroY) + 0.5);
    ctx.lineTo(frame.left + frame.plotWidth, Math.floor(zeroY) + 0.5);
    ctx.stroke();
    ctx.restore();
    if (top < 0) break;
  }
  drawTimeAxis(ctx, frame.axisTicks, {
    plotLeft: frame.left,
    plotRight: frame.left + frame.plotWidth,
    startHour: frame.xDomain[0],
    endHour: frame.xDomain[1],
    toX: frame.toX,
    axisTop: frame.axisY,
    playhead: model.hour,
    playheadLabel: formatDateOnly(model.hour),
    playheadLineTop: frame.top,
    hairline: 'none',
    tickColor: tokens.zeroLine,
    theme,
  });
}

export function hitChartRows(frame: RowsFrame, point: readonly [number, number]): string | null {
  const [px, py] = point;
  const placed = frame.rows.rows.find(row => py >= row.top && py < row.top + row.height);
  if (!placed) return null;
  if (px < frame.left || px > frame.left + frame.plotWidth) return `row:${placed.index}`;
  const index = placed.row.months.findIndex(month => px >= frame.toX(month.firstHour) && px < frame.toX(month.lastHour + 1));
  return index < 0 ? `row:${placed.index}` : `cell:${placed.index}:${index}@${placed.row.months[index]!.firstHour}`;
}

export function parseRowsInspect(key: string | null | undefined): { readonly row: number; readonly month: number } | null {
  const match = /^cell:(\d+):(\d+)@/.exec(key ?? '');
  return match ? { row: Number(match[1]), month: Number(match[2]) } : null;
}

export function rowsTooltip(model: TimeChartModel, key: string | null | undefined): FamilyTooltipEntry | null {
  const rowMatch = /^row:(\d+)$/.exec(key ?? '');
  if (rowMatch && key) {
    const row = model.rows?.[Number(rowMatch[1])];
    if (!row) return null;
    return { hour: model.hour, rowKey: key, payload: { title: row.name, subtitle: formatDateOnly(model.hour), note: row.valueMissing ?? `${row.valueText} a day` } };
  }
  const cell = parseRowsInspect(key);
  if (!cell || !key) return null;
  const row = model.rows?.[cell.row];
  const month = row?.months[cell.month];
  if (!row || !month) return null;
  const span = `${formatDateOnly(month.firstHour)} – ${formatDateOnly(month.lastHour)}`;
  return {
    hour: month.firstHour,
    rowKey: key,
    payload: month.value === null
      ? { title: row.name, subtitle: span, note: month.reason ?? notRecordedLabel() }
      : { title: row.name, subtitle: span, note: `${month.valueText} a day${month.partial ? ' (days recorded only)' : ''}` },
  };
}
