import type { FamilyTooltipEntry, TooltipListItem } from '../host/tooltips.ts';
import type { BoundedSeries } from '../plan.ts';
import { formatDateOnly, notRecordedLabel } from '../valueFormat.ts';
import { formatChartAxisValue, type TimeChartLayout } from './layout.ts';
import type { TimeChartModel, TimeChartSeriesModel } from './model.ts';

export type ParsedChartInspect =
  | { readonly kind: 'marker'; readonly index: number; readonly hour?: number }
  | { readonly kind: 'label'; readonly seriesId: string }
  | { readonly kind: 'gap'; readonly index: number }
  | { readonly kind: 'fold'; readonly index: number }
  | { readonly kind: 'month'; readonly index: number }
  | { readonly kind: 'line'; readonly seriesId: string; readonly hour: number }
  | { readonly kind: 'plot'; readonly hour: number };

export function parseChartInspect(key: string | null | undefined): ParsedChartInspect | null {
  if (!key) return null;
  if (key.startsWith('marker:')) {
    const [rawIndex, rawHour] = key.slice(7).split('@');
    const index = Number(rawIndex);
    const hour = rawHour === undefined ? undefined : Number(rawHour);
    if (!Number.isInteger(index) || index < 0 || (hour !== undefined && !Number.isFinite(hour))) return null;
    return hour === undefined ? { kind: 'marker', index } : { kind: 'marker', index, hour };
  }
  if (key.startsWith('label:')) {
    const seriesId = key.slice(6);
    return seriesId ? { kind: 'label', seriesId } : null;
  }
  if (key.startsWith('gap:')) {
    const index = Number(key.slice(4));
    return Number.isInteger(index) && index >= 0 ? { kind: 'gap', index } : null;
  }
  if (key.startsWith('fold:')) {
    const index = Number(key.slice(5));
    return Number.isInteger(index) && index >= 0 ? { kind: 'fold', index } : null;
  }
  if (key.startsWith('month:')) {
    const index = Number(key.slice(6));
    return Number.isInteger(index) && index >= 0 ? { kind: 'month', index } : null;
  }
  if (key.startsWith('line:')) {
    const at = key.lastIndexOf('@');
    if (at <= 5) return null;
    const seriesId = key.slice(5, at);
    const hour = Number(key.slice(at + 1));
    return seriesId && Number.isFinite(hour) ? { kind: 'line', seriesId, hour } : null;
  }
  if (key.startsWith('plot:')) {
    const hour = Number(key.slice(5));
    return Number.isFinite(hour) ? { kind: 'plot', hour } : null;
  }
  return null;
}

function sampleBoundedAt(
  bounded: BoundedSeries,
  hour: number,
  scale = 1,
): { readonly value: number; readonly min: number; readonly max: number; readonly exact: boolean } | null {
  if (bounded.runs && bounded.runs.length > 0) {
    let lo = 0;
    let hi = bounded.runs.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >>> 1;
      const run = bounded.runs.at(mid)!;
      if (hour < run.firstHour) hi = mid - 1;
      else if (hour > run.lastHour) lo = mid + 1;
      else {
        const v = run.value * scale;
        return { value: v, min: v, max: v, exact: true };
      }
    }
    return null;
  }
  const idx = Math.floor((hour - bounded.from) / bounded.binHours);
  if (idx < 0 || idx >= bounded.buckets.length) return null;
  const bucket = bounded.buckets.at(idx);
  if (!bucket || hour < bucket.firstHour || hour > bucket.lastHour) return null;
  const exact = bucket.firstHour === bucket.lastHour || bucket.minValue === bucket.maxValue;
  return { value: bucket.lastValue * scale, min: bucket.minValue * scale, max: bucket.maxValue * scale, exact };
}

