import { auditRow } from '../../families/auditRow.ts';
import { CHART_MEASURE_COLOURS, nudgeApart, readableColour } from '../../charts/palette.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { liveKey } from '../../charts/live.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import type { ChartExactRational, ChartGap, ChartMarker, ChartStackBand, TimeChartModel } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { formatRational, formatRecordedPercent, formatScaledPercent, formatShare, formatShareRatio } from '../../families/valueFormat.ts';
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
const WORLD_UNKNOWN = 'World tension was not recorded at this hour.';
const END_UNKNOWN = 'Not recorded at this hour.';
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

function parseRawInt(raw: string | null | undefined): bigint | null {
  return typeof raw === 'string' && /^-?\d+$/.test(raw) ? BigInt(raw) : null;
}

function exactBandRational(
  rawStr: string | null | undefined,
  mode: 'share' | 'recorded',
  tensionRawStr: string | null | undefined,
  sumRawStr: string | null | undefined,
  tensionScale: number,
  contributionScale: number,
): ChartExactRational | null {
  const raw = parseRawInt(rawStr);
  if (raw === null) return null;
  if (mode === 'recorded') {
    return contributionScale > 0 ? { raw, denominator: BigInt(contributionScale) } : null;
  }
  const tension = parseRawInt(tensionRawStr);
  const sum = parseRawInt(sumRawStr);
  if (tension === null || sum === null || tensionScale <= 0) return null;
  if (sum > 0n) {
    return { raw: raw * tension, denominator: sum * BigInt(tensionScale) };
  }
  return tension === 0n && sum === 0n && raw === 0n ? { raw: 0n, denominator: 1n } : null;
}

function formatEndPct(value: number, mode: 'share' | 'recorded', exact: ChartExactRational | null): string {
  if (exact) {
    if (mode === 'share' && exact.raw >= 0n && exact.raw <= exact.denominator * 100n) {
      return formatShareRatio(exact.raw, exact.denominator * 100n);
    }
    return formatRational(exact.raw, exact.denominator, { maxDecimals: 5, percent: true });
  }
  return mode === 'share' && value >= 0 && value <= 100 ? formatShare(value / 100) : formatRecordedPercent(value);
}

function formatTotalPct(value: number, exact: ChartExactRational | null): string {
  return exact ? formatRational(exact.raw, exact.denominator, { maxDecimals: 5, percent: true }) : formatRecordedPercent(value);
}

function formatRawPct(raw: string, scale: number, signed = true): string | null {
  if (!/^-?\d+$/.test(raw) || !Number.isSafeInteger(scale) || scale <= 0) return null;
  return formatScaledPercent(BigInt(raw), BigInt(scale), { signed });
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
      const detail = formatRawPct(ev.addedRaw, reply.contributionScale);
      return {
        title: ev.reason,
        ...(detail !== null ? { detail } : {}),
        ...(flag ? { flag } : {}),
      };
    });
    const leadDetail = formatRawPct(lead.addedRaw, reply.contributionScale);
    markers.push({
      id: `wt:${evHour}`,
      hour: evHour,
      title: lead.reason,
      ...(leadDetail !== null ? { detail: leadDetail } : {}),
      ...(leadFlag ? { flag: leadFlag } : {}),
      items,
    });
  }
  return markers;
}

