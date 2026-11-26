import { bucketSamples } from '../../charts/chartSummary.ts';
import { colour } from '../../charts/palette.ts';
import { runAt } from '../../charts/series.ts';
import { fixedNumber } from '../../charts/valueAxis.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { CELL, cellState, cellValue, rowReason, type FollowedAnswer } from '../../dashboard/follow/message.ts';
import { followedRow, keyId, keyedTileParams, validateKeyedStates, type KeyedState, type KeyedStates, type SeriesKey } from '../../dashboard/keyedSeriesData.ts';
import type { Gate } from '../../dashboard/regionGate';
import { FutureDataError, type BoundedReadView, type BoundedSeries, type FamilyPlanContext, type SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartGap, ChartHoverDetail, ChartStackBand, ChartStackModel, TimeChartModel } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { knownValue, zeroValue } from '../../families/value.ts';
import type { HourSpan } from '../../generated/dashboard';
import { engineSum, partLayers, partName, partsAtStates, partsWithin, REMAINDER_LABEL, type PartLayer } from './partsData.ts';
import { formatPercent, type Measure } from './stabilityData.ts';
import type { FixedRun, MeasureParts, StabilityPart, StabilityPartName } from './types';

export const PARTS_FAMILY = { kind: 'stability-parts', measure: 'value' } as const;
export const STABILITY_PARTS_TABLE_KIND = 'stability-parts-table';
export const STABILITY_PARTS_CHART_KIND = 'stability-parts-chart';

export interface StabilitySeriesOwner {
  readonly key: string;
  readonly spans: readonly HourSpan[] | null;
}

export type StabilityNavKey = 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End';

export function buildStabilityNextChangeParams(input: {
  readonly branchId: string;
  readonly owner: StabilitySeriesOwner;
  readonly parts: readonly string[];
  readonly key: StabilityNavKey;
  readonly fromHour: number | null;
  readonly visible: readonly [number, number];
  readonly playhead: number | null;
  readonly gate: Pick<Gate, 'span' | 'within'>;
}): Readonly<Record<string, unknown>> | null {
  if (!input.parts.length) return null;
  const until = input.playhead === null ? input.visible[1] : Math.min(input.visible[1], input.playhead);
  const range = input.gate.span(input.visible[0], until);
  if (!range) return null;
  const keys: SeriesKey[] = input.parts.map(part => ({ tag: input.owner.key, part }));
  const within = input.owner.spans
    ? Object.fromEntries(keys.map(k => [keyId(k), input.owner.spans!]))
    : undefined;
  return {
    branchId: input.branchId,
    family: PARTS_FAMILY,
    ...keyedTileParams(keys, within, input.gate.span),
    key: input.key,
    from: input.gate.within(input.fromHour),
    ...range,
  };
}

const DECIMALS = 5;
const SCALE = 10 ** DECIMALS;
const LAYER_SLOTS = 12;

export const MEASURE_NAMES: Record<Measure, string> = {
  stability: 'Stability',
  warSupport: 'War support',
};

export interface StabilityDetailOptions {
  readonly ownerKey: string | null;
  readonly spans: readonly HourSpan[] | null;
  readonly parts: readonly string[];
  readonly partBits: number;
  readonly names: readonly StabilityPartName[];
  readonly namesReady: boolean;
  readonly warnings: readonly string[];
  readonly missingReasons?: readonly string[];
  readonly measure: Measure;
  readonly extent: readonly [number, number];
  readonly selectedHour?: number | null;
  readonly windowHours?: number | null;
  readonly surface: string;
  readonly gaps?: readonly ChartGap[];
}

const STABILITY_PARTS_COLUMNS: readonly TableColumnSpec[] = [
  {
    key: 'part',
    label: 'Source',
    priority: 1,
    as: 'text',
    offered: ['text'],
    grow: 1,
    minWidth: 160,
    sortable: false,
  },
  {
    key: 'value',
    label: 'Effect',
    priority: 2,
    as: 'faint',
    offered: ['faint'],
    numeric: true,
    barEligible: true,
    minWidth: 96,
    sortable: false,
    widest: ['+100.0%'],
  },
];

