import { ellipsise, layoutEndLabels } from '../../charts/labels.ts';
import type { MeasureText } from '../../charts/textWidth.ts';
import { exactDecimal, linearTicks, logTicks, thinLogTicks } from '../../charts/valueAxis.ts';
import { monthNames } from '../../dashboard/calendar.ts';
import { computeTimeAxisTicks, type TimeAxisTick } from '../../draw/kit.ts';
import type { BoundedSeries } from '../plan.ts';
import { closedExtent, historyFits } from './history.ts';
import { layoutChartRows, type RowsFrame } from './rows.ts';
import { formatCompactScalar, formatDateOnly, formatScalar } from '../valueFormat.ts';
import type {
  ChartColumnModel,
  ChartGap,
  ChartMarker,
  ChartMonthSlot,
  ChartValueFormat,
  TimeChartModel,
  TimeChartSeriesModel,
} from './model.ts';

function shortMonth(month: number): string {
  return monthNames[month - 1]!.slice(0, 3);
}

const AXIS_MAX_DECIMALS = 12;

function axisDecimalPlaces(ticks: readonly number[] | undefined, value: number, cap: number): number {
  let places = 0;
  for (const tick of ticks?.length ? ticks : [value]) {
    if (Number.isFinite(tick)) places = Math.max(places, exactDecimal(tick).scale);
  }
  return Math.min(ticks?.length ? AXIS_MAX_DECIMALS : cap, places);
}

function compactCollides(ticks: readonly number[] | undefined, value: number, threshold: number): boolean {
  if (!ticks || ticks.length < 2) return !Number.isInteger(value) && Math.abs(value) >= threshold;
  const seenLabels = new Set<string>();
  const seenValues = new Set<number>();
  for (const tick of ticks) {
    if (!Number.isFinite(tick) || seenValues.has(tick)) continue;
    seenValues.add(tick);
    const label = tick === 0
      ? '0'
      : Math.abs(tick) >= threshold
        ? formatCompactScalar(Math.round(tick), 'integer').text
        : formatScalar(tick, 'integer');
    if (seenLabels.has(label)) return true;
    seenLabels.add(label);
  }
  return false;
}

export function formatChartAxisValue(value: number, format: ChartValueFormat, ticks?: readonly number[]): string {
  if (!Number.isFinite(value)) return '';
  if (format === 'percent' || format === 'share' || format === 'signedPercent') {
    const places = axisDecimalPlaces(ticks, value, 2);
    return formatScalar(value, { decimals: 0, maxDecimals: places, fixedDecimals: places > 0, percent: true });
  }
  if (format === 'signed1') {
    const places = axisDecimalPlaces(ticks, value, 1);
    return formatScalar(value, { decimals: 0, maxDecimals: places, fixedDecimals: places > 0, signed: true });
  }
  const places = axisDecimalPlaces(ticks, value, 2);
  if (format === 'compact') {
    if (places === 0 && !compactCollides(ticks, value, 0)) {
      return value === 0 ? '0' : formatCompactScalar(Math.round(value), 'integer').text;
    }
    return formatScalar(value, { decimals: 0, maxDecimals: places, fixedDecimals: places > 0 });
  }
  if (format === 'power' && places === 0 && Math.abs(value) >= 1e4 && !compactCollides(ticks, value, 1e4)) {
    return formatCompactScalar(Math.round(value), 'integer').text;
  }
  return formatScalar(value, { decimals: 0, maxDecimals: places, fixedDecimals: places > 0 });
}

export interface PlacedEndLabel {
  readonly id: string;
  readonly seriesId: string;
  readonly anchorY: number;
  readonly y: number;
  readonly x: number;
  readonly width: number;
  readonly height: number;
  readonly color?: string;
  readonly faded: boolean;
  readonly colorBar?: string;
  readonly flag?: TimeChartSeriesModel['flag'];
  readonly icon?: TimeChartSeriesModel['icon'];
  readonly name: string | null;
  readonly fullName: string;
  readonly value: string;
  readonly valueColor?: string;
  readonly valueMuted?: boolean;
  readonly negative?: boolean;
  readonly hasLeader: boolean;
}

export interface PlacedMarker {
  readonly index: number;
  readonly marker: ChartMarker;
  readonly x: number;
  readonly y: number;
  readonly kind: 'icon' | 'flag' | 'tick';
  readonly size: number;
}