export function sampleSeriesValueAt(
  series: TimeChartSeriesModel,
  hour: number,
  until: number,
): { readonly value: number; readonly min: number; readonly max: number; readonly exact: boolean } | null {
  if (hour > until) return null;
  if (series.bounded) {
    const sampled = sampleBoundedAt(series.bounded, hour, series.valueScale ?? 1);
    if (sampled) return sampled;
    if (series.headValue != null && Number.isFinite(series.headValue) && hour === (series.headHour ?? until)) {
      return { value: series.headValue, min: series.headValue, max: series.headValue, exact: true };
    }
  }
  if (series.points && series.points.length > 0) {
    let prev: { readonly hour: number; readonly value: number | null } | null = null;
    for (const pt of series.points) {
      if (pt.hour > until) break;
      if (pt.hour > hour) {
        if (!prev || prev.value === null) return null;
        if (series.step || pt.value === null) {
          return { value: prev.value, min: prev.value, max: prev.value, exact: true };
        }
        const f = (hour - prev.hour) / Math.max(1, pt.hour - prev.hour);
        const v = prev.value + f * (pt.value - prev.value);
        return { value: v, min: v, max: v, exact: true };
      }
      prev = pt;
    }
    if (prev && prev.value !== null) {
      return { value: prev.value, min: prev.value, max: prev.value, exact: true };
    }
  }
  return null;
}

export function hitTestTimeChart(
  model: TimeChartModel,
  layout: TimeChartLayout,
  point: readonly [number, number],
): string | null {
  const [px, py] = point;
  if (px < 0 || py < 0 || px > layout.width || py > layout.height) return null;

  for (const label of layout.endLabels) {
    if (px >= label.x - 4 && px <= label.x + label.width + 4 && py >= label.y - label.height / 2 - 2 && py <= label.y + label.height / 2 + 2) {
      return `label:${label.seriesId}`;
    }
  }

  if (layout.markerLaneHeight > 0 && py >= layout.baseY && py <= layout.axisY + 2 && px >= layout.left - 8 && px <= layout.left + layout.plotWidth + 8) {
    let best: (typeof layout.markers)[number] | null = null;
    let bestDist = 11;
    for (const m of layout.markers) {
      const d = Math.abs(px - m.x);
      if (d <= bestDist) {
        bestDist = d;
        best = m;
      }
    }
    if (best) return `marker:${best.index}@${best.marker.hour}`;
  }

  if (px < layout.left || px > layout.left + layout.plotWidth || py < layout.top - 4 || py > layout.baseY + 4) {
    return null;
  }

  if (model.mode === 'columns') {
    for (const col of layout.columns) {
      if (px >= col.x && px <= col.x + col.width) {
        return col.kind === 'fold' ? `fold:${col.index}` : `month:${col.index}`;
      }
    }
    return null;
  }

  for (const gap of layout.gaps) {
    if (px >= gap.x0 && px <= gap.x1) {
      return `gap:${gap.index}`;
    }
  }

  const rawHour = Math.round(layout.fromX(px));
  const clampedHour = Math.max(Math.floor(layout.xDomain[0]), Math.min(Math.floor(model.hour), rawHour));

  if (model.mode === 'lines') {
    let bestId: string | null = null;
    let bestDist = 9;
    for (const s of model.series) {
      const sampled = sampleSeriesValueAt(s, clampedHour, model.hour);
      if (!sampled) continue;
      const sy = layout.toY(sampled.value);
      const d = Math.abs(py - sy);
      if (d <= bestDist) {
        bestDist = d;
        bestId = s.id;
      }
    }
    if (bestId) return `line:${bestId}@${clampedHour}`;
  }

  return `plot:${clampedHour}`;
}

function formatExactFigure(value: number, model: TimeChartModel): string {
  if (model.yAxis.format === 'percent') {
    return `${Math.round(value * 10) / 10}%`;
  }
  if (model.yAxis.format === 'signed1') {
    if (Math.abs(value) < 1e-9) return '0';
    return `${value < 0 ? '\u2212' : '+'}${Math.abs(value).toFixed(2).replace(/\.?0+$/, '')}`;
  }
  const decimals = model.yAxis.decimals ?? 0;
  const sign = value < 0 ? '\u2212' : '';
  const abs = Math.abs(value);
  if (decimals > 0 && abs < 1000) {
    return `${sign}${abs.toLocaleString('en-US', { maximumFractionDigits: decimals })}`;
  }
  return `${sign}${Math.round(abs).toLocaleString('en-US')}`;
}

