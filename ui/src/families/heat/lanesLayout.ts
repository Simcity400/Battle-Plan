import { formatDateOnly } from '../valueFormat.ts';
import type { HeatLaneCell, HeatLaneLegend, HeatLaneRow, HeatLaneRun } from './lanesModel.ts';

export const LANE_NAME_WIDTH = 168;
export const LANE_NAME_COMPACT_WIDTH = 116;
export const LANE_READOUT_DEFAULT_WIDTH = 164;
export const LANE_READOUT_MIN_WIDTH = 150;
export const LANE_READOUT_MAX_WIDTH = 204;
export const LANE_ROW_MIN = 18;
export const LANE_ROW_DEFAULT = 26;
export const LANE_ROW_MAX = 28;
export const LANE_ROW_GAP = 4;
export const LANE_LEGEND_HEIGHT = 24;
export const LANE_AXIS_HEIGHT = 20;
export const LANE_PAD = 8;

export interface HeatLanesLayout {
  readonly width: number;
  readonly height: number;
  readonly legendHeight: number;
  readonly axisHeight: number;
  readonly nameWidth: number;
  readonly readoutWidth: number;
  readonly showReadout: boolean;
  readonly showFlags: boolean;
  readonly plotLeft: number;
  readonly plotWidth: number;
  readonly plotTop: number;
  readonly plotHeight: number;
  readonly rowHeight: number;
  readonly rowStride: number;
  readonly totalHeight: number;
  readonly firstRow: number;
  readonly endRow: number;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly displayedHour: number;
}

export interface LayoutHeatLanesInput {
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly displayedHour: number;
  readonly rows: readonly HeatLaneRow[];
  readonly legend?: HeatLaneLegend;
  readonly requestedReadoutWidth?: number;
  readonly requestedRowHeight?: number;
}

export function layoutHeatLanes(input: LayoutHeatLanesInput): HeatLanesLayout {
  const width = Math.max(1, Math.floor(input.width));
  const height = Math.max(1, Math.floor(input.height));
  const hasLegend = Boolean(
    input.legend && ((input.legend.entries?.length ?? 0) > 0 || input.legend.gradient || input.legend.rightNote),
  );
  const legendHeight = hasLegend ? LANE_LEGEND_HEIGHT : 0;
  const axisHeight = LANE_AXIS_HEIGHT;

  const showFlags = input.rows.some(row => row.flag !== undefined || row.tag !== undefined);
  const hasReadout = input.rows.some(row => row.readout !== undefined);
  const nameWidth = width < 420 ? LANE_NAME_COMPACT_WIDTH : LANE_NAME_WIDTH;
  const baseReadout = Math.max(
    LANE_READOUT_MIN_WIDTH,
    Math.min(LANE_READOUT_MAX_WIDTH, input.requestedReadoutWidth ?? LANE_READOUT_DEFAULT_WIDTH),
  );
  const showReadout = hasReadout && width >= nameWidth + baseReadout + 80;
  const readoutWidth = showReadout ? baseReadout : 0;

  const plotLeft = nameWidth + LANE_PAD;
  const plotRight = width - readoutWidth - LANE_PAD;
  const plotWidth = Math.max(1, plotRight - plotLeft);
  const plotTop = legendHeight + 4;
  const plotHeight = Math.max(1, height - plotTop - axisHeight);

  const rowHeight = Math.max(
    LANE_ROW_MIN,
    Math.min(LANE_ROW_MAX, Math.round(input.requestedRowHeight ?? LANE_ROW_DEFAULT)),
  );
  const rowStride = rowHeight + LANE_ROW_GAP;
  const contentHeight = input.rows.length > 0 ? input.rows.length * rowStride - LANE_ROW_GAP + 8 : 0;
  const totalHeight = Math.max(height, plotTop + contentHeight + axisHeight);

  const scrollTop = Math.max(0, input.scrollTop);
  const firstRow = Math.max(0, Math.floor(scrollTop / rowStride) - 1);
  const endRow = Math.min(input.rows.length, Math.ceil((scrollTop + plotHeight) / rowStride) + 1);

  const firstHour = input.firstHour;
  const lastHour = Math.max(firstHour, input.lastHour);

  return {
    width,
    height,
    legendHeight,
    axisHeight,
    nameWidth,
    readoutWidth,
    showReadout,
    showFlags,
    plotLeft,
    plotWidth,
    plotTop,
    plotHeight,
    rowHeight,
    rowStride,
    totalHeight,
    firstRow,
    endRow,
    firstHour,
    lastHour,
    displayedHour: input.displayedHour,
  };
}