export function hoverTimeChartLayout(base: TimeChartLayout, hoveredSeriesId: string | null): TimeChartLayout {
  const endLabels = hoveredSeriesId === null
    ? base.endLabels
    : base.endLabels.map(label => (label.id === TOTAL_LABEL || label.seriesId === hoveredSeriesId ? label : { ...label, faded: true }));
  return endLabels === base.endLabels ? base : { ...base, endLabels };
}

export interface PlacedColumnItem {
  readonly kind: 'fold' | 'month';
  readonly index: number;
  readonly x: number;
  readonly width: number;
  readonly barX: number;
  readonly barWidth: number;
  readonly label: string | null;
  readonly majorLabel: boolean;
  readonly month?: ChartMonthSlot;
  readonly foldMonths?: readonly ChartMonthSlot[];
  readonly foldLabel?: string;
  readonly foldSpanText?: string;
}

export interface PlacedGap {
  readonly index: number;
  readonly gap: ChartGap;
  readonly ongoing: boolean;
  readonly x0: number;
  readonly x1: number;
}

export interface TimeChartLayout {
  readonly width: number;
  readonly height: number;
  readonly small: boolean;
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
  readonly plotWidth: number;
  readonly plotHeight: number;
  readonly baseY: number;
  readonly markerLaneHeight: number;
  readonly axisY: number;
  readonly xDomain: readonly [number, number];
  readonly yDomain: readonly [number, number];
  readonly logShown: boolean;
  readonly logRefused: boolean;
  readonly yTicks: readonly number[];
  readonly playheadX: number;
  readonly dateTabText: string;
  readonly dateTabX: number;
  readonly dateTabWidth: number;
  readonly rawAxisTicks: readonly TimeAxisTick[];
  readonly endLabels: readonly PlacedEndLabel[];
  readonly markers: readonly PlacedMarker[];
  readonly gaps: readonly PlacedGap[];
  readonly columns: readonly PlacedColumnItem[];
  readonly rows?: RowsFrame;
  readonly legendTop: number | null;
  readonly toX: (hour: number) => number;
  readonly toY: (value: number) => number;
  readonly fromX: (px: number) => number;
}

const TOTAL_LABEL = '__total__';
const END_LABEL_HEIGHT = 15;
const END_LABEL_GAP = 2;
const END_LABEL_MAX_NAME_PX = 150;
const FOLD_SLOT_WIDTH = 46;
const MARKER_MIN_ICON_GAP = 21;

function boundedExtent(bounded: BoundedSeries, xDomain: readonly [number, number], scale: number, headInView: boolean): readonly [number, number] | null {
  let low = Infinity;
  let high = -Infinity;
  const include = (v: number) => {
    const scaled = v * scale;
    if (!Number.isFinite(scaled)) return;
    if (scaled < low) low = scaled;
    if (scaled > high) high = scaled;
  };
  const runs = bounded.runs && bounded.runs.length > 0 ? bounded.runs : null;
  const history = closedExtent(bounded, scale);
  const fits = historyFits(history, { from: xDomain[0], to: xDomain[1] });
  if (fits) {
    if (history.low < low) low = history.low;
    if (history.high > high) high = history.high;
  }
  const from = fits ? history.items : 0;
  if (runs) {
    for (let i = from; i < runs.length; i++) {
      const run = runs.at(i)!;
      if (run.lastHour < xDomain[0] || run.firstHour > xDomain[1]) continue;
      include(run.value);
    }
  } else {
    for (let i = from; i < bounded.buckets.length; i++) {
      const bucket = bounded.buckets.at(i);
      if (!bucket || bucket.lastHour < xDomain[0] || bucket.firstHour > xDomain[1]) continue;
      include(bucket.minValue);
      include(bucket.maxValue);
    }
  }
  if (headInView && bounded.headValue && (bounded.headValue.kind === 'known' || bounded.headValue.kind === 'zero' || bounded.headValue.kind === 'partial')) {
    const num = Number(bounded.headValue.value);
    include(num);
  }
  return low <= high ? [low, high] : null;
}

