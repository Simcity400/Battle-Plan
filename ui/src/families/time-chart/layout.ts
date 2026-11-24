import { ellipsise, layoutEndLabels } from '../../charts/labels.ts';
import type { MeasureText } from '../../charts/textWidth.ts';
import { engineHour, gameDate } from '../../dashboard/calendar.ts';
import type { BoundedSeries } from '../plan.ts';
import { formatCompactScalar, formatDateOnly } from '../valueFormat.ts';
import type {
  ChartColumnModel,
  ChartGap,
  ChartMarker,
  ChartMonthSlot,
  ChartValueFormat,
  TimeChartModel,
  TimeChartSeriesModel,
} from './model.ts';

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] as const;

export function formatChartAxisValue(value: number, format: ChartValueFormat): string {
  if (!Number.isFinite(value)) return '';
  if (format === 'percent') return `${Math.round(value)}%`;
  if (format === 'signed1') {
    if (Math.abs(value) < 1e-9) return '0';
    const text = Math.abs(value).toFixed(1).replace(/\.0$/, '');
    return `${value < 0 ? '\u2212' : '+'}${text}`;
  }
  if (format === 'compact') {
    if (value === 0) return '0';
    return formatCompactScalar(Math.round(value), 'integer').text;
  }
  if (format === 'power') {
    const abs = Math.abs(value);
    if (abs >= 1e4) return formatCompactScalar(Math.round(value), 'integer').text;
    const sign = value < 0 ? '\u2212' : '';
    return `${sign}${Math.round(abs).toLocaleString('en-US')}`;
  }
  const sign = value < 0 ? '\u2212' : '';
  return `${sign}${Math.round(Math.abs(value)).toLocaleString('en-US')}`;
}

function ticksForStep(lo: number, hi: number, step: number): number[] {
  const start = Math.floor(lo / step) * step;
  const ticks: number[] = [];
  for (let v = start; v <= hi + step * 1e-6; v += step) {
    if (v >= lo - step * 1e-6) {
      const clean = Math.abs(v) < step * 1e-9 ? 0 : Math.round(v * 1e6) / 1e6;
      ticks.push(clean);
    }
    if (ticks.length > 20) break;
  }
  return ticks;
}

export function niceChartTicks(lo: number, hi: number, intervals: number): number[] {
  const span = hi - lo;
  if (!(span > 0) || !Number.isFinite(span)) return [lo];
  const target = Math.max(2, intervals + 1);
  const raw = span / Math.max(1, intervals);
  const mag = 10 ** Math.floor(Math.log10(raw));
  let best: number[] | null = null;
  let bestScore = Infinity;
  for (const mult of [0.5, 1, 2, 2.5, 5, 10]) {
    const step = mult * mag;
    if (!(step > 0)) continue;
    const candidate = ticksForStep(lo, hi, step);
    if (candidate.length < 2) continue;
    const diff = Math.abs(candidate.length - target);
    const penalty = candidate.length > target + 2 ? 4 : candidate.length < target - 1 ? 2 : 0;
    const score = diff + penalty;
    if (score < bestScore) {
      bestScore = score;
      best = candidate;
    }
  }
  return best ?? [lo, hi];
}

function monthStartHour(year: number, month: number): number {
  return engineHour({ year, month, day: 1, hour: 1 }) ?? 0;
}

function yearStartHour(year: number): number {
  return engineHour({ year, month: 1, day: 1, hour: 1 }) ?? 0;
}

