import { CHART_MEASURE_COLOURS, nudgeApart, readableColour } from '../../charts/palette.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { liveKey } from '../../charts/live.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import type { ChartGap, ChartMarker, ChartStackBand, TimeChartModel } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import type { WorldTensionReply, WorldTensionStackReply } from '../../generated/dashboard';

export interface WorldTensionChartOptions {
  readonly extent: readonly [number, number];
  readonly surface: string;
  readonly layers: number;
  readonly values: 'share' | 'recorded';
  readonly showMarkers: boolean;
  readonly gaps: readonly ChartGap[];
}

const OTHER_COLOR = CHART_MEASURE_COLOURS.mutedFallback;
const FALLBACK_COLOR = CHART_MEASURE_COLOURS.mutedFallback;

export function validateWorldTensionStack(raw: unknown): WorldTensionStackReply {
  const reply = raw as WorldTensionStackReply;
  if (
    !reply
    || typeof reply.branchId !== 'string'
    || !Array.isArray(reply.hours)
    || !Array.isArray(reply.total)
    || !Array.isArray(reply.bands)
    || !Array.isArray(reply.events)
    || !Array.isArray(reply.missing)
    || !Array.isArray(reply.warnings)
  ) {
    throw new Error('Invalid world tension stack reply');
  }
  return reply;
}

export function validateWorldTensionHover(raw: unknown): WorldTensionReply {
  const reply = raw as WorldTensionReply;
  if (!reply || !Array.isArray(reply.contributions) || !Array.isArray(reply.sources) || !Array.isArray(reply.warnings)) {
    throw new Error('Invalid world tension reply');
  }
  return reply;
}

function formatPct(value: number, decimals = 0): string {
  if (!Number.isFinite(value)) return '0%';
  if (decimals === 0) return `${Math.round(value)}%`;
  return `${(Math.round(value * 10 ** decimals) / 10 ** decimals).toFixed(decimals).replace(/\.0+$/, '')}%`;
}

function formatRawPct(raw: string, scale: number, decimals = 1): string {
  const n = Number(raw);
  if (!Number.isFinite(n) || scale <= 0) return '0%';
  const pct = n / scale;
  const sign = pct > 0 ? '+' : pct < 0 ? '\u2212' : '';
  const abs = Math.abs(pct);
  const rounded = (Math.round(abs * 10 ** decimals) / 10 ** decimals).toFixed(decimals).replace(/\.0+$/, '');
  return `${sign}${rounded}%`;
}

function contributorLabel(
  readView: BoundedReadView,
  tag: string | null,
  remembered: boolean,
  atHour: number,
): { readonly name: string; readonly fullName: string; readonly color: string | null; readonly flagChoice: string | null } {
  if (!tag) return { name: 'Unknown', fullName: 'Unknown country', color: null, flagChoice: null };
  const id = readView.who('country', tag, atHour);
  const baseName = id.name.status === 'ready' ? id.name.value : tag;
  const fullName = remembered ? `${baseName} (former)` : baseName;
  const color = id.colour?.status === 'ready' ? id.colour.value : null;
  const flagChoice = id.flagChoice?.status === 'ready' ? id.flagChoice.value : null;
  return { name: baseName, fullName, color, flagChoice };
}

function resolveBandColors(rawColors: readonly (string | null)[], surface: string): string[] {
  const surfaceRgb = parseColor(surface) ?? { r: 17, g: 17, b: 17 };
  const parsed: { r: number; g: number; b: number }[] = [];
  const indices: number[] = [];
  for (let i = 0; i < rawColors.length; i++) {
    const c = rawColors[i] ? parseColor(readableColour(rawColors[i]!, surface)) : null;
    if (c) {
      parsed.push(c);
      indices.push(i);
    }
  }
  const nudged = parsed.length > 0 ? nudgeApart(parsed, surfaceRgb) : [];
  const out: string[] = rawColors.map(() => FALLBACK_COLOR);
  for (let j = 0; j < indices.length; j++) {
    out[indices[j]!] = toHex(nudged[j]!);
  }
  return out;
}