function buildHoverDetail(
  readView: BoundedReadView,
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
  } else {
    const tensionRaw = cells.raw?.(liveKey('world', 'tension'));
    const tensionPct = Number(tensionVal) / detail.tensionScale;
    const tensionText = tensionRaw !== undefined
      ? formatRawPct(tensionRaw.toString(), detail.tensionScale, false)
      : Number.isFinite(tensionPct) ? formatRecordedPercent(tensionPct) : null;
    items.push(tensionText !== null
      ? { label: 'World tension', value: tensionText, strong: true }
      : { label: 'World tension', missingReason: detail.missing ?? cells.missing(liveKey('world', 'tension')), tone: 'muted', strong: true });
  }
  if (sumVal !== undefined) {
    const sumRaw = cells.raw?.(liveKey('world', 'sum'));
    const sumPct = Number(sumVal) / detail.contributionScale;
    const tensionPct = tensionVal !== undefined ? Number(tensionVal) / detail.tensionScale : null;
    if (tensionPct === null || Math.abs(sumPct - tensionPct) >= 0.05) {
      const sumText = sumRaw !== undefined
        ? formatRawPct(sumRaw.toString(), detail.contributionScale, false)
        : Number.isFinite(sumPct) ? formatRecordedPercent(sumPct) : null;
      if (sumText !== null) items.push({ label: 'Uncapped sum', value: sumText, tone: 'muted' });
    }
  }
  const topContribs = detail.contributions.slice(0, 10);
  topContribs.forEach((c, idx) => {
    const key = liveKey(String(c.slot), 'value');
    const val = cells.get(key);
    const valRaw = cells.raw?.(key);
    const info = contributorLabel(readView, c.tag, c.remembered, hover.hour);
    const flagRes = info.flagChoice ? readView.art({ kind: 'flag', choice: info.flagChoice, size: 'small' }) : null;
    const valText = val !== undefined
      ? (valRaw !== undefined
        ? formatRawPct(valRaw.toString(), detail.contributionScale, false)
        : Number.isFinite(Number(val) / detail.contributionScale) ? formatRecordedPercent(Number(val) / detail.contributionScale) : null)
      : null;
    items.push({
      label: info.fullName,
      ...(valText !== null
        ? { value: valText }
        : { missingReason: cells.missing(key), tone: 'muted' as const }),
      ...(flagRes?.status === 'ready' && flagRes.url ? { countryFlag: { flag: flagRes.url } } : {}),
      ...(idx === 0 ? { divider: true } : {}),
    });
  });
  if (detail.contributions.length > topContribs.length) {
    items.push({ label: `${detail.contributions.length - topContribs.length} more countries`, tone: 'muted' });
  }
  const sameHourSources = detail.sources.filter(src => src.date === hover.hour);
  sameHourSources.forEach((src, idx) => {
    const evText = formatRawPct(src.finalRaw, detail.contributionScale);
    items.push({
      label: src.reason,
      ...(evText !== null ? { value: evText } : { missingReason: 'Tension amount not recorded', tone: 'muted' as const }),
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
    if (reply.refused) {
      return {
        kind: 'world-tension',
        measureTitle: 'World tension',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: { format: 'percent', fixed: options.values === 'share' ? [0, 100] : null, min: 0 },
        series: [],
        warnings: reply.warnings,
        empty: { message: reply.refused, kind: 'unavailable' },
      };
    }
    const gaps: ChartGap[] = [
      ...options.gaps,
      ...reply.missing.map(m => ({ firstHour: m.firstHour, lastHour: m.lastHour, reason: m.reason })),
    ];
    const hasRecorded = reply.total.some(v => v !== null)
      || reply.bands.some(b => b.values.some(v => v !== null))
      || (reply.other?.values.some(v => v !== null) ?? false);
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
    const hasNegative = reply.total.some(v => v !== null && v < 0)
      || reply.bands.some(b => b.values.some(v => v !== null && v < 0))
      || (reply.other?.values.some(v => v !== null && v < 0) ?? false);
    const endHour = reply.endHour;
    const totalScale = reply.tensionScale;
    const totalExactSeries: readonly (ChartExactRational | null)[] | undefined = Array.isArray(reply.totalRaw) && reply.totalRaw.length === reply.hours.length && totalScale > 0
      ? reply.totalRaw.map(rawStr => {
          const parsed = parseRawInt(rawStr);
          return parsed !== null ? { raw: parsed, denominator: BigInt(totalScale) } : null;
        })
      : undefined;
    const bandInfos = reply.bands.map(band => contributorLabel(readView, band.tag, band.remembered, hour));
    const bandColors = resolveBandColors(bandInfos.map(info => info.color), options.surface);
    const orderedBands: ChartStackBand[] = [];
    const bandEndExacts: (ChartExactRational | null)[] = [];
    for (let i = 0; i < reply.bands.length; i++) {
      const band = reply.bands[i]!;
      const info = bandInfos[i]!;
      const color = bandColors[i]!;
      const flag = info.flagChoice
        ? { art: readView.art(flagArtHandle(info.flagChoice, 'panel')), tag: band.tag ?? undefined, label: info.name }
        : undefined;
      const endVal = endHour !== null ? band.endValue : null;
      const endExact = endHour !== null
        ? exactBandRational(band.endRaw, options.values, reply.cappedEndTensionRaw, reply.endSumRaw, reply.tensionScale, reply.contributionScale)
        : null;
      bandEndExacts.push(endExact);
      const validEnd = endVal != null && Number.isFinite(endVal) && (endExact ? endExact.raw !== 0n : endVal !== 0);
      const bandExactSeries: readonly (ChartExactRational | null)[] | undefined = Array.isArray(band.valuesRaw) && band.valuesRaw.length === reply.hours.length
        ? band.valuesRaw.map((rawStr, idx) =>
            exactBandRational(rawStr, options.values, reply.totalRaw?.[idx], reply.sumRaw?.[idx], reply.tensionScale, reply.contributionScale))
        : undefined;
      orderedBands.push({
        id: `slot:${band.slot}`,
        key: `slot:${band.slot}`,
        name: info.fullName,
        color,
        opacity: 0.84,
        values: band.values,
        ...(bandExactSeries ? { exact: bandExactSeries } : {}),
        ...(flag ? { flag } : {}),
        endLabel: validEnd
          ? {
              ...(flag ? { flag } : {}),
              name: info.name,
              fullName: info.fullName,
              value: formatEndPct(endVal, options.values, endExact),
            }
          : null,
      });
    }
    const otherEndExact = endHour !== null && reply.other
      ? exactBandRational(reply.other.endRaw, options.values, reply.cappedEndTensionRaw, reply.endSumRaw, reply.tensionScale, reply.contributionScale)
      : null;
    if (reply.other) {
      const otherLabel = reply.other.count > 0 ? `Other (${reply.other.count})` : 'Other';
      const otherEnd = endHour !== null ? reply.other.endValue : null;
      const validOther = otherEnd != null && Number.isFinite(otherEnd) && (otherEndExact ? otherEndExact.raw !== 0n : otherEnd !== 0);
      const otherExactSeries: readonly (ChartExactRational | null)[] | undefined = Array.isArray(reply.other.valuesRaw) && reply.other.valuesRaw.length === reply.hours.length
        ? reply.other.valuesRaw.map((rawStr, idx) =>
            exactBandRational(rawStr, options.values, reply.totalRaw?.[idx], reply.sumRaw?.[idx], reply.tensionScale, reply.contributionScale))
        : undefined;
      orderedBands.push({
        id: 'other',
        key: 'other',
        name: otherLabel,
        color: OTHER_COLOR,
        opacity: 0.72,
        values: reply.other.values,
        ...(otherExactSeries ? { exact: otherExactSeries } : {}),
        endLabel: validOther
          ? {
              name: otherLabel,
              shortName: 'Other',
              fullName: otherLabel,
              value: formatEndPct(otherEnd, options.values, otherEndExact),
            }
          : null,
      });
    }
    const worldRaw = parseRawInt(reply.cappedEndTensionRaw);
    const lastTotalExact: ChartExactRational | null = worldRaw !== null && totalScale > 0 ? { raw: worldRaw, denominator: BigInt(totalScale) } : null;
    const lastTotal = reply.cappedEndTension !== null && Number.isFinite(reply.cappedEndTension) ? reply.cappedEndTension : null;
    const worldText = lastTotal !== null ? formatTotalPct(lastTotal, lastTotalExact) : null;
    const endText = (value: number | null, exact: ChartExactRational | null, missing: string | null) =>
      endHour !== null && value !== null && Number.isFinite(value) ? formatEndPct(value, options.values, exact) : (missing ?? END_UNKNOWN);
    const worldIcon = { art: readView.art(measureArtHandle('worldTension')), label: 'World tension' };
    const markers = options.showMarkers ? buildMarkers(readView, reply, hour) : undefined;
    const hoverDetail = buildHoverDetail(readView, hour);
    const auditRows = [
      auditRow('__total__', worldText ?? reply.endMissing ?? WORLD_UNKNOWN),
      ...reply.bands.map((band, i) => auditRow(`slot:${band.slot}`, endText(band.endValue, bandEndExacts[i] ?? null, band.endMissing))),
      ...(reply.other ? [auditRow('other', endText(reply.other.endValue, otherEndExact, reply.other.endMissing))] : []),
    ];
    return {
      kind: 'world-tension',
      measureTitle: 'World tension',
      mode: 'stack',
      hour,
      extent: options.extent,
      yAxis: {
        format: 'percent',
        fixed: options.values === 'share' && !hasNegative ? [0, 100] : null,
        ...(hasNegative ? {} : { min: 0 }),
      },
      series: [],
      stack: {
        hours: reply.hours,
        bands: orderedBands,
        bandFormat: 'signedPercent',
        step: true,
        total: {
          color: CHART_MEASURE_COLOURS.lightInk,
          values: reply.total,
          ...(totalExactSeries ? { exact: totalExactSeries } : {}),
          endLabel: worldText !== null ? { icon: worldIcon, value: worldText } : null,
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
    const top = ctx.countries.top ?? null;
    const keyed = !ctx.countries.all && top == null;
    const group = keyed ? groupSelection(ctx.widget.countries) : null;
    const countries = keyed && !group ? [...ctx.countries.tags] : null;
    const width = Math.max(240, Math.min(2400, Math.ceil(view.width ?? 1024)));
    const includeEvents = ctx.options.markers !== false;
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
              top,
              group,
              width,
              includeStack: true,
              includeEvents,
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
