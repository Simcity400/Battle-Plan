import { auditRow } from '../../families/auditRow.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView, BoundedSeries, FamilyPlanContext, FieldStatus, SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import { seriesLookAt, type ChartGap, type TimeChartModel, type TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import { branchGapsOf, countrySeriesLooks, missingSpansOf } from '../../families/time-chart/seriesHelpers.ts';
import type { ApmPlayer, ApmReply, ApmSeries, ApmValue } from '../../generated/dashboard';
import { APM_EMPTY, APM_NOT_RECORDED, apmRateRational, formatApmOperands, formatApmRate, GAPS_NOTE, playerNames, validateApm } from '../player-apm/apmData.ts';
import { apmHeadQuery, apmHeadRows } from '../player-apm/apmHead.ts';
import { identityName } from '../../families/table/identityCell.ts';

const TOP_NAMED = 8;

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

function unknownApmReason(value: ApmValue): string {
  if (!value.complete) return 'Player orders or unpaused time were not fully recorded for this period; the rate is unknown.';
  if (value.unpausedMinutesNum === '0' || value.unpausedMinutes === 0) return 'No unpaused time was observed in this period.';
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
  readonly summary: ApmValue | null;
  readonly gaps: boolean;
  readonly bounded: BoundedSeries | null;
}

