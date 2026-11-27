import type { FamilyTooltipEntry, TooltipListItem } from '../host/tooltips.ts';
import type { BoundedSeries } from '../plan.ts';
import { formatDateHover, formatDateOnly, formatRational, formatRecordedPercent, formatScalar, formatShare, formatShareRatio, notRecordedLabel } from '../valueFormat.ts';
import { formatChartAxisValue, type TimeChartLayout } from './layout.ts';
import { seriesLookAt, type ChartValueFormat, type TimeChartModel, type TimeChartSeriesModel } from './model.ts';
import { hitChartRows, rowsTooltip } from './rows.ts';

const NEGATIVE_SWATCH = 'var(--short-ink)';

export type ParsedChartInspect =
  | { readonly kind: 'marker'; readonly index: number; readonly hour?: number }
  | { readonly kind: 'label'; readonly seriesId: string }
  | { readonly kind: 'gap'; readonly index: number; readonly hour?: number }
  | { readonly kind: 'fold'; readonly index: number; readonly hour?: number }
  | { readonly kind: 'month'; readonly index: number; readonly hour?: number }
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
    const [rawIndex, rawHour] = key.slice(4).split('@');
    const index = Number(rawIndex);
    const hour = rawHour === undefined ? undefined : Number(rawHour);
    if (!Number.isInteger(index) || index < 0 || (hour !== undefined && !Number.isFinite(hour))) return null;
    return hour === undefined ? { kind: 'gap', index } : { kind: 'gap', index, hour };
  }
  if (key.startsWith('fold:')) {
    const [rawIndex, rawHour] = key.slice(5).split('@');
    const index = Number(rawIndex);
    const hour = rawHour === undefined ? undefined : Number(rawHour);
    if (!Number.isInteger(index) || index < 0 || (hour !== undefined && !Number.isFinite(hour))) return null;
    return hour === undefined ? { kind: 'fold', index } : { kind: 'fold', index, hour };
  }
  if (key.startsWith('month:')) {
    const [rawIndex, rawHour] = key.slice(6).split('@');
    const index = Number(rawIndex);
    const hour = rawHour === undefined ? undefined : Number(rawHour);
    if (!Number.isInteger(index) || index < 0 || (hour !== undefined && !Number.isFinite(hour))) return null;
    return hour === undefined ? { kind: 'month', index } : { kind: 'month', index, hour };
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
  if (idx === bounded.buckets.closedLength && bounded.straddling === 'cut' && bounded.straddlingRuns?.length) {
    for (const run of bounded.straddlingRuns) {
      if (hour < run.firstHour) break;
      if (hour <= run.lastHour) {
        const v = run.value * scale;
        return { value: v, min: v, max: v, exact: true };
      }
    }
    return null;
  }
  const bucket = bounded.buckets.at(idx);
  if (!bucket || hour < bucket.firstHour || hour > bucket.lastHour) return null;
  const exact = !bucket.hasBreak && (bucket.firstHour === bucket.lastHour || bucket.minValue === bucket.maxValue);
  return { value: bucket.lastValue * scale, min: bucket.minValue * scale, max: bucket.maxValue * scale, exact };
}

