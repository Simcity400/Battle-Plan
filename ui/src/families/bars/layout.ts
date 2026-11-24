import { linearTicks } from '../../charts/valueAxis.ts';
import { FLAG_SLOT_HEIGHT, FLAG_SLOT_WIDTH, ROW_SECTION_HEIGHT } from '../../draw/kit.ts';
import type { FamilyTheme } from '../contract.ts';
import type { BarsDataRow, BarsForm, BarsModel, BarsTwoSidedLayout } from './model.ts';

export const BARS_PAD_X = 12;
export const BARS_PAD_TOP = 6;
export const BARS_PAD_BOTTOM = 6;
export const BARS_HEADER_HEIGHT = 22;
export const BARS_AXIS_HEIGHT = 20;
export const BARS_LEGEND_ROW_HEIGHT = 22;
export const BARS_VALUE_GAP = 6;
export const BARS_SPLIT_CENTER_GAP = 36;
export const BARS_GRIP_WIDTH = 10;
export const PICK_WIDTH = 330;
export const PICK_GAP = 24;
export const PICK_ROW_HEIGHT = 22;
export const PICK_DIVIDER_GAP = 8;

export function pickListWidth(width: number): number {
  return width >= PICK_WIDTH + PICK_GAP + 360 ? PICK_WIDTH : 0;
}

export interface BarsLayout {
  readonly form: BarsForm;
  readonly bodyLeft: number;
  readonly pickWidth: number;
  readonly twoSidedLayout: BarsTwoSidedLayout;
  readonly width: number;
  readonly height: number;
  readonly headerTop: number;
  readonly headerHeight: number;
  readonly bodyTop: number;
  readonly bodyBottom: number;
  readonly bodyHeight: number;
  readonly axisTop: number;
  readonly axisHeight: number;
  readonly legendTop: number;
  readonly legendHeight: number;
  readonly dataRowHeight: number;
  readonly barHeight: number;
  readonly mediaWidth: number;
  readonly mediaHeight: number;
  readonly mediaGap: number;
  readonly nameX: number;
  readonly nameWidth: number;
  readonly plotLeft: number;
  readonly plotRight: number;
  readonly plotWidth: number;
  readonly centerX: number;
  readonly valueLeftGutter: number;
  readonly valueRightGutter: number;
  readonly readoutX: number;
  readonly readoutWidth: number;
  readonly pairX: number;
  readonly pairWidth: number;
  readonly rightPlotLeft: number;
  readonly rightPlotRight: number;
  readonly rightNameX: number;
  readonly rightNameWidth: number;
  readonly showReadout: boolean;
  readonly showPair: boolean;
  readonly showAxis: boolean;
  readonly valueInsideBar: boolean;
  readonly totalHeight: number;
  readonly rowTops: readonly number[];
  readonly rowHeights: readonly number[];
  readonly scaleMax: number;
  readonly scaleLeftMax: number;
  readonly scaleRightMax: number;
}

function hasAnyMedia(model: BarsModel): { kind: 'none' | 'flag' | 'art'; size: number } {
  let sawFlag = false;
  let sawArt = false;
  for (let i = 0; i < model.rows.length; i++) {
    const row = model.rows[i]!;
    if (row.kind !== 'data') continue;
    if (row.media?.kind === 'art') sawArt = true;
    else if (row.media?.kind === 'flag') sawFlag = true;
    else if (row.rightSide?.media?.kind === 'flag') sawFlag = true;
  }
  if (sawArt) return { kind: 'art', size: model.form === 'net' ? 22 : 16 };
  if (sawFlag) return { kind: 'flag', size: FLAG_SLOT_WIDTH };
  return { kind: 'none', size: 0 };
}