function partPalette(surface: string): string[] {
  const out: string[] = [];
  for (let i = 0; i < LAYER_SLOTS; i++) out.push(colour(surface, i));
  return out;
}

function namedOf(part: StabilityPart, measure: Measure, ready: boolean): { readonly name: string; readonly recorded: boolean } {
  if (ready || part.term || part.id === 'remainder' || part.id === 'clamp') return partName(part, measure);
  return { name: 'Not identified', recorded: false };
}

function namesMapOf(names: readonly StabilityPartName[]): Map<string, StabilityPartName> {
  return new Map(names.map(item => [item.part, item]));
}

function statesFromFollowed(
  answer: FollowedAnswer,
  ownerKey: string,
  parts: readonly string[],
  partBits: number,
  hour: number,
): KeyedState[] {
  if (answer.kind === 'notRecorded') {
    return answer.reason
      ? [{ tag: ownerKey, part: parts[0] ?? '', hour, drop: null, missing: answer.reason, run: null }]
      : [];
  }
  const out: KeyedState[] = [];
  for (const part of parts) {
    const row = followedRow(answer, { tag: ownerKey, part }, partBits);
    if (row < 0 || row >= answer.rows) continue;
    if (cellState(answer, row * answer.columns) !== CELL.value) {
      const missing = rowReason(answer, row);
      if (missing) out.push({ tag: ownerKey, part, hour, drop: null, missing, run: null });
      continue;
    }
    const raw = cellValue(answer, row * answer.columns);
    out.push({
      tag: ownerKey,
      part,
      hour,
      drop: null,
      missing: null,
      run: {
        firstHour: hour,
        lastHour: hour,
        valueRaw: raw.toString(),
        slot: 0,
        countryId: 0,
        startsSegment: false,
        segmentFirstHour: hour,
        firstExact: true,
        lastExact: true,
      },
    });
  }
  return out;
}

function percentOf(raw: string): number {
  return fixedNumber(raw, DECIMALS) * 100;
}

function tableMissingReason(states: readonly KeyedState[] | null, options: StabilityDetailOptions): string | null {
  const reasons: string[] = [];
  if (states && options.ownerKey) {
    for (const state of states) {
      if (state.tag === options.ownerKey && state.missing) reasons.push(state.missing);
    }
  }
  if (!reasons.length && options.missingReasons?.length) reasons.push(...options.missingReasons);
  const unique = [...new Set(reasons)];
  return unique.length ? unique.join(' · ') : null;
}