export function sampleSeriesValueAt(
  series: TimeChartSeriesModel,
  hour: number,
  until: number,
): { readonly value: number; readonly min: number; readonly max: number; readonly exact: boolean; readonly raw?: bigint; readonly denominator?: bigint } | null {
  if (hour > until || series.missingSpans?.some(span => span.firstHour <= hour && hour <= span.lastHour)) return null;
  if (series.bounded) {
    const sampled = sampleBoundedAt(series.bounded, hour, series.valueScale ?? 1);
    if (sampled) return sampled;
  }
  if (series.points && series.points.length > 0) {
    const pointSample = (pt: { readonly value: number | null; readonly raw?: bigint; readonly denominator?: bigint; readonly min?: number | null; readonly max?: number | null; readonly approximate?: boolean }) => {
      if (pt.value === null) return null;
      const min = pt.min != null && Number.isFinite(pt.min) ? pt.min : pt.value;
      const max = pt.max != null && Number.isFinite(pt.max) ? pt.max : pt.value;
      return {
        value: pt.value,
        min,
        max,
        exact: !pt.approximate && min === max,
        ...(pt.raw !== undefined && pt.denominator !== undefined ? { raw: pt.raw, denominator: pt.denominator } : {}),
      };
    };
    let prev: (typeof series.points)[number] | null = null;
    for (const pt of series.points) {
      if (pt.hour > until) break;
      if (pt.hour > hour) {
        if (!prev || prev.value === null) return null;
        if (series.step || pt.value === null || prev.approximate) {
          return pointSample(prev);
        }
        const f = (hour - prev.hour) / Math.max(1, pt.hour - prev.hour);
        const v = prev.value + f * (pt.value - prev.value);
        return { value: v, min: v, max: v, exact: true };
      }
      prev = pt;
    }
    if (prev && prev.value !== null) {
      return pointSample(prev);
    }
  }
  if (series.headValue != null && Number.isFinite(series.headValue) && hour === (series.headHour ?? until)) {
    return { value: series.headValue, min: series.headValue, max: series.headValue, exact: true };
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
  if (layout.rows) return hitChartRows(layout.rows, point);

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
        if (col.kind === 'fold') {
          const hour = col.foldMonths?.[0]?.firstHour;
          return hour !== undefined ? `fold:${col.index}@${hour}` : `fold:${col.index}`;
        }
        const hour = col.month?.firstHour;
        return hour !== undefined ? `month:${col.index}@${hour}` : `month:${col.index}`;
      }
    }
    return null;
  }

  const rawHour = Math.round(layout.fromX(px));
  const clampedHour = Math.max(Math.floor(layout.xDomain[0]), Math.min(Math.floor(model.hour), rawHour));

  for (const gap of layout.gaps) {
    if (px >= gap.x0 && px <= gap.x1) {
      const gapLast = Math.min(Math.floor(model.hour), Math.floor(gap.gap.lastHour));
      const gapHour = Math.max(Math.floor(gap.gap.firstHour), Math.min(gapLast, clampedHour));
      return `gap:${gap.index}@${gapHour}`;
    }
  }

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
  } else if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    let at = -1;
    for (let i = 0; i < model.stack.hours.length; i++) {
      if (model.stack.hours[i]! <= clampedHour) at = i;
      else break;
    }
    if (at >= 0) {
      let posAcc = 0;
      let negAcc = 0;
      for (const band of model.stack.bands) {
        const v = band.values[at];
        if (v == null || !Number.isFinite(v) || band.outside?.[at]) continue;
        const base = v >= 0 ? posAcc : negAcc;
        const yBot = layout.toY(base);
        const yTop = layout.toY(base + v);
        if (v >= 0) posAcc += v;
        else negAcc += v;
        if (py >= Math.min(yTop, yBot) && py <= Math.max(yTop, yBot)) {
          return `line:${band.id}@${clampedHour}`;
        }
      }
    }
  }

  return `plot:${clampedHour}`;
}

function formatExactFigure(value: number, model: TimeChartModel, format: ChartValueFormat, raw?: bigint, denominator?: bigint): string {
  if (raw !== undefined && denominator !== undefined) {
    if (format === 'share') {
      return raw >= 0n && raw <= denominator * 100n
        ? formatShareRatio(raw, denominator * 100n)
        : formatRational(raw, denominator, { maxDecimals: 5, percent: true });
    }
    if (format === 'percent' || format === 'signedPercent') {
      return formatRational(raw, denominator, { maxDecimals: 5, percent: true, signed: format === 'signedPercent' });
    }
    if (format === 'signed1') {
      if (raw === 0n) return '0';
      return formatRational(raw, denominator, { maxDecimals: 2, signed: true });
    }
    const decimals = model.yAxis.exactDecimals ?? model.yAxis.decimals ?? 0;
    const absWhole = (raw < 0n ? -raw : raw) / denominator;
    if (decimals > 0 && (format === 'number' || absWhole < 1000n || model.yAxis.exactDecimals !== undefined)) {
      return formatRational(raw, denominator, { maxDecimals: decimals });
    }
    return formatRational(raw, denominator, { maxDecimals: 0 });
  }
  if (format === 'share') {
    return value >= 0 && value <= 100 ? formatShare(value / 100) : formatRecordedPercent(value);
  }
  if (format === 'percent' || format === 'signedPercent') {
    return formatRecordedPercent(value, { signed: format === 'signedPercent' });
  }
  if (format === 'signed1') {
    if (value === 0) return '0';
    return formatScalar(value, { decimals: 0, maxDecimals: 2, signed: true });
  }
  const decimals = model.yAxis.exactDecimals ?? model.yAxis.decimals ?? 0;
  if (decimals > 0 && (Math.abs(value) < 1000 || model.yAxis.exactDecimals !== undefined)) {
    return formatScalar(value, { decimals: 0, maxDecimals: decimals });
  }
  return formatScalar(value, 'integer');
}