function buildMarkers(
  readView: BoundedReadView,
  reply: WorldTensionStackReply,
  hour: number,
): ChartMarker[] {
  const byHour = new Map<number, WorldTensionStackReply['events'][number][]>();
  for (const ev of reply.events) {
    if (ev.hour > hour || ev.firstRead > hour) continue;
    let bucket = byHour.get(ev.hour);
    if (!bucket) {
      bucket = [];
      byHour.set(ev.hour, bucket);
    }
    bucket.push(ev);
  }
  const markers: ChartMarker[] = [];
  for (const [evHour, list] of [...byHour.entries()].sort((a, b) => a[0] - b[0])) {
    const ordered = [...list].sort((a, b) => Math.abs(Number(b.addedRaw)) - Math.abs(Number(a.addedRaw)));
    const lead = ordered[0]!;
    const leadInfo = contributorLabel(readView, lead.tag, lead.remembered, Math.min(lead.firstRead, hour));
    const leadFlag = leadInfo.flagChoice
      ? { art: readView.art(flagArtHandle(leadInfo.flagChoice, 'panel')), tag: lead.tag ?? undefined, label: leadInfo.name }
      : undefined;
    const items = ordered.map(ev => {
      const info = contributorLabel(readView, ev.tag, ev.remembered, Math.min(ev.firstRead, hour));
      const flag = info.flagChoice
        ? { art: readView.art(flagArtHandle(info.flagChoice, 'panel')), tag: ev.tag ?? undefined, label: info.name }
        : undefined;
      return {
        title: ev.reason,
        detail: formatRawPct(ev.addedRaw, reply.contributionScale, 1),
        ...(flag ? { flag } : {}),
      };
    });
    markers.push({
      id: `wt:${evHour}`,
      hour: evHour,
      title: lead.reason,
      detail: formatRawPct(lead.addedRaw, reply.contributionScale, 1),
      ...(leadFlag ? { flag: leadFlag } : {}),
      items,
    });
  }
  return markers;
}

function buildHoverDetail(
  readView: BoundedReadView,
  reply: WorldTensionStackReply,
  hour: number,
): TimeChartModel['hoverDetail'] {
  const hover = readView.hover<WorldTensionReply>('hover');
  if (!hover || hover.hour > hour) return null;
  const detail = hover.data;
  if (!detail) return null;
  const cells = repliedCells(detail).read();
  const tensionVal = cells.get(liveKey('world', 'tension'));
  const sumVal = cells.get(liveKey('world', 'sum'));
  const items: TooltipListItem[] = [];
  if (tensionVal === undefined) {
    items.push({
      label: 'World tension',
      missingReason: detail.missing ?? cells.missing(liveKey('world', 'tension')),
      tone: 'muted',
      strong: true,
    });
    return { hour: hover.hour, items, replacesSeries: true };
  }
  const tensionPct = Number(tensionVal) / detail.tensionScale;
  items.push({ label: 'World tension', value: formatPct(tensionPct, 1), strong: true });
  if (sumVal !== undefined) {
    const sumPct = Number(sumVal) / detail.contributionScale;
    if (Math.abs(sumPct - tensionPct) >= 0.05) {
      items.push({ label: 'Uncapped sum', value: formatPct(sumPct, 1), tone: 'muted' });
    }
  }
  const topContribs = detail.contributions.slice(0, 10);
  topContribs.forEach((c, idx) => {
    const val = cells.get(liveKey(String(c.slot), 'value'));
    const info = contributorLabel(readView, c.tag, c.remembered, hover.hour);
    const flagRes = info.flagChoice ? readView.art({ kind: 'flag', choice: info.flagChoice, size: 'small' }) : null;
    items.push({
      label: info.fullName,
      ...(val !== undefined
        ? { value: formatPct(Number(val) / detail.contributionScale, 1) }
        : { missingReason: cells.missing(liveKey(String(c.slot), 'value')), tone: 'muted' as const }),
      ...(flagRes?.status === 'ready' && flagRes.url ? { countryFlag: { flag: flagRes.url } } : {}),
      ...(idx === 0 ? { divider: true } : {}),
    });
  });
  if (detail.contributions.length > topContribs.length) {
    items.push({ label: `${detail.contributions.length - topContribs.length} more countries`, tone: 'muted' });
  }
  const sameHourEvents = reply.events.filter(ev => ev.hour === hover.hour && ev.firstRead <= hour);
  sameHourEvents.forEach((ev, idx) => {
    items.push({
      label: ev.reason,
      value: formatRawPct(ev.addedRaw, reply.contributionScale, 1),
      strong: true,
      ...(idx === 0 ? { divider: true } : {}),
    });
  });
  return { hour: hover.hour, items, replacesSeries: true };
}