function resolveRowMetrics(model: BarsModel, bodyHeight: number): { dataRowHeight: number; barHeight: number } {
  const dataCount = model.rows.reduce((n, r) => (r.kind === 'data' ? n + 1 : n), 0);
  if (model.form === 'net') {
    return { dataRowHeight: model.rowHeight ?? 48, barHeight: model.barHeight ?? 12 };
  }
  if (model.form === 'share') {
    return { dataRowHeight: model.rowHeight ?? 26, barHeight: model.barHeight ?? 14 };
  }
  if (model.form === 'stacked') {
    return { dataRowHeight: model.rowHeight ?? 28, barHeight: model.barHeight ?? 20 };
  }
  if (model.form === 'two-sided' && model.twoSidedLayout === 'stacked-pair') {
    return { dataRowHeight: model.rowHeight ?? 26, barHeight: model.barHeight ?? 7 };
  }
  const dataRowHeight = model.rowHeight ?? (
    dataCount > 0 && dataCount <= 10 && bodyHeight >= dataCount * 30
      ? 30
      : dataCount > 0 && dataCount * 24 <= bodyHeight
        ? 24
        : 20
  );
  const barHeight = model.barHeight ?? (dataRowHeight >= 24 ? 12 : 10);
  return { dataRowHeight, barHeight };
}

export function axisCeiling(dataMax: number): number {
  if (!(dataMax > 0) || !Number.isFinite(dataMax)) return dataMax;
  const ticks = linearTicks(0, dataMax, 4);
  const last = ticks[ticks.length - 1] ?? dataMax;
  if (last >= dataMax) return last;
  const step = ticks.length > 1 ? ticks[1]! - ticks[0]! : dataMax;
  return last + step;
}

function positiveScale(provided: number | null | undefined, fallback: number): number {
  if (provided !== undefined && provided !== null && Number.isFinite(provided) && provided > 0) return provided;
  return fallback > 0 ? fallback : 1;
}

export function layoutFamilyBars(
  model: BarsModel,
  width: number,
  height: number,
  theme?: FamilyTheme,
  measure?: (text: string) => number,
): BarsLayout {
  const pickWidth = model.pickList ? pickListWidth(width) : 0;
  if (pickWidth <= 0) return layoutBarsBody(model, width, height, theme, measure);
  const offset = pickWidth + PICK_GAP;
  const body = layoutBarsBody(model, Math.max(1, width - offset), height, theme, measure);
  return {
    ...body,
    width: Math.max(1, Math.floor(width)),
    bodyLeft: offset,
    pickWidth,
    nameX: body.nameX + offset,
    plotLeft: body.plotLeft + offset,
    plotRight: body.plotRight + offset,
    centerX: body.centerX + offset,
    readoutX: body.readoutX + offset,
    pairX: body.pairX + offset,
    rightPlotLeft: body.rightPlotLeft + offset,
    rightPlotRight: body.rightPlotRight + offset,
    rightNameX: body.rightNameX + offset,
  };
}