function seriesExtent(
  series: TimeChartSeriesModel,
  xDomain: readonly [number, number],
  until: number,
): readonly [number, number] | null {
  const headHour = series.headHour ?? until;
  const headInView = headHour >= xDomain[0] && headHour <= xDomain[1];
  if (series.bounded) {
    const ext = boundedExtent(series.bounded, xDomain, series.valueScale ?? 1, headInView);
    if (ext) {
      if (headInView && series.headValue != null && Number.isFinite(series.headValue)) {
        return [Math.min(ext[0], series.headValue), Math.max(ext[1], series.headValue)];
      }
      return ext;
    }
  }
  if (!series.points?.length) {
    if (series.headValue != null && Number.isFinite(series.headValue) && headInView) {
      return [series.headValue, series.headValue];
    }
    return null;
  }
  const end = Math.min(xDomain[1], until);
  let low = Infinity;
  let high = -Infinity;
  for (const pt of series.points) {
    if (pt.hour > end) break;
    if (pt.hour < xDomain[0] || pt.value === null || !Number.isFinite(pt.value)) continue;
    for (const value of [pt.value, pt.min, pt.max]) {
      if (value == null || !Number.isFinite(value)) continue;
      if (value < low) low = value;
      if (value > high) high = value;
    }
  }
  return low <= high ? [low, high] : null;
}

function seriesEndPoint(
  series: TimeChartSeriesModel,
  until: number,
): { readonly hour: number; readonly value: number } | null {
  if (series.headValue != null && Number.isFinite(series.headValue)) {
    return { hour: Math.min(series.headHour ?? until, until), value: series.headValue };
  }
  const scale = series.valueScale ?? 1;
  if (series.bounded) {
    if (series.bounded.runs && series.bounded.runs.length > 0) {
      const last = series.bounded.runs.at(series.bounded.runs.length - 1);
      if (last && Number.isFinite(last.value)) return { hour: last.lastHour, value: last.value * scale };
    }
    for (let i = series.bounded.buckets.length - 1; i >= 0; i--) {
      const b = series.bounded.buckets.at(i);
      if (b && Number.isFinite(b.lastValue)) return { hour: b.lastHour, value: b.lastValue * scale };
    }
  }
  if (series.points?.length) {
    for (let i = series.points.length - 1; i >= 0; i--) {
      const pt = series.points[i]!;
      if (pt.hour <= until && pt.value !== null && Number.isFinite(pt.value)) {
        return { hour: pt.hour, value: pt.value };
      }
    }
  }
  return null;
}

function buildColumnsLayout(
  columns: ChartColumnModel,
  xDomain: readonly [number, number],
  until: number,
  left: number,
  plotWidth: number,
  small: boolean,
  measure: MeasureText,
  smallFont: string,
): PlacedColumnItem[] {
  const cutoff = Math.min(xDomain[1], until);
  const visible = columns.months.filter(m => m.lastHour >= xDomain[0] && m.firstHour <= cutoff);
  if (!visible.length) return [];
  const threshold = columns.foldThreshold ?? 6;
  const groups: ({ kind: 'fold'; months: ChartMonthSlot[] } | { kind: 'month'; month: ChartMonthSlot })[] = [];
  let run: ChartMonthSlot[] = [];
  const flushRun = () => {
    if (!run.length) return;
    if (columns.foldUnrecorded && run.length >= threshold) {
      groups.push({ kind: 'fold', months: run });
    } else {
      for (const m of run) groups.push({ kind: 'month', month: m });
    }
    run = [];
  };
  for (const m of visible) {
    if (m.status === 'recorded') {
      flushRun();
      groups.push({ kind: 'month', month: m });
    } else {
      run.push(m);
    }
  }
  flushRun();

  const foldCount = groups.filter(g => g.kind === 'fold').length;
  const monthCount = groups.length - foldCount;
  const foldWidth = Math.min(FOLD_SLOT_WIDTH, Math.max(28, plotWidth * 0.25 / Math.max(1, foldCount)));
  const slotWidth = Math.max(4, (plotWidth - foldCount * foldWidth) / Math.max(1, monthCount));
  const out: PlacedColumnItem[] = [];
  let x = left;
  let lastLabelRight = -1e9;

  for (let idx = 0; idx < groups.length; idx++) {
    const g = groups[idx]!;
    if (g.kind === 'fold') {
      const a = g.months[0]!;
      const b = g.months[g.months.length - 1]!;
      const shortEnd = String(b.year).slice(-2);
      const foldLabel = a.year === b.year ? String(a.year) : `${a.year}\u2013${shortEnd}`;
      const foldSpanText = `${shortMonth(a.month)} ${a.year} \u2013 ${shortMonth(b.month)} ${b.year}`;
      const cx = x + foldWidth / 2;
      const lw = measure(foldLabel, smallFont);
      const shownLabel = cx - lw / 2 >= lastLabelRight + 6 ? foldLabel : null;
      if (shownLabel) lastLabelRight = cx + lw / 2;
      out.push({
        kind: 'fold',
        index: idx,
        x,
        width: foldWidth,
        barX: x + 4,
        barWidth: Math.max(4, foldWidth - 8),
        label: shownLabel,
        majorLabel: true,
        foldMonths: g.months,
        foldLabel,
        foldSpanText,
      });
      x += foldWidth;
      continue;
    }
    const m = g.month;
    const prevFold = idx > 0 && groups[idx - 1]!.kind === 'fold';
    const isTerminalAtStart = idx === groups.length - 1 && m.firstHour === until && m.status === 'recorded';
    const barW = Math.max(isTerminalAtStart ? 2 : 1, Math.min(slotWidth * 0.62, 26));
    const barX = x + (slotWidth - barW) / 2;
    let label: string | null = null;
    if (m.month === 1) label = String(m.year);
    else if (prevFold) label = `${shortMonth(m.month)} ${m.year}`;
    else if (m.month === 4 || m.month === 7 || m.month === 10 || m.status === 'recorded') {
      label = shortMonth(m.month);
    }
    if (small && idx > 0 && idx < groups.length - 1 && m.month !== 1 && !prevFold) {
      label = null;
    }
    if (label) {
      const cx = x + slotWidth / 2;
      const lw = measure(label, smallFont);
      if (cx - lw / 2 < lastLabelRight + 8) {
        label = null;
      } else {
        lastLabelRight = cx + lw / 2;
      }
    }
    out.push({
      kind: 'month',
      index: idx,
      x,
      width: slotWidth,
      barX,
      barWidth: barW,
      label,
      majorLabel: m.month === 1,
      month: m,
    });
    x += slotWidth;
  }
  return out;
}