export const worldTensionProjection: TimeChartProjection<WorldTensionChartOptions> = registerFamilyProjection({
  kind: 'world-tension',
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'step',
  measureTitle: 'World tension',
  project(readView: BoundedReadView, options: WorldTensionChartOptions): TimeChartModel {
    const hour = readView.hour;
    const reply = readView.panel<WorldTensionStackReply>('main');
    const refused = readView.refused('main');
    if (refused) {
      return {
        kind: 'world-tension',
        measureTitle: 'World tension',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: { format: 'percent', fixed: options.values === 'share' ? [0, 100] : null, min: 0 },
        series: [],
        empty: { message: refused, kind: 'unavailable' },
      };
    }
    if (!reply) {
      return {
        kind: 'world-tension',
        measureTitle: 'World tension',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: { format: 'percent', fixed: options.values === 'share' ? [0, 100] : null, min: 0 },
        series: [],
      };
    }
    const gaps: ChartGap[] = [
      ...options.gaps,
      ...reply.missing.map(m => ({ firstHour: m.firstHour, lastHour: m.lastHour, reason: m.reason })),
    ];
    const hasRecorded = reply.total.some(v => v !== null);
    if (!hasRecorded) {
      const reason = reply.missing[0]?.reason ?? 'World tension was not recorded for this period.';
      return {
        kind: 'world-tension',
        measureTitle: 'World tension',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: { format: 'percent', fixed: options.values === 'share' ? [0, 100] : null, min: 0 },
        series: [],
        warnings: reply.warnings,
        empty: { message: reason, kind: 'unavailable' },
      };
    }
    const endHour = reply.endHour ?? hour;
    const bandInfos = reply.bands.map(band => contributorLabel(readView, band.tag, band.remembered, endHour));
    const bandColors = resolveBandColors(bandInfos.map(info => info.color), options.surface);
    const orderedBands: ChartStackBand[] = [];
    for (let i = 0; i < reply.bands.length; i++) {
      const band = reply.bands[i]!;
      const info = bandInfos[i]!;
      const color = bandColors[i]!;
      const flag = info.flagChoice
        ? { art: readView.art(flagArtHandle(info.flagChoice, 'panel')), tag: band.tag ?? undefined, label: info.name }
        : undefined;
      const endVal = band.endValue;
      orderedBands.push({
        id: `slot:${band.slot}`,
        key: band.tag ?? `slot:${band.slot}`,
        name: info.fullName,
        color,
        opacity: 0.84,
        values: band.values,
        ...(flag ? { flag } : {}),
        endLabel: endVal != null && endVal >= 0.05
          ? {
              ...(flag ? { flag } : {}),
              name: info.name,
              fullName: info.fullName,
              value: formatPct(endVal, 1),
            }
          : null,
      });
    }
    if (reply.other) {
      const otherLabel = reply.other.count > 0 ? `Other (${reply.other.count})` : 'Other';
      const otherEnd = reply.other.endValue;
      orderedBands.push({
        id: 'other',
        key: 'other',
        name: otherLabel,
        color: OTHER_COLOR,
        opacity: 0.72,
        values: reply.other.values,
        endLabel: otherEnd != null && otherEnd >= 0.05
          ? {
              name: otherLabel,
              shortName: 'Other',
              fullName: otherLabel,
              value: formatPct(otherEnd, 1),
            }
          : null,
      });
    }
    let lastTotal: number | null = null;
    for (let i = reply.hours.length - 1; i >= 0; i--) {
      if (reply.hours[i]! <= hour) {
        lastTotal = reply.total[i] ?? null;
        break;
      }
    }
    const worldIcon = { art: readView.art(measureArtHandle('worldTension')), label: 'World tension' };
    const markers = options.showMarkers ? buildMarkers(readView, reply, hour) : undefined;
    const hoverDetail = buildHoverDetail(readView, reply, hour);
    const auditRows = [
      ...(lastTotal !== null ? [{ row: 'World tension', key: '__total__', value: formatPct(lastTotal) }] : []),
      ...reply.bands.flatMap((band, idx) => (band.endValue != null && band.endValue >= 0.05
        ? [{ row: bandInfos[idx]!.fullName, key: band.tag ?? `slot:${band.slot}`, value: formatPct(band.endValue) }]
        : [])),
      ...(reply.other && reply.other.endValue != null && reply.other.endValue >= 0.05
        ? [{ row: reply.other.count > 0 ? `Other (${reply.other.count})` : 'Other', key: 'other', value: formatPct(reply.other.endValue) }]
        : []),
    ];
    return {
      kind: 'world-tension',
      measureTitle: 'World tension',
      mode: 'stack',
      hour,
      extent: options.extent,
      yAxis: {
        format: 'percent',
        fixed: options.values === 'share' ? [0, 100] : null,
        min: 0,
      },
      series: [],
      stack: {
        hours: reply.hours,
        bands: orderedBands,
        step: true,
        total: {
          color: CHART_MEASURE_COLOURS.lightInk,
          values: reply.total,
          endLabel: lastTotal !== null ? { icon: worldIcon, value: formatPct(lastTotal) } : null,
        },
      },
      ...(markers && markers.length > 0 ? { markers } : {}),
      ...(gaps.length > 0 ? { gaps } : {}),
      ...(hoverDetail ? { hoverDetail } : {}),
      warnings: reply.warnings,
      auditRows,
    };
  },
}, 'tension');