export function buildStabilityPartsTableModel(
  hour: number,
  states: readonly KeyedState[] | null,
  options: StabilityDetailOptions,
): TableModel {
  const base: TableModel = {
    columns: STABILITY_PARTS_COLUMNS,
    rows: [],
    hour,
    unsorted: true,
    hideHeader: false,
    ...(options.warnings.length ? { warnings: options.warnings } : {}),
  };
  if (!options.ownerKey || !options.parts.length || states === null) {
    return { ...base, empty: tableMissingReason(states, options) ?? 'No breakdown was recorded for this date.', emptyKind: 'unavailable' };
  }
  const names = namesMapOf(options.names);
  const filtered = states.filter(state => state.tag === options.ownerKey);
  const measured = partsAtStates(options.parts, filtered, hour, names);
  if (!measured.observed.length) {
    return { ...base, empty: tableMissingReason(filtered, options) ?? 'No breakdown was recorded for this date.', emptyKind: 'unavailable' };
  }
  const active = measured.parts
    .map(part => ({ part, raw: part.runs[0]!.valueRaw }))
    .filter(item => item.raw !== '0');
  const sum = engineSum(active.map(item => item.raw));
  const nonRemainder = active.filter(item => item.part.id !== 'remainder');
  const remainder = active.find(item => item.part.id === 'remainder');
  nonRemainder.sort((a, b) => {
    const da = Math.abs(percentOf(a.raw));
    const db = Math.abs(percentOf(b.raw));
    if (db !== da) return db - da;
    return a.part.id.localeCompare(b.part.id);
  });
  const maxAbs = active.reduce((best, item) => Math.max(best, Math.abs(percentOf(item.raw))), 0);
  const makeRow = (id: string, label: string, raw: string, muted: boolean): TableRow => {
    const pct = percentOf(raw);
    const formatted = formatPercent(raw, DECIMALS, true);
    const fraction = maxAbs > 0 ? Math.abs(pct) / maxAbs : 0;
    return {
      key: id,
      name: label,
      muted,
      cells: {
        part: { value: knownValue(label), ...(muted ? { tone: 'muted' } : {}) },
        value: {
          value: pct === 0 ? zeroValue() : knownValue(formatted),
          sortValue: knownValue(Math.abs(pct)),
          ...(pct < 0 ? { tone: 'negative' } : {}),
          bar: maxAbs > 0 ? { fraction: knownValue(fraction), alert: pct < 0 } : null,
        },
      },
      tooltipTitle: label,
    };
  };
  const rows: TableRow[] = nonRemainder.map(item => {
    const label = namedOf(item.part, options.measure, options.namesReady);
    return makeRow(item.part.id, label.name, item.raw, label.recorded);
  });
  if (remainder) {
    rows.push(makeRow('remainder', REMAINDER_LABEL, remainder.raw, false));
  }
  const totalText = formatPercent(sum.toString(), DECIMALS);
  const totalRow: TableRow = {
    key: '__total__',
    name: 'Total',
    cells: {
      part: { value: knownValue('Total'), strong: true },
      value: { value: knownValue(totalText), strong: true, bar: null },
    },
  };
  return {
    ...base,
    rows,
    totalRow,
    ...(!options.namesReady ? { endNote: 'Loading names…' } : {}),
    ...(rows.length === 0 ? { empty: 'No active modifiers at this date.' } : {}),
  };
}

export const stabilityPartsTableProjection: TableProjection<StabilityDetailOptions> = registerFamilyProjection({
  kind: STABILITY_PARTS_TABLE_KIND,
  family: 'table',
  project(readView: BoundedReadView, options: StabilityDetailOptions): TableModel {
    const hour = readView.hour;
    const answer = readView.answer('state');
    if (answer && options.ownerKey) {
      return buildStabilityPartsTableModel(hour, statesFromFollowed(answer, options.ownerKey, options.parts, options.partBits, hour), options);
    }
    const panel = readView.panel<KeyedStates>('state');
    if (panel) {
      return buildStabilityPartsTableModel(hour, panel.states, options);
    }
    return buildStabilityPartsTableModel(hour, null, options);
  },
});

export const stabilityPartsTableDeclaration: TableWidgetDeclaration<StabilityDetailOptions> = {
  family: 'table',
  kind: STABILITY_PARTS_TABLE_KIND,
  label: 'Stability breakdown',
  subject: 'stability breakdown',
  emptyCampaign: 'Load a campaign to see stability breakdown.',
  countries: { mode: 'none' },
  columns: STABILITY_PARTS_COLUMNS,
  settings: {
    kind: 'fixed',
    base: { density: 'compact', barsBehind: 'all', sort: { column: 'value', direction: 'descending' } },
  },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const opts = ctx.options as unknown as StabilityDetailOptions;
    const owner = `stability-parts-table:${ctx.track.source.id}:${ctx.branch.id}:${opts.ownerKey ?? ''}:${opts.measure}:${opts.parts.join(',')}`;
    if (!opts.ownerKey || !opts.parts.length) {
      return { owner, entries: [], allowEmptyFollow: true };
    }
    const country = countryFromKey(opts.ownerKey);
    return {
      owner,
      entries: [{
        name: 'state',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: { mode: 'keyed', keys: [opts.ownerKey] },
          product: 'series-state',
          followQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family: PARTS_FAMILY, reads: [{ country }] } },
          params: { branchId: ctx.branch.id, family: PARTS_FAMILY },
          buildExactParams: (_base, hour) => ({
            branchId: ctx.branch.id,
            family: PARTS_FAMILY,
            reads: opts.parts.map(part => ({ country, part, hour })),
          }),
          validate: validateKeyedStates,
        },
      }],
    };
  },
  projection: stabilityPartsTableProjection,
  projectionOptions(ctx) {
    return ctx.options as unknown as StabilityDetailOptions;
  },
};

