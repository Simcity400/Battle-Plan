import { auditRow } from '../../families/auditRow.ts';
import { categoricalPalette, CHART_MEASURE_COLOURS, nudgeApart, readableColour } from '../../charts/palette.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { TRAILING_HOURS } from '../../dashboard/model.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView, BoundedSeries, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { ChartGap, TimeChartModel, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import { branchGapsOf, missingSpansOf } from '../../families/time-chart/seriesHelpers.ts';
import type { ApmPlayer, ApmReply, ApmSeries, ApmValue } from '../../generated/dashboard';
import { GAPS_NOTE, playerNames, validateApm } from '../player-apm/apmData.ts';
import { apmHeadQuery, apmHeadRows } from '../player-apm/apmHead.ts';

const TOP_NAMED = 8;
const NOT_RECORDED_CAMPAIGN = 'Player orders were not recorded in this campaign.';

export interface ApmGraphOptions {
  readonly group: 'country' | 'player';
  readonly extent: readonly [number, number];
  readonly surface: string;
  readonly gaps: readonly ChartGap[];
  readonly yMin: number | null;
  readonly yMax: number | null;
  readonly available: boolean | null;
  readonly players?: readonly ApmPlayer[];
}

function formatApm(value: number): string {
  return String(Math.round(value));
}

function unknownApmReason(value: ApmValue): string {
  if (!value.complete) return 'Player orders or unpaused time were not fully recorded for this period; the rate is unknown.';
  if (value.unpausedMinutes === 0) return 'No unpaused time was observed in this period.';
  return 'It is unclear whether some orders count toward eAPM; the rate is unknown.';
}

function hasRecordedByHour(bounded: BoundedSeries | null, hour: number): boolean {
  if (!bounded) return false;
  if (bounded.runs) {
    for (let i = 0; i < bounded.runs.length; i++) {
      const r = bounded.runs.at(i);
      if (r && r.firstHour <= hour) return true;
    }
  }
  for (let i = 0; i < bounded.buckets.length; i++) {
    const b = bounded.buckets.at(i);
    if (b && b.firstHour <= hour) return true;
  }
  return false;
}

interface SeriesEntry {
  readonly id: string;
  readonly name: string | null;
  readonly tag: string | null;
  readonly obsHour: number;
  readonly headValue: number | null;
  readonly gaps: boolean;
  readonly bounded: BoundedSeries | null;
}

function resolveSeriesColors(
  readView: BoundedReadView,
  entries: readonly SeriesEntry[],
  hour: number,
  surface: string,
): string[] {
  const surfaceRgb = parseColor(surface) ?? { r: 17, g: 17, b: 17 };
  const raw: { r: number; g: number; b: number }[] = [];
  const indices: number[] = [];
  for (let i = 0; i < entries.length; i++) {
    const tag = entries[i]!.tag;
    const obsHour = Number.isFinite(entries[i]!.obsHour) ? Math.min(entries[i]!.obsHour, hour) : hour;
    const id = tag ? readView.who('country', tag, obsHour) : null;
    const rgb = id?.colour?.status === 'ready' ? parseColor(readableColour(id.colour.value, surface)) : null;
    if (rgb) {
      raw.push(rgb);
      indices.push(i);
    }
  }
  const nudged = raw.length > 0 ? nudgeApart(raw, surfaceRgb) : [];
  const fallbacks = categoricalPalette(surface);
  const out: string[] = entries.map((_, i) => fallbacks[i % fallbacks.length]!);
  for (let j = 0; j < indices.length; j++) {
    out[indices[j]!] = toHex(nudged[j]!);
  }
  return out;
}

function datedPlayerIdentity(
  player: ApmPlayer | undefined,
  atHour: number,
): { readonly name: string | null; readonly tag: string | null } {
  if (!player) return { name: null, tag: null };
  let chosen: ApmPlayer['runs'][number] | null = null;
  for (const run of player.runs ?? []) {
    if (run.firstHour <= atHour) chosen = run;
    else break;
  }
  if (!chosen) return { name: null, tag: null };
  return { name: chosen.name, tag: chosen.tag };
}

function buildHoverDetail(
  readView: BoundedReadView,
  options: ApmGraphOptions,
  colorById: ReadonlyMap<string, string>,
  hour: number,
): TimeChartModel['hoverDetail'] {
  const hover = readView.hover<ApmReply>('hover');
  if (!hover || hover.hour > hour || !hover.data) return null;
  const reply = hover.data;
  if (!reply.available) return null;
  const hoverDisambiguated = options.group === 'player' ? playerNames(reply.series) : null;
  const items: TooltipListItem[] = [];
  let anyGaps = false;
  for (const row of reply.series) {
    if (row.summary.gaps) anyGaps = true;
    const identity = row.tag ? readView.who('country', row.tag, hover.hour) : null;
    const flagChoice = identity?.flagChoice?.status === 'ready' ? identity.flagChoice.value : null;
    const flagRes = flagChoice ? readView.art(flagArtHandle(flagChoice, 'small')) : null;
    const label = options.group === 'player'
      ? (hoverDisambiguated?.get(row.id) ?? row.name ?? 'Name not recorded')
      : (identity?.name.status === 'ready' ? identity.name.value : row.tag ?? row.id);
    const swatch = colorById.get(row.id);
    items.push({
      label,
      ...(row.summary.apm !== null
        ? { value: formatApm(row.summary.apm) }
        : { missingReason: unknownApmReason(row.summary), tone: 'muted' as const }),
      ...(swatch ? { swatch } : {}),
      ...(flagRes?.status === 'ready' && flagRes.url ? { countryFlag: { flag: flagRes.url } } : {}),
    });
  }
  if (anyGaps) {
    items.push({ label: GAPS_NOTE, tone: 'muted', ...(items.length > 0 ? { divider: true } : {}) });
  }
  if (items.length === 0) return null;
  return { hour: hover.hour, items, replacesSeries: true };
}

function extractHeadRows(readView: BoundedReadView): { readonly rows: readonly ApmSeries[]; readonly ready: boolean; readonly refusal: string | null; readonly warnings: readonly string[] } {
  const answer: FollowedAnswer | undefined = readView.answer('head');
  if (answer) {
    if (answer.kind === 'notRecorded') return { rows: [], ready: true, refusal: answer.reason, warnings: [] };
    return { rows: apmHeadRows(answer), ready: true, refusal: null, warnings: [] };
  }
  const panel = readView.panel<ApmReply>('head');
  if (panel) {
    if (!panel.available) return { rows: [], ready: true, refusal: NOT_RECORDED_CAMPAIGN, warnings: panel.warnings };
    return { rows: panel.series, ready: true, refusal: null, warnings: panel.warnings };
  }
  return { rows: [], ready: false, refusal: null, warnings: [] };
}

export const apmGraphProjection: TimeChartProjection<ApmGraphOptions> = registerFamilyProjection({
  kind: 'apm-graph',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'join',
  measureTitle: 'eAPM',
  project(readView: BoundedReadView, options: ApmGraphOptions): TimeChartModel {
    const hour = readView.hour;
    const yAxis = {
      format: 'number' as const,
      ...(options.yMin !== null ? { min: options.yMin } : { min: 0 }),
      ...(options.yMax !== null ? { max: options.yMax } : {}),
    };
    if (options.available === false) {
      return {
        kind: 'apm-graph',
        measureTitle: 'eAPM',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis,
        series: [],
        empty: { message: NOT_RECORDED_CAMPAIGN, kind: 'unavailable' },
      };
    }
    const { rows, ready, refusal, warnings } = extractHeadRows(readView);
    if (refusal) {
      return {
        kind: 'apm-graph',
        measureTitle: 'eAPM',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis,
        series: [],
        warnings,
        empty: { message: refusal, kind: 'unavailable' },
      };
    }
    const tiled = readView.series('apm');
    const registryById = new Map<string, ApmPlayer>((options.players ?? []).map(p => [p.id, p]));
    const entries: SeriesEntry[] = [];
    const seen = new Set<string>();
    for (const row of rows) {
      seen.add(row.id);
      const obsHour = Number.isFinite(row.hour) && row.hour > 0 ? Math.min(row.hour, hour) : hour;
      entries.push({
        id: row.id,
        name: row.name,
        tag: row.tag,
        obsHour,
        headValue: row.summary.apm,
        gaps: row.summary.gaps,
        bounded: tiled?.get(row.id) ?? null,
      });
    }
    if (tiled) {
      const historicalIds = [...tiled.keys()].filter(id => !seen.has(id)).sort();
      for (const id of historicalIds) {
        const bounded = tiled.get(id) ?? null;
        if (!hasRecordedByHour(bounded, hour)) continue;
        const reg = options.group === 'player' ? registryById.get(id) : undefined;
        const lastHour = bounded?.buckets.length
          ? Math.min(bounded.buckets.at(bounded.buckets.length - 1)?.lastHour ?? hour, hour)
          : hour;
        const dated = options.group === 'player' ? datedPlayerIdentity(reg, lastHour) : { name: id, tag: id };
        entries.push({
          id,
          name: dated.name,
          tag: dated.tag,
          obsHour: lastHour,
          headValue: null,
          gaps: false,
          bounded,
        });
      }
    }
    if (!entries.length) {
      return {
        kind: 'apm-graph',
        measureTitle: 'eAPM',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis,
        series: [],
        warnings,
        ...(ready ? { empty: { message: 'No eAPM is available for these countries, players and dates.', kind: 'empty' as const } } : {}),
      };
    }
    const disambiguated = options.group === 'player' ? playerNames(entries) : null;
    const topEntries = entries.slice(0, TOP_NAMED);
    const topColors = resolveSeriesColors(readView, topEntries, hour, options.surface);
    const colorById = new Map<string, string>();
    const series: TimeChartSeriesModel[] = entries.map((entry, rank) => {
      const isTop = rank < TOP_NAMED;
      const identity = entry.tag ? readView.who('country', entry.tag, entry.obsHour) : null;
      const flagChoice = identity?.flagChoice?.status === 'ready' ? identity.flagChoice.value : null;
      const flag = flagChoice
        ? { art: readView.art(flagArtHandle(flagChoice, 'panel')), tag: entry.tag ?? undefined, label: entry.tag ?? undefined }
        : undefined;
      const name = options.group === 'player'
        ? (disambiguated?.get(entry.id) ?? entry.name ?? 'Name not recorded')
        : (identity?.name.status === 'ready' ? identity.name.value : entry.tag ?? entry.id);
      const color = isTop ? topColors[rank]! : CHART_MEASURE_COLOURS.mutedFallback;
      colorById.set(entry.id, color);
      return {
        id: entry.id,
        key: entry.id,
        name,
        color,
        role: isTop ? 'primary' : 'tail',
        step: false,
        bounded: entry.bounded,
        headValue: entry.headValue,
        headHour: entry.obsHour,
        ...(flag ? { flag } : {}),
        missingSpans: missingSpansOf(entry.bounded),
        rank,
        endLabel: isTop && entry.headValue !== null
          ? {
              ...(flag ? { flag } : {}),
              name,
              fullName: name,
              value: formatApm(entry.headValue),
              ...(entry.gaps ? { valueMuted: true } : {}),
            }
          : null,
      };
    });
    const anyHeadGaps = entries.some(e => e.gaps);
    const hoverDetail = buildHoverDetail(readView, options, colorById, hour);
    const auditRows = series
      .filter(s => s.role === 'primary' && s.headValue !== null && s.headValue !== undefined)
      .map(s => (auditRow(s.id, formatApm(s.headValue!))));
    return {
      kind: 'apm-graph',
      measureTitle: 'eAPM',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis,
      series,
      ...(options.gaps.length > 0 ? { gaps: options.gaps } : {}),
      ...(anyHeadGaps ? { note: GAPS_NOTE } : {}),
      ...(hoverDetail ? { hoverDetail } : {}),
      warnings,
      auditRows,
    };
  },
});

function headParamsOf(ctx: FamilyPlanContext<ApmReply>) {
  const membership = ctx.countries.membershipRuns
    ? ctx.countries.membershipRuns.flatMap(run => {
        const held = ctx.gateSpan(run.firstHour, run.lastHour);
        return held ? [{ tag: run.tag, ...held }] : [];
      })
    : null;
  const group: 'country' | 'player' = ctx.options.group === 'country' ? 'country' : 'player';
  return {
    branchId: ctx.branch.id,
    group,
    tags: ctx.countries.all ? null : [...ctx.countries.tags],
    membership,
    players: contentSet(ctx.options.players).map(contentKey),
  };
}

function candidateSeriesIds(ctx: FamilyPlanContext<ApmReply>, group: 'country' | 'player'): string[] {
  const registry = ctx.aux;
  if (!registry || !registry.available) return [];
  const wantedTags = ctx.countries.all ? null : new Set(ctx.countries.tags);
  const wantedPlayers = new Set(contentSet(ctx.options.players).map(contentKey));
  if (group === 'player') {
    return registry.players
      .filter(p => (wantedPlayers.size === 0 || wantedPlayers.has(p.id)) && (!wantedTags || p.tags.some(t => wantedTags.has(t))))
      .map(p => p.id);
  }
  const tags = new Set<string>();
  for (const p of registry.players) {
    for (const t of p.tags) {
      if (!wantedTags || wantedTags.has(t)) tags.add(t);
    }
  }
  return [...tags].sort();
}

export const apmGraphDeclaration: TimeChartWidgetDeclaration<ApmGraphOptions, ApmReply> = {
  family: 'time-chart',
  kind: 'apm-graph',
  label: 'eAPM graph',
  subject: 'eAPM',
  emptyCampaign: 'Load a campaign to see eAPM.',
  icon: 'chart-line',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSingleSlot: true,
    allTagsForLabels: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group',
  },
  aux: {
    product: 'player-apm',
    params: track => ({
      branchId: track.branch.id,
      group: 'player',
      registry: true,
      firstHour: null,
      lastHour: null,
      tags: null,
      membership: null,
      players: [],
      buckets: 1,
    }),
    validate: validateApm,
  },
  sources(ctx: FamilyPlanContext<ApmReply>): SourcePlan {
    if (ctx.aux && (!ctx.aux.available || ctx.aux.players.length === 0)) {
      return {
        owner: `apm-graph:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
        entries: [],
        unavailable: NOT_RECORDED_CAMPAIGN,
      };
    }
    const headParams = headParamsOf(ctx);
    const ids = candidateSeriesIds(ctx, headParams.group);
    return {
      owner: `apm-graph:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [
        {
          name: 'head',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'trailing-window',
            coverage: ctx.countries.all ? { mode: 'population', form: 'all' } : { mode: 'keyed', keys: ctx.countries.tags },
            product: 'player-apm',
            windowHours: TRAILING_HOURS,
            followQuery: apmHeadQuery(headParams, ctx.branch, TRAILING_HOURS),
            params: { ...headParams, registry: false, buckets: 1 },
            dynamicKeys: ['firstHour', 'lastHour', 'buckets'],
            validate: validateApm,
          },
        },
        {
          name: 'hover',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'exact-hover',
            product: 'player-apm',
            params: { ...headParams, registry: false, firstHour: null, lastHour: null, buckets: 1 },
            validate: validateApm,
          },
        },
        ...(ids.length > 0
          ? [{
              name: 'apm',
              source: ctx.track.source,
              branch: ctx.branch.id,
              offset: ctx.track.axisOffset,
              read: {
                shape: 'pixel-binned' as const,
                coverage: { mode: 'keyed' as const, keys: ids },
                family: {
                  kind: 'player-apm',
                  group: headParams.group,
                  tags: headParams.tags,
                  membership: headParams.membership,
                  players: headParams.players,
                  value: 'apm',
                },
                tags: ids,
                headQuery: null,
              },
            }]
          : []),
      ],
    };
  },
  projection: apmGraphProjection,
  unavailable: ctx => (ctx.aux && (!ctx.aux.available || ctx.aux.players.length === 0) ? NOT_RECORDED_CAMPAIGN : null),
  projectionOptions(ctx, { extent, surface }) {
    const group: 'country' | 'player' = ctx.options.group === 'country' ? 'country' : 'player';
    const yMin = typeof ctx.options.yMin === 'number' && Number.isFinite(ctx.options.yMin) ? ctx.options.yMin : null;
    const yMax = typeof ctx.options.yMax === 'number' && Number.isFinite(ctx.options.yMax) ? ctx.options.yMax : null;
    return {
      group,
      extent,
      surface,
      gaps: branchGapsOf(ctx.branch),
      yMin,
      yMax,
      available: ctx.aux ? ctx.aux.available : null,
      ...(ctx.aux ? { players: ctx.aux.players } : {}),
    };
  },
};