export const worldTensionDeclaration: TimeChartWidgetDeclaration<WorldTensionChartOptions> = {
  family: 'time-chart',
  kind: 'world-tension',
  label: 'World tension',
  subject: 'world tension',
  emptyCampaign: 'Load a campaign to see world tension.',
  icon: 'chart-line',
  countries: { mode: 'multi', labels: true, colours: true, allTagsForLabels: true },
  sources(ctx: FamilyPlanContext, view): SourcePlan {
    const layers = Math.max(1, Math.min(20, Number(ctx.options.layers) || 5));
    const values = ctx.options.values === 'recorded' ? 'recorded' : 'share';
    const countries = ctx.countries.all ? null : [...ctx.countries.tags];
    const width = Math.max(240, Math.min(2400, Math.ceil(view.width ?? 1024)));
    return {
      owner: `world-tension:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [
        {
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'population', form: 'all' },
            product: 'world-tension-stack',
            followQuery: null,
            params: {
              branchId: ctx.branch.id,
              layers,
              values,
              countries,
              width,
            },
            dynamicKeys: ['firstHour', 'lastHour', 'width'],
            validate: validateWorldTensionStack,
            provenInterval: (data, _requested, hour) => {
              const reply = data as WorldTensionStackReply;
              return reply.nextChangeHour !== null ? [hour, reply.nextChangeHour - 1] : [hour, Infinity];
            },
          },
        },
        {
          name: 'hover',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'exact-hover',
            product: 'world-tension',
            params: { branchId: ctx.branch.id },
            validate: validateWorldTensionHover,
          },
        },
      ],
    };
  },
  projection: worldTensionProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      extent,
      surface,
      layers: Math.max(1, Math.min(20, Number(ctx.options.layers) || 5)),
      values: ctx.options.values === 'recorded' ? 'recorded' : 'share',
      showMarkers: ctx.options.markers !== false,
      gaps: branchGapsOf(ctx.branch),
    };
  },
};