function addRun(runs: FixedRun[], run: FixedRun) {
  const previous = runs.at(-1);
  if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.valueRaw === run.valueRaw) {
    previous.lastHour = run.lastHour;
  } else {
    runs.push(run);
  }
}

function boundedToRuns(series: BoundedSeries, until: number, headHour: number): { readonly runs: FixedRun[]; readonly reduced: boolean } {
  const runs: FixedRun[] = [];
  let reduced = false;
  if (series.runs && series.runs.length > 0) {
    for (let i = 0; i < series.runs.length; i++) {
      const run = series.runs.at(i)!;
      if (run.firstHour > until) break;
      addRun(runs, {
        firstHour: run.firstHour,
        lastHour: Math.min(run.lastHour, until),
        valueRaw: String(Math.round(run.value * SCALE)),
        startsSegment: run.breakBefore === true,
      });
    }
  } else {
    for (let i = 0; i < series.buckets.length; i++) {
      const bucket = series.buckets.at(i);
      if (!bucket || bucket.firstHour > until) continue;
      if (series.binHours > 1 || bucket.lastHour > bucket.firstHour) reduced = true;
      if (i === series.buckets.closedLength && series.straddling === 'cut' && series.straddlingRuns?.length) {
        for (const run of series.straddlingRuns) {
          if (run.firstHour > until) break;
          addRun(runs, {
            firstHour: run.firstHour,
            lastHour: Math.min(run.lastHour, until),
            valueRaw: String(Math.round(run.value * SCALE)),
            startsSegment: run.breakBefore === true,
          });
        }
        continue;
      }
      const end = Math.min(bucket.lastHour, until);
      const samples = bucketSamples(bucket, until);
      if (bucket.hasBreak) {
        for (const p of samples) {
          addRun(runs, {
            firstHour: p.hour,
            lastHour: p.hour,
            valueRaw: String(Math.round(p.value * SCALE)),
            startsSegment: true,
          });
        }
        continue;
      }
      for (let s = 0; s < samples.length; s++) {
        const cur = samples[s]!;
        const next = samples[s + 1];
        const segLast = next ? next.hour - 1 : end;
        if (segLast < cur.hour) continue;
        addRun(runs, {
          firstHour: cur.hour,
          lastHour: segLast,
          valueRaw: String(Math.round(cur.value * SCALE)),
          startsSegment: s === 0 && bucket.firstBreak,
        });
      }
    }
  }
  const head = headHour === until ? series.headValue : undefined;
  if (head && (head.kind === 'known' || head.kind === 'zero' || head.kind === 'partial')) {
    const val = Number(head.value);
    if (Number.isFinite(val)) {
      const prev = runs.at(-1);
      const from = prev ? Math.max(prev.lastHour + 1, until) : until;
      if (from <= until) {
        addRun(runs, {
          firstHour: from,
          lastHour: until,
          valueRaw: String(Math.round(val * SCALE)),
          startsSegment: !prev || prev.lastHour + 1 !== from,
        });
      }
    }
  }
  return { runs, reduced };
}

function mergeSpans(spans: readonly HourSpan[]): HourSpan[] {
  if (!spans.length) return [];
  const sorted = [...spans].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  const out: HourSpan[] = [{ firstHour: sorted[0]!.firstHour, lastHour: sorted[0]!.lastHour }];
  for (let i = 1; i < sorted.length; i++) {
    const cur = sorted[i]!;
    const prev = out.at(-1)!;
    if (cur.firstHour <= prev.lastHour + 1) prev.lastHour = Math.max(prev.lastHour, cur.lastHour);
    else out.push({ firstHour: cur.firstHour, lastHour: cur.lastHour });
  }
  return out;
}

function subtractRuns(firstHour: number, lastHour: number, runs: readonly FixedRun[], out: HourSpan[]): void {
  if (firstHour > lastHour) return;
  let cursor = firstHour;
  for (const run of runs) {
    if (run.lastHour < cursor) continue;
    if (run.firstHour > lastHour) break;
    if (run.firstHour > cursor) out.push({ firstHour: cursor, lastHour: run.firstHour - 1 });
    cursor = Math.max(cursor, Math.min(run.lastHour, lastHour) + 1);
    if (cursor > lastHour) break;
  }
  if (cursor <= lastHour) out.push({ firstHour: cursor, lastHour });
}