function artUrlOf(slot: { readonly art: { readonly status: string; readonly url?: string } } | undefined): string | undefined {
  return slot?.art.status === 'ready' ? slot.art.url : undefined;
}

function seriesTooltipItemsAt(model: TimeChartModel, hour: number, only?: string, mergeHover = false): TooltipListItem[] {
  const entries: { readonly sortVal: number; readonly item: TooltipListItem }[] = [];
  if (model.mode === 'stack' && model.stack && model.stack.hours.length > 0) {
    if (hour > model.hour) return [];
    let at = -1;
    for (let i = 0; i < model.stack.hours.length; i++) {
      if (model.stack.hours[i]! <= hour) at = i;
      else break;
    }
    const anyBandAt = at >= 0 && model.stack.bands.some(band => (
      (band.values[at] !== null && band.values[at] !== undefined && Number.isFinite(band.values[at]) && !band.outside?.[at])
      || Boolean(!band.outside?.[at] && band.missingReason?.[at])
    ));
    if (at >= 0 && !anyBandAt && !(model.stack.total?.values[at] != null) && !model.stack.total?.missingReason?.[at]) {
      return [];
    }
    const insideGrouped = model.stack.bucketed
      ? true
      : Boolean(model.stack.reduced && model.stack.hours[at] !== hour && at + 1 < model.stack.hours.length);
    if (at >= 0 && (insideGrouped || model.stack.hoverPendingMessage)) {
      return mergeHover && model.hoverDetail?.hour === hour
        ? []
        : [{
            label: model.measureTitle,
            missingReason: insideGrouped
              ? 'This hour is inside a grouped span; its figures come from the read of this hour'
              : model.stack.hoverPendingMessage!,
            tone: 'muted',
          }];
    }
    if (at >= 0) {
      const totalVal = model.stack.total?.values[at];
      const totalExact = model.stack.total?.exact?.[at];
      const totalMissing = model.stack.total?.missingReason?.[at];
      const totalItem: TooltipListItem | null = totalVal !== null && totalVal !== undefined && Number.isFinite(totalVal)
        ? { label: model.measureTitle, value: formatExactFigure(totalVal, model, model.yAxis.format, totalExact?.raw, totalExact?.denominator), strong: true }
        : totalMissing
          ? { label: model.measureTitle, missingReason: totalMissing, tone: 'muted', strong: true }
          : null;
      for (const band of model.stack.bands) {
        if (only !== undefined && band.id !== only) continue;
        const v = band.values[at];
        if (v === null || v === undefined || !Number.isFinite(v) || band.outside?.[at]) {
          const reason = !band.outside?.[at] ? band.missingReason?.[at] : null;
          if (reason) {
            const look = seriesLookAt(band, hour);
            const flagUrl = artUrlOf(look.flag);
            entries.push({
              sortVal: -Infinity,
              item: {
                label: look.name,
                missingReason: reason,
                swatch: look.color,
                tone: 'muted',
                ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
              },
            });
          }
          continue;
        }
        const bandExact = band.exact?.[at];
        const look = seriesLookAt(band, hour);
        const flagUrl = artUrlOf(look.flag);
        if (v === 0 && band.clampedZeroReason) {
          entries.push({
            sortVal: -Infinity,
            item: {
              label: look.name,
              missingReason: band.clampedZeroReason,
              swatch: look.color,
              tone: 'muted',
              ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            },
          });
          continue;
        }
        entries.push({
          sortVal: v,
          item: {
            label: look.name,
            value: formatExactFigure(v, model, model.stack.bandFormat ?? model.yAxis.format, bandExact?.raw, bandExact?.denominator),
            swatch: look.color,
            ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
          },
        });
      }
      entries.sort((a, b) => b.sortVal - a.sortVal);
      const bandItems = entries.map(e => e.item);
      if (totalItem && only === undefined) {
        if (bandItems.length > 0) bandItems[0] = { ...bandItems[0]!, divider: true };
        return [totalItem, ...bandItems];
      }
      return bandItems;
    }
  } else {
    for (const s of model.series) {
      if (only !== undefined && s.id !== only) continue;
      const look = seriesLookAt(s, hour);
      const flagUrl = artUrlOf(look.flag);
      const iconUrl = artUrlOf(s.icon);
      if (s.exact && s.exact.hour === hour) {
        if (s.exact.value === null) {
          entries.push({
            sortVal: -Infinity,
            item: {
              label: look.name,
              missingReason: s.exact.reason ?? notRecordedLabel(),
              swatch: look.color,
              tone: 'muted',
              ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            },
          });
        } else {
          const negative = s.negRed && (s.exact.raw !== undefined ? s.exact.raw < 0n : s.exact.value < 0);
          entries.push({
            sortVal: s.exact.value,
            item: {
              label: look.name,
              value: formatExactFigure(s.exact.value, model, model.yAxis.format, s.exact.raw, s.exact.denominator),
              swatch: negative ? NEGATIVE_SWATCH : look.color,
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
            label: look.name,
            missingReason: missingSpan.reason,
            swatch: look.color,
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
              label: look.name,
              missingReason: s.absentReason,
              swatch: look.color,
              tone: 'muted',
              ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            },
          });
        }
        continue;
      }
      if (!sampled.exact && sampled.min === sampled.max) {
        entries.push({
          sortVal: -Infinity,
          item: {
            label: look.name,
            missingReason: 'This hour is inside a grouped span; its figures come from the read of this hour',
            swatch: look.color,
            tone: 'muted',
            ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
            ...(iconUrl ? { icon: iconUrl } : {}),
          },
        });
        continue;
      }
      const partialSpan = s.partialSpans?.find(sp => hour >= sp.firstHour && hour <= sp.lastHour);
      const text = sampled.exact
        ? formatExactFigure(sampled.value, model, model.yAxis.format, sampled.raw, sampled.denominator)
        : `${formatChartAxisValue(sampled.min, model.yAxis.format)}\u2013${formatChartAxisValue(sampled.max, model.yAxis.format)}`;
      entries.push({
        sortVal: sampled.value,
        item: {
          label: look.name,
          value: text,
          ...(partialSpan ? { partialReason: partialSpan.reason } : {}),
          swatch: s.negRed && sampled.value < 0 ? NEGATIVE_SWATCH : look.color,
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
  if (layout.rows) return rowsTooltip(model, inspectKey);
  const parsed = parseChartInspect(inspectKey);
  if (!parsed || !inspectKey) return null;

  if (parsed.kind === 'gap') {
    const placed = layout.gaps.find(item => item.index === parsed.index);
    if (!placed) return null;
    const span = placed.ongoing
      ? `From ${formatDateOnly(placed.gap.firstHour)}`
      : `${formatDateOnly(placed.gap.firstHour)} \u2013 ${formatDateOnly(placed.gap.lastHour)}`;
    return {
      hour: parsed.hour ?? placed.gap.firstHour,
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
    if (m.missingParts) {
      for (const part of m.missingParts) {
        items.push({
          label: part.label,
          swatch: part.color,
          missingReason: part.reason,
          tone: 'muted',
        });
      }
    }
    if (m.totalMissingReason) {
      items.push({ label: 'Total', missingReason: m.totalMissingReason, tone: 'muted', strong: true, divider: true });
    } else {
      items.push({ label: 'Total', value: m.totalText, strong: true, divider: true });
    }
    return {
      hour: m.firstHour,
      rowKey: inspectKey,
      payload: {
        title,
        subtitle: model.measureTitle,
        items,
        ...(m.note ? { note: m.note } : {}),
      },
    };
  }

  if (parsed.kind === 'marker') {
    const placed = layout.markers[parsed.index];
    if (!placed) return null;
    const m = placed.marker;
    const eventItems: TooltipListItem[] = [];
    for (const marker of placed.members ?? [m]) {
      const date = placed.members ? formatDateHover(marker.hour) : null;
      for (const item of marker.items?.length ? marker.items : [marker]) {
        const iconUrl = artUrlOf(item.icon);
        const flagUrl = artUrlOf(item.flag);
        const value = date && item.detail ? `${date} · ${item.detail}` : date ?? item.detail;
        eventItems.push({
          label: item.title,
          ...(value ? { value } : {}),
          ...(iconUrl ? { icon: iconUrl } : {}),
          ...(flagUrl ? { countryFlag: { flag: flagUrl } } : {}),
          strong: true,
        });
      }
    }
    const detail = model.hoverDetail;
    const valueItems = detail?.hour === m.hour && detail.replacesSeries ? [...detail.items] : seriesTooltipItemsAt(model, m.hour);
    if (valueItems.length > 0 && eventItems.length > 0) {
      valueItems[0] = { ...valueItems[0]!, divider: true };
    }
    return {
      hour: m.hour,
      rowKey: inspectKey,
      payload: {
        title: formatDateOnly(m.hour),
        items: [...eventItems, ...valueItems],
        ...(placed.members ? { scrollable: true } : {}),
      },
    };
  }

  if (parsed.kind === 'label') {
    if (model.mode === 'stack' && model.stack) {
      if (parsed.seriesId === '__total__') {
        const items = seriesTooltipItemsAt(model, model.hour);
        return {
          hour: model.hour,
          rowKey: inspectKey,
          payload: {
            title: model.measureTitle,
            subtitle: formatDateOnly(model.hour),
            items,
          },
        };
      }
      const band = model.stack.bands.find(b => b.id === parsed.seriesId);
      if (!band) return null;
      const items = seriesTooltipItemsAt(model, model.hour, band.id);
      return {
        hour: model.hour,
        rowKey: inspectKey,
        payload: {
          title: band.endLabel?.fullName ?? band.name,
          subtitle: model.measureTitle,
          items,
        },
      };
    }
    const s = model.series.find(item => item.id === parsed.seriesId);
    if (s) {
      const labelHour = Math.min(model.hour, s.headHour ?? model.hour);
      const items = seriesTooltipItemsAt(model, labelHour, s.id);
      return {
        hour: labelHour,
        rowKey: inspectKey,
        payload: {
          title: s.endLabel?.fullName ?? s.name,
          subtitle: model.measureTitle,
          items,
        },
      };
    }
    const band = model.mode === 'stack' ? model.stack?.bands.find(item => item.id === parsed.seriesId) : undefined;
    if (!band) return null;
    const items = seriesTooltipItemsAt(model, model.hour, band.id);
    return {
      hour: model.hour,
      rowKey: inspectKey,
      payload: {
        title: band.endLabel?.fullName ?? band.name,
        subtitle: model.measureTitle,
        items,
      },
    };
  }

  const hour = Math.floor(parsed.hour);
  const atHour = model.hoverDetail && model.hoverDetail.hour === hour ? model.hoverDetail : null;
  const items = atHour?.replacesSeries ? [] : seriesTooltipItemsAt(model, hour, undefined, Boolean(atHour));
  const detail = (parsed.kind === 'line' ? atHour?.itemsBySeries?.get(parsed.seriesId) : undefined) ?? atHour?.items;
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