function layoutBarsBody(
  model: BarsModel,
  width: number,
  height: number,
  _theme?: FamilyTheme,
  measure?: (text: string) => number,
): BarsLayout {
  const w = Math.max(1, Math.floor(width));
  const h = Math.max(1, Math.floor(height));
  const form = model.form;
  const twoSidedLayout: BarsTwoSidedLayout = model.twoSidedLayout ?? 'paired';
  const hasHeader = Boolean(model.caption || model.leftHeader || model.rightHeader || (twoSidedLayout === 'stacked-pair' && model.legend?.length));
  const headerTop = BARS_PAD_TOP;
  const headerHeight = hasHeader ? BARS_HEADER_HEIGHT : 0;
  const hasLegend = (form === 'stacked' || form === 'share') && Boolean(model.legend?.length || model.scaleUnit);
  let legendRows = hasLegend ? 1 : 0;
  if (hasLegend && model.legend?.length && h >= 240) {
    const entryGap = 14;
    const rightReserve = model.scaleUnit ? Math.min(Math.floor(w * 0.3), Math.ceil(measure ? measure(model.scaleUnit) : model.scaleUnit.length * 6.5)) + entryGap : 0;
    const avail = Math.max(80, w - BARS_PAD_X * 2 - rightReserve);
    let totalW = 0;
    for (let i = 0; i < model.legend.length; i++) {
      const entry = model.legend[i]!;
      const text = entry.share ? `${entry.label} ${entry.share}` : entry.value ? `${entry.label} ${entry.value}` : entry.label;
      const itemW = 18 + (measure ? measure(text) : text.length * 6.5);
      totalW += (i > 0 ? entryGap : 0) + itemW;
    }
    if (totalW > avail) legendRows = 2;
  }
  const legendHeight = legendRows * BARS_LEGEND_ROW_HEIGHT;
  const wantAxis = (model.showAxis ?? (form === 'ordered')) && h >= 140 && w >= 240;
  const axisHeight = wantAxis ? BARS_AXIS_HEIGHT : 0;

  const bodyTop = headerTop + headerHeight;
  const bodyBottom = Math.max(bodyTop + 1, h - BARS_PAD_BOTTOM - axisHeight - legendHeight);
  const bodyHeight = Math.max(1, bodyBottom - bodyTop);
  const axisTop = bodyBottom;
  const legendTop = axisTop + axisHeight;

  const { dataRowHeight, barHeight } = resolveRowMetrics(model, bodyHeight);
  const media = hasAnyMedia(model);
  const mediaWidth = media.kind === 'none' ? 0 : media.size;
  const mediaHeight = media.kind === 'flag' ? FLAG_SLOT_HEIGHT : media.kind === 'art' ? media.size : 0;
  const mediaGap = mediaWidth > 0 ? 8 : 0;

  const rowTops: number[] = [];
  const rowHeights: number[] = [];
  let cursorY = 0;
  for (let i = 0; i < model.rows.length; i++) {
    const row = model.rows[i]!;
    rowTops.push(cursorY);
    const rh = row.kind === 'section' ? ROW_SECTION_HEIGHT : dataRowHeight;
    rowHeights.push(rh);
    cursorY += rh;
  }
  const totalHeight = cursorY;

  let maxMeasuredLabel = 0;
  if (measure) {
    for (let i = 0; i < model.rows.length; i++) {
      const row = model.rows[i]!;
      if (row.kind === 'data') {
        maxMeasuredLabel = Math.max(maxMeasuredLabel, measure(row.label));
      }
    }
  }

  let nameX = BARS_PAD_X + mediaWidth + mediaGap;
  let nameWidth = 0;
  let plotLeft = BARS_PAD_X;
  let plotRight = w - BARS_PAD_X;
  let centerX = Math.round(w / 2);
  let valueLeftGutter = 0;
  let valueRightGutter = 0;
  let readoutX = w - BARS_PAD_X;
  let readoutWidth = 0;
  let pairX = w - BARS_PAD_X;
  let pairWidth = 0;
  let rightPlotLeft = centerX;
  let rightPlotRight = w - BARS_PAD_X;
  let rightNameX = w - BARS_PAD_X;
  let rightNameWidth = 0;
  let showReadout = false;
  let showPair = false;
  let valueInsideBar = false;

  if (form === 'net') {
    const narrow = w < 480;
    showPair = !narrow && model.rows.some(r => r.kind === 'data' && Boolean(r.pair));
    showReadout = true;
    nameWidth = narrow ? 92 : Math.min(140, Math.max(92, Math.round(w * 0.2)));
    readoutWidth = narrow ? 54 : 84;
    pairWidth = showPair ? 92 : 0;
    pairX = w - BARS_PAD_X - pairWidth;
    readoutX = pairX - (showPair ? 12 : 0) - readoutWidth;
    plotLeft = nameX + nameWidth + 10;
    plotRight = Math.max(plotLeft + 40, readoutX - 12);
    centerX = Math.round((plotLeft + plotRight) / 2);
  } else if (form === 'share') {
    const narrow = w < 380;
    showReadout = !narrow && model.rows.some(r => r.kind === 'data' && Boolean(r.readout));
    nameWidth = narrow ? 92 : Math.min(170, Math.max(100, Math.round(w * 0.26)));
    readoutWidth = showReadout ? 96 : 0;
    readoutX = w - BARS_PAD_X - readoutWidth;
    plotLeft = nameX + nameWidth + 10;
    plotRight = Math.max(plotLeft + 40, readoutX - (showReadout ? 10 : 0));
    centerX = Math.round((plotLeft + plotRight) / 2);
  } else if (form === 'stacked') {
    const hasLabels = model.rows.some(r => r.kind === 'data' && r.label.trim().length > 0);
    const hasReadout = w >= 360 && model.rows.some(r => r.kind === 'data' && Boolean(r.readout));
    showReadout = hasReadout;
    nameWidth = hasLabels ? (w < 380 ? 88 : Math.min(150, Math.round(w * 0.24))) : 0;
    readoutWidth = hasReadout ? 84 : 0;
    readoutX = w - BARS_PAD_X - readoutWidth;
    plotLeft = nameWidth > 0 ? nameX + nameWidth + 10 : BARS_PAD_X;
    plotRight = Math.max(plotLeft + 40, readoutX - (hasReadout ? 10 : 0));
    centerX = Math.round((plotLeft + plotRight) / 2);
  } else if (form === 'two-sided' && twoSidedLayout === 'split-sides') {
    const narrow = w < 520;
    const sideNameW = narrow ? 72 : Math.min(150, Math.max(96, Math.round(w * 0.2)));
    const valW = narrow ? 44 : 58;
    const halfGap = Math.round(BARS_SPLIT_CENTER_GAP / 2);
    centerX = Math.round(w / 2);
    nameWidth = sideNameW;
    plotLeft = nameX + nameWidth + 8;
    plotRight = Math.max(plotLeft + 24, centerX - halfGap - valW);
    valueLeftGutter = valW;
    valueRightGutter = valW;
    rightPlotLeft = centerX + halfGap + valW;
    rightNameWidth = sideNameW;
    rightNameX = w - BARS_PAD_X - mediaWidth - mediaGap - rightNameWidth;
    rightPlotRight = Math.max(rightPlotLeft + 24, rightNameX - 8);
  } else if (form === 'two-sided' && twoSidedLayout === 'stacked-pair') {
    const narrow = w < 440;
    nameWidth = narrow ? 88 : Math.min(168, Math.max(100, Math.round(w * 0.2)));
    showReadout = model.rows.some(r => r.kind === 'data' && Boolean(r.readout));
    readoutWidth = showReadout ? (narrow ? 104 : 128) : 0;
    readoutX = w - BARS_PAD_X - readoutWidth;
    plotLeft = nameX + nameWidth + 10;
    plotRight = Math.max(plotLeft + 40, readoutX - (showReadout ? 12 : 0));
    centerX = Math.round((plotLeft + plotRight) / 2);
  } else if (form === 'two-sided') {
    const narrow = w < 440;
    nameWidth = narrow ? 88 : Math.min(168, Math.max(100, Math.round(w * 0.24)));
    valueLeftGutter = narrow ? 48 : 64;
    valueRightGutter = narrow ? 48 : 64;
    showReadout = !narrow && model.rows.some(r => r.kind === 'data' && Boolean(r.readout));
    readoutWidth = showReadout ? 64 : 0;
    readoutX = w - BARS_PAD_X - readoutWidth;
    plotLeft = nameX + nameWidth + 10;
    plotRight = Math.max(plotLeft + 60, (showReadout ? readoutX - 8 : w - BARS_PAD_X));
    centerX = Math.round((plotLeft + plotRight) / 2);
  } else {
    const hasReadoutColumn = model.rows.some(r => r.kind === 'data' && Boolean(r.readout));
    const narrow = w < 340;
    const desiredName = maxMeasuredLabel > 0
      ? Math.min(240, Math.max(92, Math.ceil(maxMeasuredLabel) + 8))
      : Math.min(240, Math.max(92, Math.round(w * 0.26)));
    const maxNameRoom = Math.max(76, Math.round(w * 0.4));
    nameWidth = Math.min(desiredName, maxNameRoom);
    plotLeft = nameX + nameWidth + 10;
    if (hasReadoutColumn) {
      showReadout = !narrow;
      readoutWidth = showReadout ? Math.min(148, Math.max(88, Math.round(w * 0.24))) : 0;
      readoutX = w - BARS_PAD_X - readoutWidth;
      plotRight = Math.max(plotLeft + 40, readoutX - (showReadout ? 10 : 0));
    } else {
      valueInsideBar = w < 300;
      valueRightGutter = valueInsideBar ? 0 : 62;
      plotRight = Math.max(plotLeft + 40, w - BARS_PAD_X - valueRightGutter);
    }
    centerX = Math.round((plotLeft + plotRight) / 2);
  }

  let dataMax = 0;
  let dataLeftMax = 0;
  let dataRightMax = 0;
  for (let i = 0; i < model.rows.length; i++) {
    const row = model.rows[i]!;
    if (row.kind !== 'data') continue;
    const d = row as BarsDataRow;
    if (d.numeric !== undefined && d.numeric !== null) dataMax = Math.max(dataMax, Math.abs(d.numeric));
    if (d.target) dataMax = Math.max(dataMax, Math.abs(d.target.numeric));
    if (d.left?.numeric !== undefined && d.left.numeric !== null) dataLeftMax = Math.max(dataLeftMax, Math.abs(d.left.numeric));
    if (d.right?.numeric !== undefined && d.right.numeric !== null) dataRightMax = Math.max(dataRightMax, Math.abs(d.right.numeric));
    if (d.rightSide?.numeric !== undefined && d.rightSide.numeric !== null) dataRightMax = Math.max(dataRightMax, Math.abs(d.rightSide.numeric));
  }
  const sharedTwoSidedMax = Math.max(dataMax, dataLeftMax, dataRightMax);
  const scaleMax = positiveScale(model.scaleMax, form === 'two-sided' ? sharedTwoSidedMax : wantAxis ? axisCeiling(dataMax) : dataMax);
  const scaleLeftMax = positiveScale(model.scaleLeftMax ?? model.scaleMax, sharedTwoSidedMax);
  const scaleRightMax = positiveScale(model.scaleRightMax ?? model.scaleMax, sharedTwoSidedMax);

  return {
    form,
    bodyLeft: 0,
    pickWidth: 0,
    twoSidedLayout,
    width: w,
    height: h,
    headerTop,
    headerHeight,
    bodyTop,
    bodyBottom,
    bodyHeight,
    axisTop,
    axisHeight,
    legendTop,
    legendHeight,
    dataRowHeight,
    barHeight,
    mediaWidth,
    mediaHeight,
    mediaGap,
    nameX,
    nameWidth,
    plotLeft,
    plotRight,
    plotWidth: Math.max(1, plotRight - plotLeft),
    centerX,
    valueLeftGutter,
    valueRightGutter,
    readoutX,
    readoutWidth,
    pairX,
    pairWidth,
    rightPlotLeft,
    rightPlotRight,
    rightNameX,
    rightNameWidth,
    showReadout,
    showPair,
    showAxis: wantAxis,
    valueInsideBar,
    totalHeight,
    rowTops,
    rowHeights,
    scaleMax,
    scaleLeftMax,
    scaleRightMax,
  };
}

export function visibleBarsRange(
  layout: BarsLayout,
  scrollTop: number,
  overscan = 2,
): { readonly first: number; readonly last: number } {
  const count = layout.rowTops.length;
  if (!count) return { first: 0, last: 0 };
  const topEdge = Math.max(0, scrollTop);
  const bottomEdge = topEdge + layout.bodyHeight;
  let lo = 0;
  let hi = count - 1;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (layout.rowTops[mid]! + layout.rowHeights[mid]! <= topEdge) lo = mid + 1;
    else hi = mid;
  }
  const first = Math.max(0, lo - overscan);
  let last = lo;
  while (last < count && layout.rowTops[last]! < bottomEdge) last++;
  return { first, last: Math.min(count, last + overscan) };
}

export function barsAuditWindow(layout: BarsLayout, scrollTop: number): { readonly first: number; readonly count: number } {
  const { first, last } = visibleBarsRange(layout, scrollTop, 0);
  return { first, count: last - first };
}

export function barsScrollHeight(layout: BarsLayout): number {
  return layout.totalHeight + layout.height - layout.bodyHeight;
}
