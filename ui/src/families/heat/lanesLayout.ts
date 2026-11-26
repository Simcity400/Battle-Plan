import { ROW_SECTION_HEIGHT, rowGripBox, rowRemoveBox } from '../../draw/kit.ts';
import type { HeatLaneCell, HeatLaneLegend, HeatLaneRegions, HeatLaneRow, HeatLaneRun } from './lanesModel.ts';

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
  readonly rowTops: readonly number[];
  readonly sections: readonly { readonly label: string; readonly top: number }[];
  readonly addTop: number | null;
  readonly addVisible: boolean;
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
  readonly regions?: HeatLaneRegions;
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

  const sectionAt = new Map<number, string>();
  if (input.regions) {
    const firstPicked = input.rows.findIndex(row => row.pinned);
    const firstAuto = input.rows.findIndex(row => !row.pinned);
    if (firstPicked >= 0) sectionAt.set(firstPicked, input.regions.pickedHeading);
    if (firstAuto >= 0) sectionAt.set(firstAuto, input.regions.automaticHeading);
  }
  const units = input.rows.length + (input.regions ? 1 : 0);
  const sectionsHeight = sectionAt.size * ROW_SECTION_HEIGHT;
  const shared = units > 0 ? Math.floor((plotHeight - 8 - sectionsHeight + LANE_ROW_GAP) / units) - LANE_ROW_GAP : LANE_ROW_DEFAULT;
  const rowHeight = Math.max(
    LANE_ROW_MIN,
    Math.min(LANE_ROW_MAX, Math.round(input.requestedRowHeight ?? shared)),
  );
  const rowStride = rowHeight + LANE_ROW_GAP;
  const rowTops: number[] = [];
  const sections: { label: string; top: number }[] = [];
  let y = 0;
  for (let i = 0; i < input.rows.length; i++) {
    const label = sectionAt.get(i);
    if (label !== undefined) {
      sections.push({ label, top: y });
      y += ROW_SECTION_HEIGHT;
    }
    rowTops.push(y);
    y += rowStride;
  }
  const addTop = input.regions ? y : null;
  if (addTop !== null) y += rowStride;
  const contentHeight = y > 0 ? y - LANE_ROW_GAP + 8 : 0;
  const totalHeight = Math.max(height, plotTop + contentHeight + axisHeight);

  const window = lanesWindow({ rowTops, rowHeight, plotHeight, addTop }, input.scrollTop);

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
    rowTops,
    sections,
    addTop,
    totalHeight,
    ...window,
    firstHour,
    lastHour,
    displayedHour: input.displayedHour,
  };
}