function unresolvedOpenIntervals(series: BoundedSeries, runs: readonly FixedRun[], until: number): HourSpan[] {
  const out: HourSpan[] = [];
  for (let i = 0; i < series.missing.length; i++) {
    const span = series.missing.at(i);
    if (!span || span.firstHour > until) continue;
    out.push({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, until) });
  }
  if (!series.runs || !series.runs.length) {
    for (let i = 0; i < series.buckets.length; i++) {
      const bucket = series.buckets.at(i);
      if (!bucket || !bucket.hasBreak || bucket.firstHour > until) continue;
      subtractRuns(bucket.firstHour, Math.min(bucket.lastHour, until), runs, out);
    }
  }
  if (series.binHours > 1 && series.straddling !== 'none') {
    const binStart = series.from + Math.floor((until - series.from) / series.binHours) * series.binHours;
    const openStart = series.straddlingFrom ?? binStart;
    subtractRuns(openStart, until, runs, out);
  }
  return out;
}

function overlapsWithin(firstHour: number, lastHour: number, within: readonly HourSpan[] | null): boolean {
  if (!within) return true;
  return within.some(span => Math.max(firstHour, span.firstHour) <= Math.min(lastHour, span.lastHour));
}

function measurePartsFromSeries(
  ownerKey: string,
  parts: readonly string[],
  seriesMap: ReadonlyMap<string, BoundedSeries>,
  names: ReadonlyMap<string, StabilityPartName>,
  from: number,
  until: number,
  headHour: number,
  within: readonly HourSpan[] | null,
): {
  readonly measured: MeasureParts;
  readonly unresolvedById: ReadonlyMap<string, readonly HourSpan[]>;
  readonly missingReasons: readonly string[];
  readonly reduced: boolean;
  readonly anyLoading: boolean;
} {
  const observed: HourSpan[] = [];
  const byId = new Map<string, StabilityPart>();
  const unresolvedRaw = new Map<string, HourSpan[]>();
  const missingReasons: string[] = [];
  let reduced = false;
  let anyLoading = false;
  for (const part of parts) {
    const series = seriesMap.get(keyId({ tag: ownerKey, part }));
    if (!series) continue;
    if (series.straddling === 'loading') anyLoading = true;
    for (let i = 0; i < series.missing.length; i++) {
      const span = series.missing.at(i);
      if (!span || span.firstHour > until || span.lastHour < from) continue;
      const first = Math.max(span.firstHour, from);
      const last = Math.min(span.lastHour, until);
      if (first <= last && span.reason && overlapsWithin(first, last, within)) {
        missingReasons.push(span.reason);
      }
    }
    const { runs, reduced: partReduced } = boundedToRuns(series, until, headHour);
    if (partReduced) reduced = true;
    const meta = names.get(part);
    if (meta?.kind === 'total') {
      for (const run of runs) {
        const last = observed.at(-1);
        if (last && !run.startsSegment && last.lastHour + 1 === run.firstHour) last.lastHour = run.lastHour;
        else observed.push({ firstHour: run.firstHour, lastHour: run.lastHour });
      }
      continue;
    }
    const id = meta?.id ?? `part:${part}`;
    let held = byId.get(id);
    if (!held) {
      held = { id, term: meta?.term ?? null, line: meta?.line ?? null, runs: [] };
      byId.set(id, held);
    }
    for (const run of runs) addRun(held.runs, run);
    const missingSpans = unresolvedOpenIntervals(series, runs, until);
    if (missingSpans.length) {
      let bucket = unresolvedRaw.get(id);
      if (!bucket) unresolvedRaw.set(id, bucket = []);
      bucket.push(...missingSpans);
    }
  }
  for (const part of byId.values()) part.runs.sort((a, b) => a.firstHour - b.firstHour);
  const unresolvedById = new Map<string, readonly HourSpan[]>();
  for (const [id, spans] of unresolvedRaw) unresolvedById.set(id, mergeSpans(spans));
  return {
    measured: { parts: [...byId.values()], observed: observed.sort((a, b) => a.firstHour - b.firstHour) },
    unresolvedById,
    missingReasons: [...new Set(missingReasons)],
    reduced,
    anyLoading,
  };
}