export function hourToLaneX(
  hour: number,
  firstHour: number,
  lastHour: number,
  plotLeft: number,
  plotWidth: number,
): number {
  const span = Math.max(1, lastHour - firstHour + 1);
  const clamped = Math.max(firstHour, Math.min(lastHour + 1, hour));
  return plotLeft + ((clamped - firstHour) / span) * plotWidth;
}

export function laneXToHour(
  x: number,
  firstHour: number,
  lastHour: number,
  plotLeft: number,
  plotWidth: number,
): number {
  const span = Math.max(1, lastHour - firstHour + 1);
  const ratio = Math.max(0, Math.min(1, (x - plotLeft) / Math.max(1, plotWidth)));
  return Math.min(lastHour, Math.floor(firstHour + ratio * span));
}

export interface PixelLaneSpan {
  readonly x: number;
  readonly width: number;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly cell: HeatLaneCell;
}

function sameLaneCell(a: HeatLaneCell, b: HeatLaneCell): boolean {
  if (a === b) return true;
  if (a.kind !== b.kind) return false;
  switch (a.kind) {
    case 'clear':
      return a.label === (b as typeof a).label;
    case 'missing':
      return a.reason === (b as typeof a).reason;
    case 'gap':
      return a.reason === (b as typeof a).reason;
    case 'solid': {
      const sb = b as typeof a;
      return a.color === sb.color && a.label === sb.label && a.textColor === sb.textColor;
    }
    case 'parts': {
      const pb = b as typeof a;
      if (a.look !== pb.look || a.label !== pb.label || a.color !== pb.color || a.parts.length !== pb.parts.length) {
        return false;
      }
      for (let i = 0; i < a.parts.length; i++) {
        const pa = a.parts[i]!;
        const q = pb.parts[i]!;
        if (pa.id !== q.id || pa.color !== q.color || Math.abs(pa.share - q.share) > 1e-4) return false;
      }
      return true;
    }
  }
}