function artUrlOf(slot: { readonly art: { readonly status: string; readonly url?: string } } | undefined): string | undefined {
  return slot?.art.status === 'ready' ? slot.art.url : undefined;
}

function seriesTooltipItemsAt(model: TimeChartModel, hour: number): TooltipListItem[] {
  const entries: { readonly sortVal: number; readonly item: TooltipListItem }[] = [];
  if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    let at = -1;
    for (let i = 0; i < model.stack.hours.length; i++) {
      if (model.stack.hours[i]! <= hour) at = i;
      else break;
    }
    if (at >= 0) {
      for (const band of model.stack.bands) {
        const v = band.values[at];
        if (v === null || v === undefined || !Number.isFinite(v)) continue;
        const flagUrl = artUrlOf(band.flag);
        entries.push({
          sortVal: v,
          item: {
            label: band.name,
            value: formatExactFigure(v, model),
            swatch: band.color,
            ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
          },
        });
      }
    }
  } else {
    for (const s of model.series) {
      const flagUrl = artUrlOf(s.flag);
      const iconUrl = artUrlOf(s.icon);
      if (s.exact && s.exact.hour === hour) {
        if (s.exact.value === null) {
          entries.push({
            sortVal: -Infinity,
            item: {
              label: s.name,
              missingReason: s.exact.reason ?? notRecordedLabel(),
              swatch: s.color,
              tone: 'muted',
              ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            },
          });
        } else {
          const negative = s.negRed && s.exact.value < 0;
          entries.push({
            sortVal: s.exact.value,
            item: {
              label: s.name,
              value: formatExactFigure(s.exact.value, model),
              swatch: negative ? '#ff6b6f' : s.color,
              tone: negative ? 'negative' : undefined,
              ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
              ...(iconUrl ? { icon: iconUrl } : {}),
            },
          });
        }
        continue;
      }
      const missingSpan = s.missingSpans?.find(sp => hour >= sp.firstHour && hour <= sp.lastHour);
      if (missingSpan) {
        entries.push({
          sortVal: -Infinity,
          item: {
            label: s.name,
            missingReason: missingSpan.reason,
            swatch: s.color,
            tone: 'muted',
            ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
          },
        });
        continue;
      }
      const sampled = sampleSeriesValueAt(s, hour, model.hour);
      if (!sampled) {
        if (s.absentReason) {
          entries.push({
            sortVal: -Infinity,
            item: {
              label: s.name,
              missingReason: s.absentReason,
              swatch: s.color,
              tone: 'muted',
              ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            },
          });
        }
        continue;
      }
      const text = sampled.exact || sampled.min === sampled.max
        ? formatExactFigure(sampled.value, model)
        : `${formatChartAxisValue(sampled.min, model.yAxis.format)}\u2013${formatChartAxisValue(sampled.max, model.yAxis.format)}`;
      entries.push({
        sortVal: sampled.value,
        item: {
          label: s.name,
          value: text,
          swatch: s.negRed && sampled.value < 0 ? '#ff6b6f' : s.color,
          tone: s.negRed && sampled.value < 0 ? 'negative' : undefined,
          ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
          ...(iconUrl ? { icon: iconUrl } : {}),
        },
      });
    }
  }
  entries.sort((a, b) => b.sortVal - a.sortVal);
  return entries.map(e => e.item);
}