function buildStackFromParts(
  parts: MeasureParts,
  layers: readonly PartLayer[],
  from: number,
  to: number,
  unresolvedById: ReadonlyMap<string, readonly HourSpan[]>,
): ChartStackModel | undefined {
  if (!parts.observed.length || !layers.length) return undefined;
  const layerOf = new Map<string, number>();
  layers.forEach((layer, index) => {
    for (const member of layer.members) layerOf.set(member, index);
  });
  const events: { readonly hour: number; readonly row: number; readonly delta: bigint }[] = [];
  const cutSet = new Set<number>();
  const spans: { readonly first: number; readonly last: number }[] = [];
  const lo = Math.floor(from);
  const hi = Math.floor(to);
  for (const span of parts.observed) {
    const first = Math.max(span.firstHour, lo);
    const last = Math.min(span.lastHour, hi);
    if (first > last) continue;
    spans.push({ first, last });
    cutSet.add(first);
    if (last + 1 <= hi) cutSet.add(last + 1);
  }
  if (!spans.length) return undefined;
  const layerUnresolved: HourSpan[][] = layers.map(layer => {
    const collected: HourSpan[] = [];
    for (const member of layer.members) {
      const memberSpans = unresolvedById.get(member);
      if (!memberSpans) continue;
      for (const span of memberSpans) {
        const firstHour = Math.max(span.firstHour, lo);
        const lastHour = Math.min(span.lastHour, hi);
        if (firstHour > lastHour) continue;
        collected.push({ firstHour, lastHour });
      }
    }
    const merged = mergeSpans(collected);
    for (const span of merged) {
      cutSet.add(span.firstHour);
      if (span.lastHour + 1 <= hi) cutSet.add(span.lastHour + 1);
    }
    return merged;
  });
  for (const part of parts.parts) {
    const layer = layerOf.get(part.id);
    if (layer === undefined) continue;
    for (const run of part.runs) {
      if (run.lastHour < from || run.firstHour > to || run.valueRaw === '0') continue;
      const raw = BigInt(run.valueRaw);
      const row = layer * 2 + (raw > 0n ? 0 : 1);
      const first = Math.max(run.firstHour, lo);
      const end = Math.min(run.lastHour, hi) + 1;
      cutSet.add(first);
      if (end <= hi) cutSet.add(end);
      events.push({ hour: first, row, delta: raw });
      events.push({ hour: end, row, delta: -raw });
    }
  }
  const hours = [...cutSet].sort((a, b) => a - b);
  events.sort((a, b) => a.hour - b.hour);
  const running = new Array<bigint>(layers.length * 2).fill(0n);
  const rowValues: (number | null)[][] = running.map(() => new Array<number | null>(hours.length).fill(null));
  const rowActive = new Array<boolean>(running.length).fill(false);
  const unresolvedIdx = new Array<number>(layers.length).fill(0);
  let spanIdx = 0;
  let evIdx = 0;
  for (let i = 0; i < hours.length; i++) {
    const h = hours[i]!;
    while (evIdx < events.length && events[evIdx]!.hour <= h) {
      const ev = events[evIdx++]!;
      running[ev.row] = running[ev.row]! + ev.delta;
    }
    while (spanIdx < spans.length && spans[spanIdx]!.last < h) spanIdx++;
    const inSpan = spanIdx < spans.length && spans[spanIdx]!.first <= h && h <= spans[spanIdx]!.last;
    if (!inSpan) continue;
    for (let l = 0; l < layers.length; l++) {
      const uSpans = layerUnresolved[l]!;
      let uIdx = unresolvedIdx[l]!;
      while (uIdx < uSpans.length && uSpans[uIdx]!.lastHour < h) uIdx++;
      unresolvedIdx[l] = uIdx;
      if (uIdx < uSpans.length && uSpans[uIdx]!.firstHour <= h && h <= uSpans[uIdx]!.lastHour) continue;
      for (let side = 0; side < 2; side++) {
        const r = l * 2 + side;
        const v = Number(running[r]!) / SCALE * 100;
        rowValues[r]![i] = v;
        if (v !== 0) rowActive[r] = true;
      }
    }
  }
  const bands: ChartStackBand[] = [];
  for (let l = 0; l < layers.length; l++) {
    const layer = layers[l]!;
    const posRow = l * 2;
    const negRow = l * 2 + 1;
    if (rowActive[posRow]) {
      bands.push({ id: layer.id, key: layer.id, name: layer.name, color: layer.color, values: rowValues[posRow]! });
    }
    if (rowActive[negRow]) {
      const id = rowActive[posRow] ? `${layer.id}:neg` : layer.id;
      bands.push({ id, key: id, name: layer.name, color: layer.color, values: rowValues[negRow]! });
    }
  }
  return bands.length ? { hours, bands, step: true, bandFormat: 'signedPercent' } : undefined;
}