export interface PlacedEndLabel {
  readonly id: string;
  readonly seriesId: string;
  readonly anchorY: number;
  readonly y: number;
  readonly x: number;
  readonly width: number;
  readonly height: number;
  readonly color: string;
  readonly faded: boolean;
  readonly colorBar?: string;
  readonly flag?: TimeChartSeriesModel['flag'];
  readonly icon?: TimeChartSeriesModel['icon'];
  readonly name: string | null;
  readonly fullName: string;
  readonly value: string;
  readonly valueColor?: string;
  readonly valueMuted?: boolean;
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

export interface PlacedAxisTick {
  readonly hour: number;
  readonly x: number;
  readonly label: string | null;
  readonly major: boolean;
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
  readonly yTicks: readonly number[];
  readonly playheadX: number;
  readonly dateTabText: string;
  readonly dateTabX: number;
  readonly dateTabWidth: number;
  readonly axisTicks: readonly PlacedAxisTick[];
  readonly endLabels: readonly PlacedEndLabel[];
  readonly markers: readonly PlacedMarker[];
  readonly gaps: readonly PlacedGap[];
  readonly columns: readonly PlacedColumnItem[];
  readonly legendTop: number | null;
  readonly toX: (hour: number) => number;
  readonly toY: (value: number) => number;
  readonly fromX: (px: number) => number;
}

const END_LABEL_HEIGHT = 15;
const END_LABEL_GAP = 2;
const END_LABEL_MAX_NAME_PX = 150;
const FOLD_SLOT_WIDTH = 46;
const MARKER_MIN_ICON_GAP = 21;

function boundedExtent(bounded: BoundedSeries, xDomain: readonly [number, number], scale = 1): readonly [number, number] | null {
  let low = Infinity;
  let high = -Infinity;
  const include = (v: number) => {
    const scaled = v * scale;
    if (!Number.isFinite(scaled)) return;
    if (scaled < low) low = scaled;
    if (scaled > high) high = scaled;
  };
  if (bounded.runs && bounded.runs.length > 0) {
    for (const run of bounded.runs) {
      if (run.lastHour < xDomain[0] || run.firstHour > xDomain[1]) continue;
      include(run.value);
    }
  } else {
    for (const bucket of bounded.buckets) {
      if (!bucket || bucket.lastHour < xDomain[0] || bucket.firstHour > xDomain[1]) continue;
      include(bucket.minValue);
      include(bucket.maxValue);
    }
  }
  if (bounded.headValue && (bounded.headValue.kind === 'known' || bounded.headValue.kind === 'zero' || bounded.headValue.kind === 'partial')) {
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
  if (series.bounded) {
    const ext = boundedExtent(series.bounded, xDomain, series.valueScale ?? 1);
    if (ext) {
      if (series.headValue != null && Number.isFinite(series.headValue)) {
        return [Math.min(ext[0], series.headValue), Math.max(ext[1], series.headValue)];
      }
      return ext;
    }
  }
  if (!series.points?.length) {
    if (series.headValue != null && Number.isFinite(series.headValue) && until >= xDomain[0] && until <= xDomain[1]) {
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
    if (pt.value < low) low = pt.value;
    if (pt.value > high) high = pt.value;
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
  until: number,
  left: number,
  plotWidth: number,
  small: boolean,
): PlacedColumnItem[] {
  const visible = columns.months.filter(m => m.firstHour <= until);
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

  for (let idx = 0; idx < groups.length; idx++) {
    const g = groups[idx]!;
    if (g.kind === 'fold') {
      const a = g.months[0]!;
      const b = g.months[g.months.length - 1]!;
      const shortEnd = String(b.year).slice(-2);
      const foldLabel = a.year === b.year ? String(a.year) : `${a.year}\u2013${shortEnd}`;
      const foldSpanText = `${MONTHS[a.month - 1]} ${a.year} \u2013 ${MONTHS[b.month - 1]} ${b.year}`;
      out.push({
        kind: 'fold',
        index: idx,
        x,
        width: foldWidth,
        barX: x + 4,
        barWidth: Math.max(4, foldWidth - 8),
        label: foldLabel,
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
    else if (prevFold) label = `${MONTHS[m.month - 1]} ${m.year}`;
    else if (m.month === 4 || m.month === 7 || m.month === 10 || m.status === 'recorded') {
      label = MONTHS[m.month - 1]!;
    }
    if (small && idx > 0 && idx < groups.length - 1 && m.month !== 1 && !prevFold) {
      label = null;
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

export function layoutTimeChart(
  model: TimeChartModel,
  width: number,
  height: number,
  fontFamily: string,
  measure: MeasureText,
  hoverHour?: number | null,
  hoveredSeriesId?: string | null,
): TimeChartLayout {
  const small = width <= 490 || height <= 200;
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
    for (const m of model.columns.months) {
      if (m.firstHour > model.hour || m.status !== 'recorded') continue;
      if (m.totalValue < dataMin) dataMin = m.totalValue;
      if (m.totalValue > dataMax) dataMax = m.totalValue;
      if (0 < dataMin) dataMin = 0;
    }
  } else if (model.mode === 'stack' && model.stack) {
    const { hours, bands, total } = model.stack;
    for (let i = 0; i < hours.length; i++) {
      const h = hours[i]!;
      if (h > model.hour) break;
      if (h < xDomain[0]) continue;
      let sum = 0;
      for (const band of bands) {
        const v = band.values[i];
        if (v !== null && v !== undefined && Number.isFinite(v)) sum += v;
      }
      const tv = total?.values[i];
      const top = tv !== null && tv !== undefined && Number.isFinite(tv) ? Math.max(sum, tv) : sum;
      if (0 < dataMin) dataMin = 0;
      if (top > dataMax) dataMax = top;
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
  if (model.view?.y) {
    yLo = model.view.y[0];
    yHi = model.view.y[1];
  } else if (model.yAxis.fixed) {
    yLo = model.yAxis.fixed[0];
    yHi = model.yAxis.fixed[1];
  } else if (model.yAxis.format === 'percent' && model.yAxis.fixed !== null) {
    yLo = 0;
    yHi = 100;
  } else if (dataMin <= dataMax) {
    const autoLo = model.yAxis.min != null ? model.yAxis.min : Math.min(0, dataMin);
    const padTop = Math.max(Math.abs(dataMax - Math.min(0, autoLo)) * 0.08, 1e-6);
    const autoHi = model.yAxis.max != null ? model.yAxis.max : Math.max(0, dataMax) + padTop;
    const negPad = autoLo < 0 ? Math.max(Math.abs(autoLo) * 0.12, 1) : 0;
    yLo = model.yAxis.min != null ? model.yAxis.min : autoLo - negPad;
    yHi = autoHi > yLo ? autoHi : yLo + 1;
  } else {
    yLo = model.yAxis.min ?? 0;
    yHi = model.yAxis.max ?? 1;
  }
  const yDomain: readonly [number, number] = [yLo, yHi];
  const yTicks = model.yAxis.ticks
    ? [...model.yAxis.ticks]
    : model.yAxis.format === 'percent' && yLo === 0 && yHi === 100
      ? (small ? [0, 50, 100] : [0, 25, 50, 75, 100])
      : niceChartTicks(yLo, yHi, small ? 2 : 4);

  let maxYLabelW = 0;
  for (const t of yTicks) {
    maxYLabelW = Math.max(maxYLabelW, measure(formatChartAxisValue(t, model.yAxis.format), smallFont));
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
    color: string;
    faded: boolean;
    colorBar?: string;
    flag?: TimeChartSeriesModel['flag'];
    icon?: TimeChartSeriesModel['icon'];
    name: string | null;
    fullName: string;
    value: string;
    valueColor?: string;
    valueMuted?: boolean;
    estWidth: number;
  }[] = [];

  const measureName = (text: string) => measure(text, labelNameFont);

  if (model.mode === 'lines') {
    for (const s of model.series) {
      if (!s.endLabel) continue;
      const end = seriesEndPoint(s, model.hour);
      if (!end || Math.abs(end.hour - model.hour) > 48) continue;
      const showName = !!(s.endLabel.name && (!small || (!s.endLabel.flag && !s.endLabel.icon)));
      const nameText = showName ? ellipsise(s.endLabel.name!, END_LABEL_MAX_NAME_PX, measureName) : null;
      const artW = s.endLabel.flag && s.endLabel.flag.art.status !== 'noArtExpected'
        ? 24
        : s.endLabel.icon && s.endLabel.icon.art.status !== 'noArtExpected'
          ? 22
          : 0;
      const nameW = nameText ? measure(nameText, labelNameFont) + 6 : 0;
      const valW = measure(s.endLabel.value, labelValueFont);
      const estWidth = artW + nameW + valW + 8;
      const isNegative = s.negRed && end.value < 0;
      rawEndLabels.push({
        id: s.id,
        seriesId: s.id,
        anchorVal: end.value,
        color: isNegative ? '#ff6b6f' : s.color,
        faded: hoveredSeriesId != null && hoveredSeriesId !== s.id,
        flag: s.endLabel.flag,
        icon: s.endLabel.icon,
        name: nameText,
        fullName: s.endLabel.fullName ?? s.name,
        value: s.endLabel.value,
        valueColor: isNegative ? '#ff6b6f' : s.endLabel.valueColor,
        valueMuted: s.endLabel.valueMuted,
        estWidth,
      });
    }
  } else if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    let lastIdx = -1;
    for (let i = model.stack.hours.length - 1; i >= 0; i--) {
      if (model.stack.hours[i]! <= model.hour) { lastIdx = i; break; }
    }
    if (lastIdx >= 0) {
      let acc = 0;
      for (const band of model.stack.bands) {
        const v = band.values[lastIdx] ?? 0;
        const mid = acc + v / 2;
        acc += v;
        if (!band.endLabel) continue;
        const showName = !!(band.endLabel.name && (!small || !band.endLabel.flag));
        const nameText = showName ? ellipsise(band.endLabel.name!, END_LABEL_MAX_NAME_PX, measureName) : null;
        const barW = 9;
        const artW = band.endLabel.flag && band.endLabel.flag.art.status !== 'noArtExpected' ? 24 : 0;
        const nameW = nameText ? measure(nameText, labelNameFont) + 6 : 0;
        const valW = measure(band.endLabel.value, labelValueFont);
        rawEndLabels.push({
          id: band.id,
          seriesId: band.id,
          anchorVal: mid,
          color: band.color,
          faded: hoveredSeriesId != null && hoveredSeriesId !== band.id,
          colorBar: band.color,
          flag: band.endLabel.flag,
          name: nameText,
          fullName: band.endLabel.fullName ?? band.name,
          value: band.endLabel.value,
          valueColor: band.endLabel.valueColor,
          estWidth: barW + artW + nameW + valW + 8,
        });
      }
      if (model.stack.total?.endLabel) {
        const tv = model.stack.total.values[lastIdx] ?? acc;
        const el = model.stack.total.endLabel;
        const artW = el.icon && el.icon.art.status !== 'noArtExpected' ? 22 : 0;
        const valW = measure(el.value, labelValueFont);
        rawEndLabels.push({
          id: '__total__',
          seriesId: '__total__',
          anchorVal: tv,
          color: model.stack.total.color ?? '#f5f5f5',
          faded: false,
          icon: el.icon,
          name: null,
          fullName: model.measureTitle,
          value: el.value,
          valueColor: el.valueColor,
          estWidth: artW + valW + 8,
        });
      }
    }
  }

  if (rawEndLabels.length > 0) {
    let widest = 0;
    for (const l of rawEndLabels) widest = Math.max(widest, l.estWidth);
    const maxRight = Math.max(76, Math.floor(width * (small ? 0.36 : 0.32)));
    right = Math.min(maxRight, Math.max(56, Math.ceil(widest + 16)));
  }

  const plotWidth = Math.max(1, width - left - right);
  const plotHeight = Math.max(1, height - top - bottom);
  const baseY = top + plotHeight;
  const axisY = baseY + markerLaneHeight;

  const toX = (h: number) => left + ((h - xDomain[0]) / (xDomain[1] - xDomain[0])) * plotWidth;
  const fromX = (px: number) => xDomain[0] + ((px - left) / plotWidth) * (xDomain[1] - xDomain[0]);
  const toY = (v: number) => {
    if (model.yAxis.log && yDomain[0] > 0 && yDomain[1] > yDomain[0] && v > 0) {
      const l0 = Math.log10(yDomain[0]);
      const l1 = Math.log10(yDomain[1]);
      return top + (1 - (Math.log10(v) - l0) / (l1 - l0)) * plotHeight;
    }
    return top + (1 - (v - yDomain[0]) / (yDomain[1] - yDomain[0])) * plotHeight;
  };

  const columns = model.mode === 'columns' && model.columns
    ? buildColumnsLayout(model.columns, model.hour, left, plotWidth, small)
    : [];

  const playheadX = model.mode === 'columns' && columns.length > 0
    ? columns[columns.length - 1]!.x + columns[columns.length - 1]!.width / 2
    : Math.max(left, Math.min(left + plotWidth, toX(model.hour)));

  const endLabels: PlacedEndLabel[] = [];
  if (rawEndLabels.length > 0) {
    const anchors = rawEndLabels.map(l => ({ id: l.id, y: toY(l.anchorVal), height: END_LABEL_HEIGHT }));
    const placedY = layoutEndLabels(anchors, top + 2, baseY + 4, END_LABEL_GAP);
    const lx = playheadX + 10;
    for (const l of rawEndLabels) {
      const anchorY = toY(l.anchorVal);
      const ly = placedY.get(l.id) ?? anchorY;
      endLabels.push({
        id: l.id,
        seriesId: l.seriesId,
        anchorY,
        y: ly,
        x: lx,
        width: Math.min(l.estWidth, Math.max(0, width - lx - 4)),
        height: 16,
        color: l.color,
        faded: l.faded,
        colorBar: l.colorBar,
        flag: l.flag,
        icon: l.icon,
        name: l.name,
        fullName: l.fullName,
        value: l.value,
        valueColor: l.valueColor,
        valueMuted: l.valueMuted,
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
      if (gb <= ga) continue;
      placedGaps.push({ index: i, gap, x0: toX(ga), x1: toX(gb) });
    }
  }

  const dateTabText = formatDateOnly(model.hour);
  const dateTabWidth = Math.ceil(measure(dateTabText, tabFont) + 14);
  const dateTabX = Math.max(left + dateTabWidth / 2, Math.min(playheadX, width - 6 - dateTabWidth / 2));

  const keepOut: [number, number][] = [[dateTabX, dateTabWidth / 2 + 14]];
  if (hoverHour != null && hoverHour >= xDomain[0] && hoverHour <= model.hour) {
    const hx = toX(hoverHour);
    const hTabW = Math.ceil(measure(formatDateOnly(hoverHour), tabFont) + 14);
    keepOut.push([Math.max(left + hTabW / 2, Math.min(hx, width - 6 - hTabW / 2)), hTabW / 2 + 14]);
  }

  const axisTicks: PlacedAxisTick[] = [];
  if (model.mode !== 'columns') {
    const spanHours = xDomain[1] - xDomain[0];
    const monthsMode = spanHours < 730 * 24;
    const startCal = gameDate(Math.floor(xDomain[0]));
    const endCal = gameDate(Math.floor(xDomain[1]));
    const rawTicks: { hour: number; label: string | null; major: boolean }[] = [];
    if (startCal && endCal && monthsMode) {
      let y = startCal.year;
      let m = startCal.month;
      let anyLabel = false;
      for (let guard = 0; guard < 36; guard++) {
        const h = monthStartHour(y, m);
        const clampedH = h < xDomain[0] && guard === 0 && startCal.day <= 15 ? xDomain[0] : h;
        if (clampedH > xDomain[1]) break;
        if (clampedH >= xDomain[0]) {
          const major = m === 1;
          let label: string | null = major ? String(y) : (m === 4 || m === 7 || m === 10 ? MONTHS[m - 1]! : null);
          if (label && !anyLabel && !major) label = `${label} ${y}`;
          if (label) anyLabel = true;
          rawTicks.push({ hour: clampedH, label, major });
        }
        m++;
        if (m > 12) { m = 1; y++; }
      }
    } else if (startCal && endCal) {
      for (let y = startCal.year; y <= endCal.year + 1; y++) {
        const h = yearStartHour(y);
        const clampedH = h < xDomain[0] && y === startCal.year && startCal.month === 1 && startCal.day <= 15 ? xDomain[0] : h;
        if (clampedH >= xDomain[0] && clampedH <= xDomain[1]) {
          rawTicks.push({ hour: clampedH, label: String(y), major: true });
        }
      }
    }
    let shownFirst = false;
    let lastLabelRight = -1e9;
    for (const tick of rawTicks) {
      const x = toX(tick.hour);
      let label = tick.label;
      if (label) {
        const lw = measure(label, smallFont);
        const hiddenByTab = keepOut.some(([kx, kr]) => Math.abs(x - kx) < kr);
        const hiddenBySmall = small && shownFirst;
        const hiddenByOverlap = x - lw / 2 < lastLabelRight + 8;
        if (hiddenByTab || hiddenBySmall || hiddenByOverlap) {
          label = null;
        } else {
          shownFirst = true;
          lastLabelRight = x + lw / 2;
        }
      }
      axisTicks.push({ hour: tick.hour, x, label, major: tick.major });
    }
  }

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
    yTicks,
    playheadX,
    dateTabText,
    dateTabX,
    dateTabWidth,
    axisTicks,
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