export function lanesWindow(layout: Pick<HeatLanesLayout, 'rowTops' | 'rowHeight' | 'plotHeight' | 'addTop'>, scrollTopRaw: number): Pick<HeatLanesLayout, 'firstRow' | 'endRow' | 'addVisible'> {
  const scrollTop = Math.max(0, Math.round(scrollTopRaw));
  const at = rowAtOrBelow(layout.rowTops, scrollTop);
  const firstRow = layout.rowTops[at] === scrollTop ? at : at + 1;
  const endRow = rowAtOrBelow(layout.rowTops, scrollTop + layout.plotHeight - layout.rowHeight) + 1;
  const addVisible = layout.addTop !== null && layout.addTop >= scrollTop && layout.addTop + layout.rowHeight <= scrollTop + layout.plotHeight;
  return { firstRow, endRow: Math.max(firstRow, endRow), addVisible };
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

type DrawnSpan = { -readonly [K in keyof PixelLaneSpan]: PixelLaneSpan[K] };

export class LanePixels {
  runs?: readonly HeatLaneRun[];
  firstHour = 0;
  lastHour = 0;
  displayedHour = 0;
  plotLeft = 0;
  plotWidth = 0;
  readonly spans: DrawnSpan[] = [];
}

export function projectLaneRunsToPixels(
  runs: readonly HeatLaneRun[],
  firstHour: number,
  lastHour: number,
  displayedHour: number,
  plotLeft: number,
  plotWidth: number,
  pixels = new LanePixels(),
): readonly PixelLaneSpan[] {
  const out = pixels.spans;
  if (pixels.runs === runs && pixels.firstHour === firstHour && pixels.lastHour === lastHour && pixels.displayedHour === displayedHour
    && pixels.plotLeft === plotLeft && pixels.plotWidth === plotWidth) return out;
  pixels.runs = runs;
  pixels.firstHour = firstHour;
  pixels.lastHour = lastHour;
  pixels.displayedHour = displayedHour;
  pixels.plotLeft = plotLeft;
  pixels.plotWidth = plotWidth;
  if (!runs.length || plotWidth <= 0 || displayedHour < firstHour) {
    out.length = 0;
    return out;
  }
  const effectiveLast = Math.max(firstHour, lastHour);
  let count = 0;
  let lo = 0;
  let hi = runs.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (runs[mid]!.lastHour < firstHour) lo = mid + 1;
    else hi = mid;
  }
  let start = 0;
  let end = 0;
  let cell: HeatLaneCell | undefined;
  for (let i = lo; i <= runs.length; i++) {
    const run = runs[i];
    if (run && run.firstHour <= displayedHour) {
      if (run.readHour !== null && run.readHour > displayedHour) continue;
      const clippedFirst = Math.max(firstHour, run.firstHour);
      const clippedLast = Math.min(effectiveLast, displayedHour, run.lastHour);
      if (clippedFirst > clippedLast) continue;
      if (cell && end + 1 === clippedFirst && sameLaneCell(cell, run.cell)) {
        end = clippedLast;
        continue;
      }
    }
    if (cell) {
      const x = hourToLaneX(start, firstHour, effectiveLast, plotLeft, plotWidth);
      const right = hourToLaneX(end + 1, firstHour, effectiveLast, plotLeft, plotWidth);
      const width = Math.max(0, right - x);
      const span = out[count++];
      if (span) {
        span.x = x;
        span.width = width;
        span.firstHour = start;
        span.lastHour = end;
        span.cell = cell;
      } else out.push({ x, width, firstHour: start, lastHour: end, cell });
    }
    if (!run || run.firstHour > displayedHour) break;
    start = Math.max(firstHour, run.firstHour);
    end = Math.min(effectiveLast, displayedHour, run.lastHour);
    cell = run.cell;
  }

  out.length = count;
  if (out.length > 0) {
    const last = out[out.length - 1]!;
    const isTerminalCurrentDay = last.lastHour === displayedHour && last.width < 1 && plotWidth >= 2;
    if (isTerminalCurrentDay) {
      const terminalX = Math.max(plotLeft, Math.min(plotLeft + plotWidth - 1, last.x));
      if (out.length >= 2) {
        const prev = out[out.length - 2]!;
        if (prev.x + prev.width > terminalX) {
          prev.width = Math.max(0, terminalX - prev.x);
        }
      }
      last.x = terminalX;
      last.width = 1;
    }
  }

  return out;
}

function rowAtOrBelow(tops: readonly number[], y: number): number {
  let lo = 0;
  let hi = tops.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (tops[mid]! <= y) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

export type LaneHit =
  | { readonly kind: 'lane'; readonly rowIndex: number; readonly hour: number }
  | { readonly kind: 'row'; readonly rowIndex: number }
  | { readonly kind: 'remove' | 'grip'; readonly rowIndex: number }
  | { readonly kind: 'add' }
  | null;

export function hitHeatLanes(
  x: number,
  y: number,
  scrollTop: number,
  layout: HeatLanesLayout,
  rows: readonly HeatLaneRow[],
  toolsRow: number | null = null,
): LaneHit {
  if (x < 0 || x > layout.width || y < layout.plotTop || y > layout.height - layout.axisHeight) return null;
  const relY = y - layout.plotTop + scrollTop;
  if (layout.addVisible && layout.addTop !== null && relY >= layout.addTop && relY <= layout.addTop + layout.rowHeight) return { kind: 'add' };
  const rowIndex = rowAtOrBelow(layout.rowTops, relY);
  if (rowIndex < layout.firstRow || rowIndex >= layout.endRow || rowIndex >= rows.length) return null;
  const inRowY = relY - layout.rowTops[rowIndex]!;
  if (inRowY > layout.rowHeight) return null;
  if (toolsRow === rowIndex && rows[rowIndex]!.region !== undefined) {
    const middleY = layout.plotTop + layout.rowTops[rowIndex]! - scrollTop + layout.rowHeight / 2;
    const inside = (box: { readonly x: number; readonly y: number; readonly width: number; readonly height: number }) =>
      x >= box.x && x <= box.x + box.width && y >= box.y && y <= box.y + box.height;
    if (inside(rowRemoveBox(layout.nameWidth, middleY))) return { kind: 'remove', rowIndex };
    if (inside(rowGripBox(LANE_PAD, middleY))) return { kind: 'grip', rowIndex };
  }
  if (x >= layout.plotLeft && x <= layout.plotLeft + layout.plotWidth) {
    const hour = laneXToHour(x, layout.firstHour, layout.lastHour, layout.plotLeft, layout.plotWidth);
    return hour > layout.displayedHour ? null : { kind: 'lane', rowIndex, hour };
  }
  return { kind: 'row', rowIndex };
}
