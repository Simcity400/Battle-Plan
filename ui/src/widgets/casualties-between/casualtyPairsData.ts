import { usNumber } from '../../charts/numberFormat.ts';
import { runAt, isOrderedSpans, spansCovered } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { object, opaqueTag, signed32, unsigned64, int } from '../../dashboard/validation.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CasualtyPairsReply as WireCasualtyPairsReply } from '../../generated/dashboard';

type PairSeries = WireCasualtyPairsReply['pairs'][number];
export type CasualtyPairsReply = Readonly<Omit<WireCasualtyPairsReply, 'pairs' | 'windows' | 'observed' | 'warnings'> & {
  pairs: ReadonlyArray<Readonly<Omit<PairSeries, 'runs'> & { runs: ReadonlyArray<Readonly<PairSeries['runs'][number]>> }>>;
  windows: ReadonlyArray<Readonly<Span>>;
  observed: ReadonlyArray<Readonly<Span>>;
  warnings: ReadonlyArray<string>;
}>;

const inRange = (value: bigint) => value >= -(1n << 127n) && value < (1n << 127n);
const amount = (value: unknown): value is string => typeof value === 'string'
  && value.length <= 40
  && /^-?(0|[1-9]\d*)$/.test(value)
  && (value.length - (value[0] === '-' ? 1 : 0) < 39 || inRange(BigInt(value)));
const append = (lists: Map<string, Readonly<Span>[]>, key: string, runs: ReadonlyArray<Readonly<Span>>) => {
  let list = lists.get(key);
  if (!list) lists.set(key, list = []);
  for (const run of runs) list.push(run);
};

export function validateCasualtyPairs(value: unknown): CasualtyPairsReply {
  const bad = () => { throw new Error('The casualty pairs reply is malformed.'); };
  if (!object(value) || !unsigned64(value.branchId) || typeof value.available !== 'boolean'
    || typeof value.hasGap !== 'boolean' || !isOrderedSpans(value.windows) || !isOrderedSpans(value.observed)
    || !(value.window === null || int(value.window) && value.window >= 0 && value.window < value.windows.length)
    || !Array.isArray(value.pairs) || !Array.isArray(value.warnings) || !value.warnings.every(w => typeof w === 'string')) return bad();
  if ([...value.windows, ...value.observed].some(span => !signed32(span.firstHour) || !signed32(span.lastHour))) return bad();
  const keys = new Set<string>();
  const endpoints = new Map<string, Readonly<Span>[]>();
  const identities = new Map<string, Readonly<Span>[]>();
  for (const pair of value.pairs) {
    if (!object(pair) || typeof pair.identity !== 'string' || !pair.identity
      || !opaqueTag(pair.firstCountry) || !opaqueTag(pair.secondCountry) || pair.firstCountry === pair.secondCountry
      || !isOrderedSpans(pair.runs) || !spansCovered(pair.runs, value.observed)) return bad();
    const key = JSON.stringify([pair.identity, pair.firstCountry, pair.secondCountry]);
    if (keys.has(key)) return bad();
    keys.add(key);
    append(endpoints, JSON.stringify([pair.firstCountry, pair.secondCountry].sort(compareTags)), pair.runs);
    append(identities, pair.identity, pair.runs);
    for (const run of pair.runs) {
      if (!signed32(run.firstHour) || !signed32(run.lastHour) || !signed32(run.readHour) || run.readHour > run.firstHour
        || typeof run.hourly !== 'boolean' || typeof run.incomplete !== 'boolean'
        || !amount(run.firstTaken) || !amount(run.secondTaken)) return bad();
      if (value.window !== null && !run.hourly) {
        const start = Math.max(run.readHour, value.windows[value.window].firstHour);
        if (!spansCovered([{ firstHour: start, lastHour: run.firstHour }], value.observed)) return bad();
      }
    }
  }
  for (const runs of [...endpoints.values(), ...identities.values()]) if (!isOrderedSpans(runs.sort((a, b) => a.firstHour - b.firstHour))) return bad();
  if (value.window === null && (value.pairs.length || value.observed.length)) return bad();
  if (value.window !== null) {
    const window = value.windows[value.window];
    if (value.observed.some(span => span.firstHour < window.firstHour || span.lastHour > window.lastHour)) return bad();
  }
  return value as unknown as CasualtyPairsReply;
}

export interface PairFlow { readonly id: string; readonly from: string; readonly to: string; readonly value: bigint; readonly readHour: number; readonly incomplete: boolean }
export interface PairSnapshot { readonly countries: ReadonlyArray<string>; readonly flows: ReadonlyArray<PairFlow> }

export const pairFlowId = (identity: string, direction: 'first' | 'second') => JSON.stringify([identity, direction]);

export function casualtyPairsAt(reply: CasualtyPairsReply, hour: number, includes: (tag: string) => boolean = () => true,
  between = false): PairSnapshot {
  return readCasualtyPairs(reply, hour, includes, between).snapshot;
}