export function projectLaneRunsToPixels(
  runs: readonly HeatLaneRun[],
  firstHour: number,
  lastHour: number,
  displayedHour: number,
  plotLeft: number,
  plotWidth: number,
): readonly PixelLaneSpan[] {
  if (!runs.length || plotWidth <= 0 || displayedHour < firstHour) return [];
  const effectiveLast = Math.max(firstHour, lastHour);
  const out: PixelLaneSpan[] = [];

  for (let i = 0; i < runs.length; i++) {
    const run = runs[i]!;
    if (run.firstHour > displayedHour) break;
    if (run.readHour !== null && run.readHour > displayedHour) continue;
    const clippedFirst = Math.max(firstHour, run.firstHour);
    const clippedLast = Math.min(effectiveLast, displayedHour, run.lastHour);
    if (clippedFirst > clippedLast) continue;

    const prev = out.at(-1);
    if (prev && prev.lastHour + 1 === clippedFirst && sameLaneCell(prev.cell, run.cell)) {
      const x0 = hourToLaneX(prev.firstHour, firstHour, effectiveLast, plotLeft, plotWidth);
      const x1 = hourToLaneX(clippedLast + 1, firstHour, effectiveLast, plotLeft, plotWidth);
      out[out.length - 1] = {
        ...prev,
        x: x0,
        width: Math.max(0, x1 - x0),
        lastHour: clippedLast,
      };
      continue;
    }

    const x0 = hourToLaneX(clippedFirst, firstHour, effectiveLast, plotLeft, plotWidth);
    const x1 = hourToLaneX(clippedLast + 1, firstHour, effectiveLast, plotLeft, plotWidth);
    out.push({
      x: x0,
      width: Math.max(0, x1 - x0),
      firstHour: clippedFirst,
      lastHour: clippedLast,
      cell: run.cell,
    });
  }

  if (out.length > 0) {
    const last = out[out.length - 1]!;
    const isTerminalCurrentDay = last.lastHour === displayedHour && last.width < 1 && plotWidth >= 2;
    if (isTerminalCurrentDay) {
      const terminalX = Math.max(plotLeft, Math.min(plotLeft + plotWidth - 1, last.x));
      if (out.length >= 2) {
        const prev = out[out.length - 2]!;
        if (prev.x + prev.width > terminalX) {
          out[out.length - 2] = { ...prev, width: Math.max(0, terminalX - prev.x) };
        }
      }
      out[out.length - 1] = {
        ...last,
        x: terminalX,
        width: Math.max(1, plotLeft + plotWidth - terminalX),
      };
    }
  }

  return out;
}

export type LaneHit =
  | { readonly kind: 'lane'; readonly rowIndex: number; readonly hour: number }
  | { readonly kind: 'row'; readonly rowIndex: number }
  | null;

export function hitHeatLanes(
  x: number,
  y: number,
  scrollTop: number,
  layout: HeatLanesLayout,
  rowCount: number,
): LaneHit {
  if (x < 0 || x > layout.width || y < layout.plotTop || y > layout.height - layout.axisHeight) return null;
  const relY = y - layout.plotTop + scrollTop;
  const rowIndex = Math.floor(relY / layout.rowStride);
  if (rowIndex < 0 || rowIndex >= rowCount) return null;
  const inRowY = relY - rowIndex * layout.rowStride;
  if (inRowY > layout.rowHeight) return null;
  if (x >= layout.plotLeft && x <= layout.plotLeft + layout.plotWidth) {
    const hour = laneXToHour(x, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
    return { kind: 'lane', rowIndex, hour: Math.min(hour, layout.displayedHour) };
  }
  return { kind: 'row', rowIndex };
}

export interface AxisTick {
  readonly hour: number;
  readonly x: number;
  readonly label: string;
}

export function computeAxisTicks(
  firstHour: number,
  lastHour: number,
  dayHours: number,
  plotLeft: number,
  plotWidth: number,
  minSpacingPx = 88,
): readonly AxisTick[] {
  if (plotWidth < 40 || lastHour < firstHour) return [];
  const maxTicks = Math.max(2, Math.floor(plotWidth / minSpacingPx));
  const totalHours = Math.max(1, lastHour - firstHour + 1);
  const stepDaysCandidates = [1, 2, 7, 14, 30, 60, 90, 180, 365, 730];
  const hoursPerDay = Math.max(1, dayHours);
  const rawStepDays = totalHours / hoursPerDay / maxTicks;
  const stepDays = stepDaysCandidates.find(c => c >= rawStepDays) ?? Math.ceil(rawStepDays / 365) * 365;
  const stepHours = stepDays * hoursPerDay;

  const firstTick = Math.ceil(firstHour / stepHours) * stepHours;
  const ticks: AxisTick[] = [];
  for (let h = firstTick; h <= lastHour; h += stepHours) {
    const x = hourToLaneX(h, firstHour, lastHour, plotLeft, plotWidth);
    if (x < plotLeft + 24 || x > plotLeft + plotWidth - 24) continue;
    ticks.push({ hour: h, x, label: formatDateOnly(h) });
  }
  return ticks;
}