function stabilityChartHoverDetail(
  readView: BoundedReadView,
  options: StabilityDetailOptions,
  names: ReadonlyMap<string, StabilityPartName>,
  colorOf: ReadonlyMap<string, string>,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<KeyedStates>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover?.data || !options.ownerKey) return null;
  const at = hover.hour;
  if (options.spans && !runAt(options.spans, at)) return null;
  const measured = partsAtStates(
    options.parts,
    hover.data.states.filter(state => state.tag === options.ownerKey),
    at,
    names,
  );
  if (!measured.observed.length) {
    return {
      hour: at,
      items: [{ label: MEASURE_NAMES[options.measure], missingReason: 'Not recorded', tone: 'muted' }],
      replacesSeries: true,
    };
  }
  const known = measured.parts
    .map(part => ({ part, raw: part.runs[0]!.valueRaw }))
    .filter(item => item.raw !== '0')
    .sort((a, b) => Math.abs(percentOf(b.raw)) - Math.abs(percentOf(a.raw)));
  const items: TooltipListItem[] = known.map(item => ({
    label: namedOf(item.part, options.measure, options.namesReady).name,
    swatch: colorOf.get(item.part.id),
    value: formatPercent(item.raw, DECIMALS, true),
  }));
  items.push({
    label: 'Total',
    value: formatPercent(engineSum(known.map(item => item.raw)).toString(), DECIMALS),
    strong: true,
    divider: items.length > 0,
  });
  return { hour: at, items, replacesSeries: true };
}

export function buildStabilityPartsChartModel(
  readView: BoundedReadView,
  options: StabilityDetailOptions,
): TimeChartModel {
  const hour = Math.min(readView.hour, options.extent[1]);
  const measureTitle = `${MEASURE_NAMES[options.measure]} parts over time`;
  const base: TimeChartModel = {
    kind: STABILITY_PARTS_CHART_KIND,
    measureTitle,
    mode: 'stack',
    hour,
    selectedHour: options.selectedHour ?? null,
    extent: options.extent,
    ...(options.windowHours ? { view: { x: [Math.max(options.extent[0], hour - options.windowHours), hour] as const } } : {}),
    yAxis: { format: 'signedPercent' },
    series: [],
    ...(options.gaps?.length ? { gaps: options.gaps } : {}),
    ...(options.warnings.length ? { warnings: options.warnings } : {}),
  };
  if (!options.ownerKey || !options.parts.length) {
    if (options.missingReasons?.length) {
      return { ...base, empty: { message: [...new Set(options.missingReasons)].join(' · '), kind: 'unavailable' } };
    }
    return { ...base, empty: { message: 'No breakdown recorded in this time.', kind: 'empty' } };
  }
  const seriesMap = readView.series('parts');
  if (!seriesMap) return base;
  const names = namesMapOf(options.names);
  const displayedEnd = Math.min(options.extent[1], hour);
  const { measured: rawParts, unresolvedById, missingReasons: tileMissingReasons, reduced, anyLoading } = measurePartsFromSeries(
    options.ownerKey,
    options.parts,
    seriesMap,
    names,
    options.extent[0],
    displayedEnd,
    readView.hour,
    options.spans,
  );
  const shown = partsWithin(partsWithin(rawParts, [{ firstHour: options.extent[0], lastHour: options.extent[1] }]), options.spans);
  const palette = partPalette(options.surface);
  const muted = colour(options.surface, LAYER_SLOTS);
  const layers = partLayers(shown, options.measure, palette, muted);
  const colorOf = new Map<string, string>(layers.flatMap(layer => layer.members.map(id => [id, layer.color] as const)));
  const built = buildStackFromParts(shown, layers, options.extent[0], displayedEnd, unresolvedById);
  const stack = built ? { ...built, ...(reduced ? { reduced: true, bucketed: true } : {}) } : undefined;
  const hoverDetail = stabilityChartHoverDetail(readView, options, names, colorOf);
  const allMissingReasons = [...new Set([...tileMissingReasons, ...(options.missingReasons ?? [])])];
  const empty = shown.observed.length > 0
    ? { message: 'No active modifiers in this time.', kind: 'empty' as const }
    : allMissingReasons.length > 0
      ? { message: allMissingReasons.join(' · '), kind: 'unavailable' as const }
      : { message: 'No breakdown recorded in this time.', kind: 'empty' as const };
  return {
    ...base,
    ...(stack ? { stack } : anyLoading ? {} : { empty }),
    ...(hoverDetail ? { hoverDetail } : {}),
  };
}