function readCasualtyPairs(reply: CasualtyPairsReply, hour: number, includes: (tag: string) => boolean,
  between: boolean, pairs = reply.pairs): { snapshot: PairSnapshot; hourly: ReadonlySet<PairFlow> } {
  if (!runAt(reply.observed, hour)) return { snapshot: { countries: [], flows: [] }, hourly: new Set() };
  const flows: PairFlow[] = [];
  const countries = new Set<string>();
  const hourly = new Set<PairFlow>();
  for (const pair of pairs) {
    const first = includes(pair.firstCountry), second = includes(pair.secondCountry);
    if (!(between ? first && second : first || second)) continue;
    const run = runAt(pair.runs, hour);
    if (!run) continue;
    const read = run.hourly ? hour : run.readHour;
    countries.add(pair.firstCountry); countries.add(pair.secondCountry);
    flows.push({ id: pairFlowId(pair.identity, 'first'), from: pair.firstCountry,
      to: pair.secondCountry, value: BigInt(run.secondTaken), readHour: read, incomplete: run.incomplete });
    flows.push({ id: pairFlowId(pair.identity, 'second'), from: pair.secondCountry,
      to: pair.firstCountry, value: BigInt(run.firstTaken), readHour: read, incomplete: run.incomplete });
    if (run.hourly) { hourly.add(flows[flows.length - 2]); hourly.add(flows[flows.length - 1]); }
  }
  return { snapshot: { countries: [...countries].sort(compareTags), flows }, hourly };
}

interface PairSelection { readonly pairs: CasualtyPairsReply['pairs']; readonly hours: ReadonlyArray<number> }
const ALL_TAGS: ReadonlySet<string> = new Set();
const selections = new WeakMap<CasualtyPairsReply, WeakMap<ReadonlySet<string>, Map<boolean, PairSelection>>>();
function pairSelection(reply: CasualtyPairsReply, tags: ReadonlySet<string> | null, between: boolean): PairSelection {
  let byTags = selections.get(reply);
  if (!byTags) selections.set(reply, byTags = new WeakMap());
  const key = tags ?? ALL_TAGS;
  let byMode = byTags.get(key);
  if (!byMode) byTags.set(key, byMode = new Map());
  let selection = byMode.get(between);
  if (!selection) {
    const pairs = tags === null ? reply.pairs : reply.pairs.filter(pair => between
      ? tags.has(pair.firstCountry) && tags.has(pair.secondCountry) : tags.has(pair.firstCountry) || tags.has(pair.secondCountry));
    const edges = new Set<number>();
    for (const runs of [reply.observed, ...pairs.map(pair => pair.runs)]) for (const run of runs) {
      edges.add(run.firstHour); edges.add(run.lastHour + 1);
    }
    selection = { pairs, hours: [...edges].sort((a, b) => a - b) };
    byMode.set(between, selection);
  }
  return selection;
}
const geometry = new WeakMap<PairSnapshot, PairSnapshot>();
export const pairGeometry = (snapshot: PairSnapshot): PairSnapshot => geometry.get(snapshot) ?? snapshot;

export class CasualtyPairsCursor {
  private held?: Readonly<{ reply: CasualtyPairsReply; tags: ReadonlySet<string> | null; between: boolean;
    first: number; end: number; hourly: ReadonlySet<PairFlow>; hour: number; base: PairSnapshot; snapshot: PairSnapshot }>;

  at(reply: CasualtyPairsReply, hour: number, tags: ReadonlySet<string> | null = null, between = false): PairSnapshot {
    const held = this.held;
    const same = held !== undefined && held.reply === reply && held.between === between && held.tags === tags;
    if (same && hour >= held.first && hour < held.end) {
      if (!held.hourly.size || held.hour === hour) return held.snapshot;
      const snapshot = { countries: held.base.countries, flows: held.base.flows.map(flow => held.hourly.has(flow)
        ? { ...flow, readHour: hour } : flow) };
      geometry.set(snapshot, held.base);
      this.held = { ...held, hour, snapshot };
      return snapshot;
    }
    const { pairs, hours } = pairSelection(reply, tags, between);
    let first = 0, last = hours.length;
    while (first < last) {
      const middle = (first + last) >>> 1;
      if (hours[middle] <= hour) first = middle + 1;
      else last = middle;
    }
    const { snapshot, hourly } = readCasualtyPairs(reply, hour, () => true, between, pairs);
    const start = hours[first - 1] ?? -Infinity, end = hours[first] ?? Infinity;
    this.held = { reply, tags, between, first: start, end, hourly, hour, base: snapshot, snapshot };
    return snapshot;
  }
}

export const exactCount = (value: number | bigint) => usNumber.format(value);
export const pairAmount = (flow: Pick<PairFlow, 'value' | 'incomplete'>) =>
  `${flow.incomplete ? 'Recorded subtotal · ' : ''}${exactCount(flow.value)}`;

export function casualtyPairTags(replies: readonly (CasualtyPairsReply | undefined)[]): readonly string[] {
  const tags = new Set<string>();
  for (const reply of replies) if (reply) for (const pair of reply.pairs) { tags.add(pair.firstCountry); tags.add(pair.secondCountry); }
  return [...tags].sort(compareTags);
}

export function casualtyPairWindows(reply: CasualtyPairsReply | undefined, hour: number, following: boolean): number[] {
  const current = reply?.windows.findIndex(span => span.firstHour <= hour && hour <= span.lastHour) ?? -1;
  return current < 0 ? [] : following && current + 1 < reply!.windows.length ? [current, current + 1] : [current];
}