function rowsChartLayout(model: TimeChartModel, width: number, height: number, small: boolean, fontFamily: string, measure: MeasureText): TimeChartLayout {
  const frame = layoutChartRows(model, width, height, small, measure, fontFamily);
  const span = Math.max(1, frame.xDomain[1] - frame.xDomain[0]);
  return {
    width, height, small, left: frame.left, right: width - frame.left - frame.plotWidth, top: frame.top, bottom: height - frame.axisY,
    plotWidth: frame.plotWidth, plotHeight: frame.axisY - frame.top, baseY: frame.axisY, markerLaneHeight: 0, axisY: frame.axisY,
    xDomain: frame.xDomain, yDomain: [0, 1], yTicks: [], playheadX: frame.toX(model.hour + 1), dateTabText: '', dateTabX: 0, dateTabWidth: 0,
    rawAxisTicks: frame.axisTicks, endLabels: [], markers: [], gaps: [], columns: [], rows: frame, legendTop: null,
    logShown: false, logRefused: false, toX: frame.toX, toY: () => frame.axisY, fromX: px => frame.xDomain[0] + ((px - frame.left) / frame.plotWidth) * span,
  };
}

export function layoutTimeChart(
  model: TimeChartModel,
  width: number,
  height: number,
  fontFamily: string,
  measure: MeasureText,
): TimeChartLayout {
  const narrow = width <= 490;
  const small = narrow || height <= 200;
  if (model.mode === 'rows') return rowsChartLayout(model, width, height, small, fontFamily, measure);
  const smallFont = `11px ${fontFamily}`;
  const tabFont = `600 11px ${fontFamily}`;
  const labelNameFont = `500 12px ${fontFamily}`;
  const labelValueFont = `600 12px ${fontFamily}`;

  const rawStart = model.view?.x?.[0] ?? model.extent[0];
  const rawEnd = Math.min(model.view?.x?.[1] ?? model.hour, model.hour);
  const xStart = Math.min(rawStart, rawEnd);
  const xEnd = Math.max(xStart + 1, rawEnd);
  const xDomain: readonly [number, number] = [xStart, xEnd];

  let dataMin = Infinity;
  let dataMax = -Infinity;
  if (model.mode === 'columns' && model.columns) {
    const cutoff = Math.min(xDomain[1], model.hour);
    for (const m of model.columns.months) {
      if (m.lastHour < xDomain[0] || m.firstHour > cutoff || m.status !== 'recorded') continue;
      let posSum = 0;
      let negSum = 0;
      for (const seg of m.segments) {
        if (seg.value > 0) posSum += seg.value;
        else if (seg.value < 0) negSum += seg.value;
      }
      const lo = Math.min(0, negSum, m.totalValue);
      const hi = Math.max(0, posSum, m.totalValue);
      if (lo < dataMin) dataMin = lo;
      if (hi > dataMax) dataMax = hi;
    }
  } else if (model.mode === 'stack' && model.stack) {
    const { hours, bands, total } = model.stack;
    for (let i = 0; i < hours.length; i++) {
      const h = hours[i]!;
      if (h > model.hour) break;
      if (h < xDomain[0]) continue;
      let posSum = 0;
      let negSum = 0;
      for (const band of bands) {
        const v = band.values[i];
        if (v === null || v === undefined || !Number.isFinite(v)) continue;
        if (v >= 0) posSum += v;
        else negSum += v;
      }
      const tv = total?.values[i];
      const hasTotal = tv !== null && tv !== undefined && Number.isFinite(tv);
      const lo = hasTotal ? Math.min(0, negSum, tv) : Math.min(0, negSum);
      const hi = hasTotal ? Math.max(0, posSum, tv) : Math.max(0, posSum);
      if (lo < dataMin) dataMin = lo;
      if (hi > dataMax) dataMax = hi;
    }
  } else {
    for (const s of model.series) {
      const ext = seriesExtent(s, xDomain, model.hour);
      if (!ext) continue;
      if (ext[0] < dataMin) dataMin = ext[0];
      if (ext[1] > dataMax) dataMax = ext[1];
    }
  }

  let yLo: number;
  let yHi: number;
  const wholeAxis = model.yAxis.format === 'number' && model.yAxis.decimals === 0;
  const bounded = model.yAxis.min != null || model.yAxis.max != null;
  const fullPercent = !model.view?.y && !model.yAxis.fixed && !bounded && (model.yAxis.format === 'percent' || model.yAxis.format === 'share') && model.yAxis.fixed !== null;
  const logAsked = model.yAxis.log === true && !model.view?.y && !model.yAxis.fixed;
  const logShown = logAsked && dataMin > 0 && dataMax >= dataMin;
  if (model.view?.y) {
    yLo = model.view.y[0];
    yHi = model.view.y[1];
  } else if (model.yAxis.fixed) {
    yLo = model.yAxis.fixed[0];
    yHi = model.yAxis.fixed[1];
  } else if (fullPercent && !logShown) {
    yLo = 0;
    yHi = dataMin <= dataMax && dataMax > 100 ? Math.ceil(dataMax / 25) * 25 : 100;
  } else if (logShown) {
    yLo = model.yAxis.min != null && model.yAxis.min > 0 ? model.yAxis.min : dataMin;
    yHi = model.yAxis.max != null && model.yAxis.max > yLo ? model.yAxis.max : Math.max(dataMax * 1.08, yLo * 10);
  } else if (dataMin <= dataMax) {
    const autoLo = model.yAxis.min != null ? model.yAxis.min : Math.min(0, dataMin);
    const span = Math.abs(dataMax - Math.min(0, autoLo));
    const padTop = Math.max(span * 0.08, span === 0 ? 1 : 1e-6);
    const autoHi = model.yAxis.max != null ? model.yAxis.max : Math.max(0, dataMax) + padTop;
    const negPad = autoLo < 0 ? Math.max(Math.abs(autoLo) * 0.12, 1) : 0;
    yLo = model.yAxis.min != null ? model.yAxis.min : autoLo - negPad;
    yHi = autoHi > yLo ? autoHi : yLo + 1;
  } else {
    yLo = model.yAxis.min ?? 0;
    yHi = model.yAxis.max ?? 1;
  }
  const yDomain: readonly [number, number] = [yLo, yHi];
  const decadeHeight = Math.max(1, height - 44) / Math.max(1e-9, Math.log10(yHi) - Math.log10(yLo));
  const yTicks = model.yAxis.ticks
    ? [...model.yAxis.ticks]
    : logShown
      ? thinLogTicks(wholeAxis ? logTicks(Math.max(1, yLo), yHi).filter(Number.isInteger) : logTicks(yLo, yHi), value => Math.log10(value) * decadeHeight, 18)
      : model.yAxis.fixed || fullPercent
      ? linearTicks(yLo, yHi, small ? 2 : 4, wholeAxis)
      : linearTicks(yLo, yHi, small ? 3 : 5, wholeAxis);

  let maxYLabelW = 0;
  for (const t of yTicks) {
    maxYLabelW = Math.max(maxYLabelW, measure(formatChartAxisValue(t, model.yAxis.format, yTicks), smallFont));
  }
  const left = Math.ceil(Math.max(30, maxYLabelW + 16));

  const hasLegend = !!(model.legend && model.legend.length > 0 && (!small || model.mode !== 'columns'));
  const top = hasLegend ? 32 : 10;
  const hasMarkers = !!(model.markers && model.markers.length > 0 && model.mode !== 'columns');
  const markerLaneHeight = hasMarkers ? (small ? 12 : 20) : 0;
  const axisH = 24;
  const bottom = markerLaneHeight + axisH;

  let right = model.mode === 'columns' ? 16 : 24;
  const rawEndLabels: {
    id: string;
    seriesId: string;
    anchorVal: number;
    color?: string;
    faded: boolean;
    colorBar?: string;
    flag?: TimeChartSeriesModel['flag'];
    icon?: TimeChartSeriesModel['icon'];
    name: string | null;
    fullName: string;
    value: string;
    valueColor?: string;
    valueMuted?: boolean;
    negative?: boolean;
    fixedWidth: number;
    estWidth: number;
  }[] = [];

  const measureName = (text: string) => measure(text, labelNameFont);

  const playheadInView = xDomain[1] >= model.hour;
  if (model.mode === 'lines' && playheadInView) {
    for (const s of model.series) {
      if (!s.endLabel) continue;
      const end = seriesEndPoint(s, model.hour);
      if (!end || Math.abs(end.hour - model.hour) > 48) continue;
      const rawName = small && s.endLabel.shortName !== undefined
        ? s.endLabel.shortName
        : s.endLabel.name;
      const showName = !!(rawName && (!small || s.endLabel.shortName !== undefined || (!s.endLabel.flag && !s.endLabel.icon)));
      const nameText = showName ? ellipsise(rawName!, END_LABEL_MAX_NAME_PX, measureName) : null;
      const artW = s.endLabel.flag && s.endLabel.flag.art.status !== 'noArtExpected'
        ? 24
        : s.endLabel.icon && s.endLabel.icon.art.status !== 'noArtExpected'
          ? 22
          : 0;
      const nameW = nameText ? measure(nameText, labelNameFont) + 6 : 0;
      const valW = measure(s.endLabel.value, labelValueFont);
      const fixedWidth = artW + valW;
      const estWidth = fixedWidth + nameW + 8;
      const isNegative = s.negRed && end.value < 0;
      rawEndLabels.push({
        id: s.id,
        seriesId: s.id,
        anchorVal: end.value,
        color: s.color,
        faded: false,
        flag: s.endLabel.flag,
        icon: s.endLabel.icon,
        name: nameText,
        fullName: s.endLabel.fullName ?? s.name,
        value: s.endLabel.value,
        valueColor: s.endLabel.valueColor,
        valueMuted: s.endLabel.valueMuted,
        negative: isNegative,
        fixedWidth,
        estWidth,
      });
    }
  } else if (model.mode === 'stack' && playheadInView && model.stack && model.stack.hours.length > 0) {
    let lastIdx = -1;
    for (let i = model.stack.hours.length - 1; i >= 0; i--) {
      if (model.stack.hours[i]! <= model.hour) { lastIdx = i; break; }
    }
    const estPlotHeight = Math.max(1, height - top - bottom);
    const ySpan = Math.max(1e-9, yDomain[1] - yDomain[0]);
    const trailingGap = model.stack.step === true && lastIdx >= 0
      && (model.gaps?.some(g => g.firstHour > model.stack!.hours[lastIdx]! && g.firstHour <= model.hour) ?? false);
    const lastTotalVal = model.stack.total?.values[lastIdx];
    const hasTotalAtLast = lastTotalVal !== null && lastTotalVal !== undefined && Number.isFinite(lastTotalVal);
    const hasAnyBandAtLast = model.stack.bands.some(band => {
      const v = band.values[lastIdx];
      return v !== null && v !== undefined && Number.isFinite(v);
    });
    const lastValid = lastIdx >= 0
      && !trailingGap
      && (model.stack.step === true || Math.abs(model.hour - model.stack.hours[lastIdx]!) <= 48)
      && (hasTotalAtLast || hasAnyBandAtLast);
    if (lastValid) {
      let posAcc = 0;
      let negAcc = 0;
      for (const band of model.stack.bands) {
        const rawV = band.values[lastIdx];
        if (rawV === null || rawV === undefined || !Number.isFinite(rawV)) continue;
        const mid = rawV >= 0 ? posAcc + rawV / 2 : negAcc + rawV / 2;
        if (rawV >= 0) posAcc += rawV;
        else negAcc += rawV;
        if (!band.endLabel) continue;
        const bandPx = (Math.abs(rawV) / ySpan) * estPlotHeight;
        if (small && bandPx < 10) continue;
        const rawName = small && band.endLabel.shortName !== undefined
          ? band.endLabel.shortName
          : band.endLabel.name;
        const showName = !!(rawName && (!narrow || !band.endLabel.flag));
        const nameText = showName ? ellipsise(rawName!, END_LABEL_MAX_NAME_PX, measureName) : null;
        const barW = 9;
        const artW = band.endLabel.flag && band.endLabel.flag.art.status !== 'noArtExpected' ? 24 : 0;
        const nameW = nameText ? measure(nameText, labelNameFont) + 6 : 0;
        const valW = measure(band.endLabel.value, labelValueFont);
        const fixedWidth = barW + artW + valW;
        rawEndLabels.push({
          id: band.id,
          seriesId: band.id,
          anchorVal: mid,
          color: band.color,
          faded: false,
          colorBar: band.color,
          flag: band.endLabel.flag,
          name: nameText,
          fullName: band.endLabel.fullName ?? band.name,
          value: band.endLabel.value,
          valueColor: band.endLabel.valueColor,
          fixedWidth,
          estWidth: fixedWidth + nameW + 8,
        });
      }
      if (model.stack.total?.endLabel && hasTotalAtLast) {
        const tv = lastTotalVal!;
        const el = model.stack.total.endLabel;
        const artW = el.icon && el.icon.art.status !== 'noArtExpected' ? 22 : 0;
        const valW = measure(el.value, labelValueFont);
        const fixedWidth = artW + valW;
        rawEndLabels.push({
          id: TOTAL_LABEL,
          seriesId: TOTAL_LABEL,
          anchorVal: tv,
          color: model.stack.total.color,
          faded: false,
          icon: el.icon,
          name: null,
          fullName: model.measureTitle,
          value: el.value,
          valueColor: el.valueColor,
          fixedWidth,
          estWidth: fixedWidth + 8,
        });
      }
    }
  }

  if (rawEndLabels.length > 0) {
    let widest = 0;
    for (const l of rawEndLabels) widest = Math.max(widest, l.estWidth);
    const maxRight = Math.max(76, Math.floor(width * (narrow ? 0.36 : 0.32)));
    right = Math.min(maxRight, Math.max(56, Math.ceil(widest + 16)));
  }

  const plotWidth = Math.max(1, width - left - right);
  const plotHeight = Math.max(1, height - top - bottom);
  const baseY = top + plotHeight;
  const axisY = baseY + markerLaneHeight;

  const toX = (h: number) => left + ((h - xDomain[0]) / (xDomain[1] - xDomain[0])) * plotWidth;
  const fromX = (px: number) => xDomain[0] + ((px - left) / plotWidth) * (xDomain[1] - xDomain[0]);
  const toY = (v: number) => {
    if (logShown && v > 0) {
      const l0 = Math.log10(yDomain[0]);
      const l1 = Math.log10(yDomain[1]);
      return top + (1 - (Math.log10(v) - l0) / (l1 - l0)) * plotHeight;
    }
    return top + (1 - (v - yDomain[0]) / (yDomain[1] - yDomain[0])) * plotHeight;
  };

  const columns = model.mode === 'columns' && model.columns
    ? buildColumnsLayout(model.columns, xDomain, model.hour, left, plotWidth, small, measure, smallFont)
    : [];

  const playheadX = model.mode === 'columns' && columns.length > 0
    ? columns[columns.length - 1]!.x + columns[columns.length - 1]!.width / 2
    : Math.max(left, Math.min(left + plotWidth, toX(model.hour)));

  const endLabels: PlacedEndLabel[] = [];
  if (rawEndLabels.length > 0) {
    const anchors = rawEndLabels.map(l => ({ id: l.id, y: toY(l.anchorVal), height: END_LABEL_HEIGHT }));
    const placedY = layoutEndLabels(anchors, Math.max(2, top - END_LABEL_HEIGHT / 2), baseY + 4, END_LABEL_GAP);
    const lx = playheadX + 10;
    const availWidth = Math.max(0, width - lx - 4);
    const minEllipsis = measure('\u2026', labelNameFont);
    for (const l of rawEndLabels) {
      const anchorY = toY(l.anchorVal);
      const ly = placedY.get(l.id) ?? anchorY;
      let fittedName = l.name;
      if (fittedName) {
        const nameBudget = availWidth - l.fixedWidth - 6;
        if (nameBudget < minEllipsis) {
          fittedName = null;
        } else if (measureName(fittedName) > nameBudget) {
          fittedName = ellipsise(fittedName, nameBudget, measureName);
        }
      }
      const fittedWidth = l.fixedWidth + (fittedName ? measureName(fittedName) + 6 : 0) + 8;
      endLabels.push({
        id: l.id,
        seriesId: l.seriesId,
        anchorY,
        y: ly,
        x: lx,
        width: Math.min(fittedWidth, availWidth),
        height: 16,
        color: l.color,
        faded: l.faded,
        colorBar: l.colorBar,
        flag: l.flag,
        icon: l.icon,
        name: fittedName,
        fullName: l.fullName,
        value: l.value,
        valueColor: l.valueColor,
        valueMuted: l.valueMuted,
        negative: l.negative,
        hasLeader: Math.abs(ly - anchorY) > 2,
      });
    }
    endLabels.sort((a, b) => a.y - b.y);
  }

  const placedMarkers: PlacedMarker[] = [];
  if (hasMarkers && model.markers) {
    const sorted = model.markers
      .filter(m => m.hour >= xDomain[0] && m.hour <= model.hour && m.hour <= xDomain[1])
      .sort((a, b) => a.hour - b.hour);
    const ly = baseY + markerLaneHeight / 2 + 1;
    let lastIconX = -1e9;
    for (let i = 0; i < sorted.length; i++) {
      const m = sorted[i]!;
      const x = toX(m.hour);
      const hasFlag = !!(m.flag && m.flag.art.status !== 'noArtExpected');
      const hasIcon = !!(m.icon && m.icon.art.status !== 'noArtExpected');
      if ((hasFlag || hasIcon) && markerLaneHeight >= 20 && x - lastIconX >= MARKER_MIN_ICON_GAP) {
        placedMarkers.push({ index: i, marker: m, x, y: ly, kind: hasFlag ? 'flag' : 'icon', size: 18 });
        lastIconX = x;
      } else {
        placedMarkers.push({ index: i, marker: m, x, y: ly, kind: 'tick', size: markerLaneHeight >= 20 ? 8 : 6 });
      }
    }
  }

  const placedGaps: PlacedGap[] = [];
  if (model.gaps && model.mode !== 'columns') {
    for (let i = 0; i < model.gaps.length; i++) {
      const gap = model.gaps[i]!;
      const ga = Math.max(xDomain[0], gap.firstHour);
      const gb = Math.min(xDomain[1], model.hour, gap.lastHour);
      if (gb < ga) continue;
      const rawX0 = toX(ga);
      const rawX1 = toX(gb);
      let x0 = rawX0;
      let x1 = Math.max(rawX0 + 2, rawX1);
      if (x1 > left + plotWidth) {
        x1 = left + plotWidth;
        x0 = Math.max(left, Math.min(rawX0, x1 - 2));
      }
      placedGaps.push({ index: i, gap, ongoing: gap.lastHour > model.hour, x0, x1 });
    }
  }

  const dateTabText = formatDateOnly(model.hour);
  const dateTabWidth = Math.ceil(measure(dateTabText, tabFont) + 14);
  const dateTabX = Math.max(left + dateTabWidth / 2, Math.min(playheadX, width - 6 - dateTabWidth / 2));

  const rawAxisTicks: TimeAxisTick[] = model.mode !== 'columns' ? computeTimeAxisTicks(xDomain[0], xDomain[1], plotWidth) : [];

  return {
    width,
    height,
    small,
    left,
    right,
    top,
    bottom,
    plotWidth,
    plotHeight,
    baseY,
    markerLaneHeight,
    axisY,
    xDomain,
    yDomain,
    logShown,
    logRefused: logAsked && !logShown,
    yTicks,
    playheadX,
    dateTabText,
    dateTabX,
    dateTabWidth,
    rawAxisTicks,
    endLabels,
    markers: placedMarkers,
    gaps: placedGaps,
    columns,
    legendTop: hasLegend ? 8 : null,
    toX,
    toY,
    fromX,
  };
}