export function deriveTimeChartTooltip(
  model: TimeChartModel,
  layout: TimeChartLayout,
  inspectKey: string | null | undefined,
): FamilyTooltipEntry | null {
  const parsed = parseChartInspect(inspectKey);
  if (!parsed || !inspectKey) return null;

  if (parsed.kind === 'gap') {
    const placed = layout.gaps.find(item => item.index === parsed.index);
    if (!placed) return null;
    const span = placed.ongoing
      ? `From ${formatDateOnly(placed.gap.firstHour)}`
      : `${formatDateOnly(placed.gap.firstHour)} \u2013 ${formatDateOnly(placed.gap.lastHour)}`;
    return {
      hour: placed.gap.firstHour,
      rowKey: inspectKey,
      payload: {
        title: notRecordedLabel(),
        subtitle: span,
        note: placed.gap.reason,
      },
    };
  }

  if (parsed.kind === 'fold') {
    const col = layout.columns[parsed.index];
    if (!col || col.kind !== 'fold' || !col.foldMonths?.length) return null;
    const hasNotRecorded = col.foldMonths.some(m => m.status === 'not-recorded');
    const hasNoRows = col.foldMonths.some(m => m.status === 'no-rows');
    const title = hasNoRows && !hasNotRecorded ? 'No loss rows recorded' : notRecordedLabel();
    const note = col.foldMonths.find(m => m.reason)?.reason
      ?? (hasNoRows && !hasNotRecorded
        ? `${col.foldMonths.length} months with no loss rows recorded`
        : `${col.foldMonths.length} unrecorded months folded`);
    return {
      hour: col.foldMonths[0]!.firstHour,
      rowKey: inspectKey,
      payload: {
        title,
        subtitle: col.foldSpanText ?? col.foldLabel ?? '',
        note,
      },
    };
  }

  if (parsed.kind === 'month') {
    const col = layout.columns[parsed.index];
    if (!col || col.kind !== 'month' || !col.month) return null;
    const m = col.month;
    const title = `${formatDateOnly(m.firstHour).replace(/^\d+\s+/, '')}${m.partial && m.partialToHour != null ? ` (to ${formatDateOnly(m.partialToHour).replace(/\s+\d{4}$/, '')})` : ''}`;
    if (m.status !== 'recorded') {
      const label = m.status === 'no-rows' ? 'No loss rows recorded for this month' : notRecordedLabel();
      return {
        hour: m.firstHour,
        rowKey: inspectKey,
        payload: {
          title: label,
          subtitle: title,
          ...(m.reason ? { note: m.reason } : {}),
        },
      };
    }
    const items: TooltipListItem[] = m.segments.map(seg => ({
      label: seg.label,
      value: seg.exactText,
      swatch: seg.color,
    }));
    items.push({ label: 'Total', value: m.totalText, strong: true, divider: true });
    return {
      hour: m.firstHour,
      rowKey: inspectKey,
      payload: {
        title,
        subtitle: model.measureTitle,
        items,
      },
    };
  }

  if (parsed.kind === 'marker') {
    const placed = layout.markers[parsed.index];
    if (!placed) return null;
    const m = placed.marker;
    const eventItems: TooltipListItem[] = (m.items && m.items.length > 0 ? m.items : [{ title: m.title, detail: m.detail, icon: m.icon, flag: m.flag }]).map(item => {
      const iconUrl = artUrlOf(item.icon);
      const flagUrl = artUrlOf(item.flag);
      return {
        label: item.title,
        ...(item.detail ? { value: item.detail } : {}),
        ...(iconUrl ? { icon: iconUrl } : {}),
        ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
        strong: true,
      };
    });
    const valueItems = seriesTooltipItemsAt(model, m.hour);
    if (valueItems.length > 0 && eventItems.length > 0) {
      valueItems[0] = { ...valueItems[0]!, divider: true };
    }
    return {
      hour: m.hour,
      rowKey: inspectKey,
      payload: {
        title: formatDateOnly(m.hour),
        items: [...eventItems, ...valueItems],
      },
    };
  }

  if (parsed.kind === 'label') {
    const s = model.series.find(item => item.id === parsed.seriesId);
    if (!s) return null;
    const items = seriesTooltipItemsAt(model, model.hour).filter(r => r.label === s.name);
    return {
      hour: model.hour,
      rowKey: inspectKey,
      payload: {
        title: s.endLabel?.fullName ?? s.name,
        subtitle: model.measureTitle,
        items,
      },
    };
  }

  const hour = parsed.hour;
  const items = seriesTooltipItemsAt(model, hour);
  const detail = model.hoverDetail && model.hoverDetail.hour === hour ? model.hoverDetail.items : null;
  if (detail?.length) {
    if (items.length) items.push({ ...detail[0]!, divider: true }, ...detail.slice(1));
    else items.push(...detail);
  }
  if (!items.length) return null;
  return {
    hour,
    rowKey: inspectKey,
    payload: {
      title: model.measureTitle,
      items,
      scrollable: items.length > 10,
    },
  };
}