export const stabilityPartsChartProjection: TimeChartProjection<StabilityDetailOptions> = registerFamilyProjection({
  kind: STABILITY_PARTS_CHART_KIND,
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'step',
  measureTitle: 'Stability parts over time',
  project: buildStabilityPartsChartModel,
});

export const stabilityPartsChartDeclaration: TimeChartWidgetDeclaration<StabilityDetailOptions> = {
  family: 'time-chart',
  kind: STABILITY_PARTS_CHART_KIND,
  label: 'Stability parts over time',
  subject: 'stability parts',
  emptyCampaign: 'Load a campaign to see stability parts.',
  countries: { mode: 'none' },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const opts = ctx.options as unknown as StabilityDetailOptions;
    const owner = `stability-parts-chart:${ctx.track.source.id}:${ctx.branch.id}:${opts.ownerKey ?? ''}:${opts.measure}:${opts.parts.join(',')}`;
    if (!opts.ownerKey || !opts.parts.length) {
      return { owner, entries: [], allowEmptyFollow: true };
    }
    const country = countryFromKey(opts.ownerKey);
    const keys: SeriesKey[] = opts.parts.map(part => ({ tag: opts.ownerKey!, part }));
    const within = opts.spans ? new Map<string, readonly HourSpan[]>(keys.map(k => [keyId(k), opts.spans!])) : undefined;
    const reads = keys.map(k => ({ country, part: k.part }));
    const hoverMembership = opts.spans
      ? { [opts.ownerKey]: opts.spans.map(span => [span.firstHour, span.lastHour] as const) }
      : undefined;
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    return {
      owner,
      entries: [
        {
          name: 'parts',
          ...common,
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'keyed', keys: keys.map(keyId) },
            family: PARTS_FAMILY,
            tags: [opts.ownerKey],
            keys,
            partBits: opts.partBits,
            ...(within ? { within } : {}),
            headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family: PARTS_FAMILY, reads: [{ country }] } },
            params: { branchId: ctx.branch.id, family: PARTS_FAMILY, reads },
            exactProduct: 'series-state',
          },
        },
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'series-state',
            params: { branchId: ctx.branch.id, family: PARTS_FAMILY, reads },
            ...(hoverMembership ? { membership: hoverMembership } : {}),
            validate: validateKeyedStates,
          },
        },
      ],
    };
  },
  projection: stabilityPartsChartProjection,
  projectionOptions(ctx, { extent, surface }) {
    const opts = ctx.options as unknown as StabilityDetailOptions;
    return {
      ...opts,
      extent: opts.extent ?? extent,
      surface,
      gaps: branchGapsOf(ctx.branch),
    };
  },
};