function runIndexAt(runs: readonly { firstHour: number }[], hour: number): number {
  let lo = 0, hi = runs.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (runs[mid]!.firstHour <= hour) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

type PlayerRuns = Pick<ApmPlayer, 'id' | 'runs'>;
type NameChanges = Map<string, { firstHour: number; labels: Map<string, string> }[]>;
type PlayerRoster = Map<string, { runs: ApmPlayer['runs']; names: NameChanges }>;
type PlayerLookup = Pick<PlayerRoster, 'get'>;
const recordedRosters = new WeakMap<readonly ApmPlayer[], PlayerRoster>();
const noPlayers: readonly ApmPlayer[] = [];

function nameChangesFor(players: readonly PlayerRuns[], only?: ReadonlySet<string>): NameChanges {
  const byName = new Map<string, { hour: number; id: string; starts: boolean }[]>();
  for (const { id, runs } of players) {
    runs?.forEach((run, i) => {
      const end = Math.min(run.lastHour + 1, runs[i + 1]?.firstHour ?? Infinity);
      if (!run.name?.trim() || (only && !only.has(run.name)) || end <= run.firstHour) return;
      let changes = byName.get(run.name);
      if (!changes) byName.set(run.name, changes = []);
      changes.push({ hour: run.firstHour, id, starts: true }, { hour: end, id, starts: false });
    });
  }
  const index: NameChanges = new Map();
  for (const [name, changes] of byName) {
    if (changes.every(change => change.id === changes[0]!.id)) continue;
    changes.sort((a, b) => a.hour - b.hour || Number(a.starts) - Number(b.starts));
    const active = new Set<string>();
    const dates = [];
    for (let i = 0; i < changes.length;) {
      const firstHour = changes[i]!.hour;
      do {
        const change = changes[i++]!;
        if (change.starts) active.add(change.id);
        else active.delete(change.id);
      } while (changes[i]?.hour === firstHour);
      if (active.size > 1 || dates.at(-1)?.labels.size) {
        dates.push({ firstHour, labels: active.size > 1 ? playerNames(Array.from(active, id => ({ id, name }))) : new Map<string, string>() });
      }
    }
    if (dates.length) index.set(name, dates);
  }
  return index;
}

function datedName(index: NameChanges, id: string, name: string | null, hour: number) {
  const dates = name && index.get(name);
  if (!dates) return { name, until: Infinity };
  const i = runIndexAt(dates, hour);
  return { name: dates[i]?.labels.get(id) ?? name, until: (dates[i + 1]?.firstHour ?? Infinity) - 1 };
}

function playerLabel(player: string | null | undefined, country?: FieldStatus<string>): string {
  const name = player ?? 'Name not recorded';
  return country?.status === 'ready' && country.value ? `${name} \u00b7 ${country.value}` : name;
}

function recordedRoster(players: readonly ApmPlayer[]): PlayerRoster {
  const cached = recordedRosters.get(players);
  if (cached) return cached;
  const names = nameChangesFor(players);
  const roster: PlayerRoster = new Map(players.map(player => [player.id, { runs: player.runs, names }]));
  recordedRosters.set(players, roster);
  return roster;
}

function coversHour(runs: readonly { firstHour: number; lastHour: number }[] | undefined, hour: number): boolean {
  if (!runs?.length) return false;
  const run = runs[runIndexAt(runs, hour)];
  return Boolean(run && hour <= run.lastHour);
}

function withObservations(recorded: PlayerRoster, entries: readonly SeriesEntry[]): PlayerLookup {
  const uncovered = entries.filter(entry => {
    const runs = recorded.get(entry.id)?.runs;
    return !runs?.length || (entry.summary !== null && !coversHour(runs, entry.obsHour));
  });
  if (!uncovered.length) return recorded;
  const baseNames = recorded.values().next().value?.names;
  const observedRuns = new Map<string, ApmPlayer['runs']>();
  const affectedNames = new Set<string>();
  for (const { id, obsHour, tag, name } of uncovered) {
    const added = { firstHour: obsHour, lastHour: obsHour, tag: tag ?? '', name, nameMissing: null };
    const existing = recorded.get(id)?.runs;
    const runs = existing?.length ? [...existing, added].sort((a, b) => a.firstHour - b.firstHour) : [added];
    observedRuns.set(id, runs);
    if (name?.trim()) affectedNames.add(name);
  }
  let names: NameChanges = baseNames ?? new Map();
  if (affectedNames.size) {
    const affected: PlayerRuns[] = [];
    for (const [id, entry] of recorded) affected.push({ id, runs: observedRuns.get(id) ?? entry.runs });
    for (const [id, runs] of observedRuns) if (!recorded.has(id)) affected.push({ id, runs });
    const recomputed = nameChangesFor(affected, affectedNames);
    names = new Map(baseNames);
    for (const name of affectedNames) {
      const dates = recomputed.get(name);
      if (dates) names.set(name, dates);
      else names.delete(name);
    }
  }
  const observed: PlayerRoster = new Map();
  for (const [id, runs] of observedRuns) observed.set(id, { runs, names });
  if (!affectedNames.size) return { get: id => observed.get(id) ?? recorded.get(id) };
  return {
    get: id => {
      const held = observed.get(id);
      if (held) return held;
      const entry = recorded.get(id);
      if (!entry) return undefined;
      const merged = { runs: entry.runs, names };
      observed.set(id, merged);
      return merged;
    },
  };
}

function apmSeriesLooks(
  entries: readonly SeriesEntry[], roster: PlayerLookup | null, readView: BoundedReadView, options: ApmGraphOptions,
) {
  const ids = entries.map(entry => entry.id);
  const input = { from: options.extent[0], surface: options.surface, withFlag: true, holders: null };
  if (!roster) return countrySeriesLooks(ids, readView, input);

  const playerView = createBoundedReadView({
    hour: readView.hour,
    art: { resolve: handle => readView.art(handle) },
    identity: { resolve(kind, key, hour) {
      const { runs, names } = roster.get(key)!;
      const i = runIndexAt(runs, hour);
      const run = runs[i];
      const until = (runs[i + 1]?.firstHour ?? Infinity) - 1;
      if (!run || hour > run.lastHour || !run.tag) {
        const absent = { status: 'absent' as const, reason: 'Player country was not recorded at this hour.' };
        return { kind, key, atHour: hour, holder: absent, flagChoice: absent, name: { status: 'ready', value: 'Name not recorded' },
          validInterval: [hour, until], nextChangeAt: until + 1 };
      }
      const country = readView.who('country', run.tag, hour);
      const player = datedName(names, key, run.name, hour);
      const end = Math.min(until, run.lastHour, player.until, country.name.status === 'pending' || country.name.status === 'failed' ? Infinity : country.validInterval[1]);
      const label = playerLabel(player.name, country.name);
      return { ...country, name: { status: 'ready', value: label }, validInterval: [hour, end], nextChangeAt: end + 1 };
    } },
  });
  const looks = countrySeriesLooks(ids, playerView, input);
  return new Map([...looks].map(([id, look]) => {
    const { runs } = roster.get(id)!;
    const identities = look.identities.map(stretch => {
      const run = runs[runIndexAt(runs, stretch.firstHour)];
      const flag = run && stretch.firstHour <= run.lastHour && run.tag && stretch.flag
        ? { ...stretch.flag, tag: run.tag } : undefined;
      return { ...stretch, flag };
    });
    return [id, { ...look, identities }];
  }));
}

function exactPlayerNames(roster: PlayerLookup, rows: readonly ApmSeries[], hour: number, chartIds: Iterable<string>): Map<string, string> {
  const labels = new Map<string, string>();
  const unmatched: { id: string; name: string }[] = [];
  for (const { id, name } of rows) {
    if (!name) continue;
    const player = roster.get(id);
    const run = player?.runs?.[runIndexAt(player.runs, hour)];
    if (player && run && hour <= run.lastHour && run.name === name) {
      labels.set(id, datedName(player.names, id, name, hour).name ?? name);
    } else {
      unmatched.push({ id, name });
    }
  }
  if (!unmatched.length) return labels;
  const fromReply = playerNames(rows);
  const used = new Set(labels.values());
  const needed = new Set(unmatched.map(row => row.name));
  for (const id of chartIds) {
    const player = roster.get(id)!;
    const run = player.runs[runIndexAt(player.runs, hour)];
    if (run?.name && hour <= run.lastHour && needed.has(run.name)) {
      used.add(run.name);
      used.add(datedName(player.names, id, run.name, hour).name ?? run.name);
    }
  }
  const next = new Map<string, number>();
  for (const { id, name } of unmatched) {
    let label = fromReply.get(id) ?? name;
    while (used.has(label)) {
      const suffix = next.get(name) ?? 1;
      label = `${name} (${suffix})`;
      next.set(name, suffix + 1);
    }
    used.add(label);
    labels.set(id, label);
  }
  return labels;
}

function buildHoverDetail(
  readView: BoundedReadView,
  seriesById: ReadonlyMap<string, TimeChartSeriesModel>,
  hour: number,
  roster: PlayerLookup | null,
): TimeChartModel['hoverDetail'] {
  const hover = readView.hover<ApmReply>('hover');
  if (!hover || hover.hour > hour || !hover.data) return null;
  const reply = hover.data;
  if (!reply.available) return null;
  const items: TooltipListItem[] = [];
  const names = roster && exactPlayerNames(roster, reply.series, hover.hour, seriesById.keys());
  let anyGaps = false;
  for (const row of reply.series) {
    if (row.summary.gaps) anyGaps = true;
    const identity = row.tag ? readView.who('country', row.tag, hover.hour) : null;
    const flagChoice = identity?.flagChoice?.status === 'ready' ? identity.flagChoice.value : null;
    const flagRes = flagChoice ? readView.art(flagArtHandle(flagChoice, 'small')) : null;
    const label = names
      ? playerLabel(names.get(row.id) ?? row.name, identity?.name)
      : (row.tag ? identityName(readView, 'country', row.tag, hover.hour) : row.id);
    const series = seriesById.get(row.id);
    const swatch = series && seriesLookAt(series, hover.hour).color;
    const rateText = formatApmRate(row.summary, 0);
    items.push({
      label,
      ...(rateText !== null
        ? { value: rateText }
        : { missingReason: unknownApmReason(row.summary), tone: 'muted' as const }),
      ...(swatch ? { swatch } : {}),
      ...(flagRes?.status === 'ready' && flagRes.url ? { countryFlag: { flag: flagRes.url } } : {}),
    });
    const operands = formatApmOperands(row.summary);
    if (operands !== null) {
      items.push({ label: operands, indent: true, tone: 'muted' });
    }
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
    if (!panel.available) return { rows: [], ready: true, refusal: APM_NOT_RECORDED, warnings: panel.warnings };
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
        empty: { message: APM_NOT_RECORDED, kind: 'unavailable' },
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
    const recorded = options.group === 'player' ? recordedRoster(options.players ?? noPlayers) : null;
    const entries: SeriesEntry[] = [];
    const seen = new Set<string>();
    for (const row of rows) {
      seen.add(row.id);
      const obsHour = Number.isFinite(row.hour) ? Math.min(row.hour, hour) : hour;
      const atDisplayedHour = row.hour === hour;
      entries.push({
        id: row.id,
        name: row.name,
        tag: row.tag,
        obsHour,
        headValue: atDisplayedHour ? row.summary.apm : null,
        summary: atDisplayedHour ? row.summary : null,
        gaps: atDisplayedHour ? row.summary.gaps : false,
        bounded: tiled?.get(row.id) ?? null,
      });
    }
    if (tiled) {
      const historicalIds = [...tiled.keys()].filter(id => !seen.has(id)).sort();
      for (const id of historicalIds) {
        const bounded = tiled.get(id) ?? null;
        if (!hasRecordedByHour(bounded, hour)) continue;
        const reg = recorded?.get(id);
        const lastHour = bounded?.buckets.length
          ? Math.min(bounded.buckets.at(bounded.buckets.length - 1)?.lastHour ?? hour, hour)
          : hour;
        const run = reg?.runs?.[runIndexAt(reg.runs, lastHour)];
        const dated = options.group === 'country' ? { name: id, tag: id } : run && lastHour <= run.lastHour ? run : { name: null, tag: null };
        entries.push({
          id,
          name: dated.name,
          tag: dated.tag,
          obsHour: lastHour,
          headValue: null,
          summary: null,
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
        ...(ready ? { empty: { message: APM_EMPTY, kind: 'empty' as const } } : {}),
      };
    }
    const roster = recorded && withObservations(recorded, entries);
    const looks = apmSeriesLooks(entries, roster, readView, options);
    const headTexts = new Map<string, string>();
    const series: TimeChartSeriesModel[] = entries.map((entry, rank) => {
      const isTop = rank < TOP_NAMED;
      const held = looks.get(entry.id)!;
      const identities = held.identities;
      const look = seriesLookAt(held, entry.obsHour);
      const { color, flag } = look;
      let name = look.name;
      if (roster) {
        name = datedName(roster.get(entry.id)!.names, entry.id, entry.name, entry.obsHour).name
          ?? playerLabel(null, entry.tag ? readView.who('country', entry.tag, entry.obsHour).name : undefined);
      }
      const headText = entry.summary ? formatApmRate(entry.summary, 0) : null;
      if (headText !== null) headTexts.set(entry.id, headText);
      const headRat = entry.summary ? apmRateRational(entry.summary) : null;
      const exact = entry.headValue !== null
        ? {
            hour: entry.obsHour,
            value: entry.headValue,
            ...(headRat ? { raw: headRat.orders * headRat.minutesDen, denominator: headRat.minutesNum } : {}),
          }
        : undefined;
      return {
        id: entry.id,
        key: entry.id,
        name,
        color,
        identities,
        role: isTop ? 'primary' : 'tail',
        step: false,
        bounded: entry.bounded,
        headValue: entry.headValue,
        headHour: entry.obsHour,
        ...(exact ? { exact } : {}),
        ...(flag ? { flag } : {}),
        missingSpans: missingSpansOf(entry.bounded),
        rank,
        endLabel: isTop && headText !== null
          ? {
              ...(flag ? { flag } : {}),
              name,
              fullName: name,
              value: headText,
              ...(entry.gaps ? { valueMuted: true } : {}),
            }
          : null,
      };
    });
    const anyHeadGaps = entries.some(e => e.gaps);
    const hoverDetail = buildHoverDetail(readView, new Map(series.map(line => [line.id, line])), hour, roster);
    const auditRows = series
      .filter(s => s.role === 'primary' && headTexts.has(s.id))
      .map(s => (auditRow(s.id, headTexts.get(s.id)!)));
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

function selectedPlayers(ctx: FamilyPlanContext<ApmReply>): readonly ApmPlayer[] {
  const players = ctx.aux?.players ?? noPlayers;
  const wantedPlayers = new Set(contentSet(ctx.options.players).map(contentKey));
  if (ctx.countries.all && !wantedPlayers.size) return players;
  const tags = ctx.countries.all ? null : new Set(ctx.countries.tags);
  return players.filter(p => (!wantedPlayers.size || wantedPlayers.has(p.id)) && (!tags || p.tags.some(t => tags.has(t))));
}

function candidateSeriesIds(ctx: FamilyPlanContext<ApmReply>, group: 'country' | 'player'): string[] {
  const registry = ctx.aux;
  if (!registry || !registry.available) return [];
  if (group === 'player') return selectedPlayers(ctx).map(p => p.id);
  const wantedTags = ctx.countries.all ? null : new Set(ctx.countries.tags);
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
    allowSingleSlot: true,
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
    if (ctx.aux && !ctx.aux.available) {
      return {
        owner: `apm-graph:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
        entries: [],
        unavailable: APM_NOT_RECORDED,
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
            shape: 'point',
            channel: 'followed',
            coverage: ctx.countries.all ? { mode: 'population', form: 'all' } : { mode: 'keyed', keys: ctx.countries.tags },
            product: 'player-apm',
            hourParam: 'hour',
            followQuery: apmHeadQuery(headParams, ctx.branch, 1),
            params: { ...headParams, registry: false, firstHour: null, lastHour: null, buckets: 1 },
            dynamicKeys: ['hour'],
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
  unavailable: ctx => (ctx.aux && !ctx.aux.available ? APM_NOT_RECORDED : null),
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
      ...(ctx.aux && group === 'player' ? { players: selectedPlayers(ctx) } : {}),
    };
  },
};
