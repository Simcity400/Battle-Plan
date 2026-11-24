import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useGate } from './regionGate.ts';
import { SPEC, UNRECORDED_MESSAGE } from './regionTime.ts';
import { useProduct } from './products.ts';
import { drawableIcon } from './artwork.ts';
import { compareTags } from './tagOrder.ts';
import { isOrderedSpans, runAt } from '../charts/series.ts';
import type { Span } from '../charts/series';
import type { RaidEndAt, RaidEpisode, RaidLogReply, RaidPreparationRun, RaidVictimAt } from '../generated/dashboard';
import { productRequest } from './request.ts';
import { countryKey } from './countryKey.ts';
import { gatedRead, readData } from './gatedRead.ts';

export const RAIDS_AVAILABILITY = { product: 'raid-log', params: { catalogue: true } } as const;

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const integer = (value: unknown) => Number.isSafeInteger(value);
const maybe = (value: unknown, test: (value: unknown) => boolean) => value === null || test(value);
const text = (value: unknown) => typeof value === 'string';
const victimSource = (value: unknown) => value === 'completionCountry' || value === 'fixedTarget' || value === 'controllerAtFirstEndedRead'
  || value === 'controllerAtObservedRead' || value === 'captorAtFirstEndedRead' || value === 'captorAtObservedRead';
const endedSource = (value: unknown) => value === 'nativeEndDate' || value === 'observedEndedRead';
const preparationRuns = (runs: unknown): runs is RaidPreparationRun[] => Array.isArray(runs) && runs.every((run, index) =>
  Array.isArray(run) && run.length === 4 && run.every(integer) && run[0] <= run[1] && (index === 0 || runs[index - 1][1] < run[0]));
const victimObservations = (items: unknown): items is RaidVictimAt[] => Array.isArray(items) && items.length > 0 && items.every((item, index) =>
  object(item) && integer(item.hour) && (index === 0 || (items[index - 1] as { hour: number }).hour < (item.hour as number))
  && maybe(item.tag, text) && maybe(item.missing, text) && (item.tag === null) !== (item.missing === null)
  && maybe(item.source, victimSource) && (item.tag !== null || item.source === null));
const endObservations = (items: unknown): items is RaidEndAt[] => Array.isArray(items) && items.every((item, index) =>
  object(item) && integer(item.hour) && (index === 0 || (items[index - 1] as { hour: number }).hour < (item.hour as number))
  && integer(item.endedHour) && typeof item.exact === 'boolean' && endedSource(item.source) && maybe(item.raider, text));

export function validateRaidLog(value: unknown): RaidLogReply {
  const ok = object(value) && Array.isArray(value.episodes) && Array.isArray(value.missing) && Array.isArray(value.warnings)
    && maybe(value.available, item => typeof item === 'boolean') && maybe(value.unavailable, text)
    && [value.phasePreparing, value.phaseEnded, value.outcomeNone, value.outcomeFailure, value.outcomeLimitedSuccess, value.outcomeSuccess, value.outcomeCriticalSuccess, value.outcomeCanceled].every(integer)
    && Array.isArray(value.types) && value.types.every(type => object(type) && text(type.token) && maybe(type.name, text) && maybe(type.missing, text))
    && Array.isArray(value.artwork) && value.artwork.every(art => object(art) && text(art.key) && maybe(art.image, text) && maybe(art.missing, text))
    && value.warnings.every(text) && value.missing.every(span => object(span) && integer(span.firstHour) && integer(span.lastHour) && text(span.reason))
    && value.episodes.every(item => object(item) && text(item.id) && integer(item.firstHour) && typeof item.firstHourExact === 'boolean' && integer(item.lastHour) && (item.firstHour as number) <= (item.lastHour as number)
      && maybe(item.raider, text) && maybe(item.raiderSlot, integer) && maybe(item.launchedRaider, text)
      && maybe(item.victim, text) && (item.victim === null) !== (item.victimMissing === null)
      && maybe(item.victimSource, victimSource) && (item.victim !== null || item.victimSource === null)
      && victimObservations(item.victims)
      && maybe(item.raidTypeHour, integer) && maybe(item.iconHour, integer) && maybe(item.placeHour, integer) && maybe(item.stateHour, integer) && maybe(item.memberHour, integer) && maybe(item.raidType, text) && maybe(item.raidTypeName, text) && maybe(item.icon, text) && maybe(item.iconMissing, text)
      && maybe(item.province, integer) && maybe(item.state, integer)
      && maybe(item.phase, integer) && maybe(item.phaseName, text) && maybe(item.outcome, integer) && maybe(item.outcomeName, text) && maybe(item.outcomeMissing, text)
      && isOrderedSpans(item.runs) && (item.runs as unknown[]).every(run => object(run) && typeof run.firstExact === 'boolean' && maybe(run.afterHour, integer) && maybe(run.raider, text) && integer(run.phase) && integer(run.outcome) && maybe(run.phaseName, text) && maybe(run.outcomeName, text))
      && preparationRuns(item.preparation)
      && maybe(item.launchedHour, integer) && typeof item.launchedExact === 'boolean' && typeof item.launched === 'boolean'
      && endObservations(item.ends) && maybe(item.succeededHour, integer)
      && maybe(item.disappearedHour, integer) && typeof item.disappearedExact === 'boolean'
      && (item.launchedHour === null || item.launched) && (!item.launchedExact || item.launchedHour !== null)
      && (!item.disappearedExact || item.disappearedHour !== null)
      && (item.succeededHour === null || (item.ends as unknown[]).length > 0));
  if (!ok) throw new Error('The raid log reply is malformed.');
  const reply = value as unknown as RaidLogReply;
  return { ...reply, artwork: reply.artwork.map(art => ({ ...art, image: drawableIcon(art.image) })) };
}

export function useRaidLog(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, catalogue = false) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
  const ranged = !catalogue && widget.time.mode === 'range';
  const range = ranged ? gate.window : null;
  const request = track && (!ranged || range) ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'raid-log',
    params: { branchId: track.branch.id, ...(catalogue ? { catalogue: true } : {}), ...(range ? { from: range.firstHour, to: range.lastHour } : {}) } }) : null;
  const product = useProduct<RaidLogReply>(runtime.client, request, active, validateRaidLog, track?.source);
  const read = catalogue ? null : gatedRead(gate, range ? { ...product, kind: 'dated', hour: range.lastHour } : { ...product, kind: 'branch' }, ranged && gate.open && !range ? UNRECORDED_MESSAGE : null);
  return { track, read, ...product, data: read ? readData(read) : product.data };
}

export type RaidMetric = 'both' | 'launched' | 'succeeded';
export interface RaidTallyRun { firstHour: number; lastHour: number; readHour: number; launched: number; succeeded: number; partial: string | null }
export const RAID_GAP_REASON = 'Raids during an unrecorded stretch before this date are not counted.';
export interface RaidTallySeries { tag: string; runs: RaidTallyRun[]; segments: Span[]; membership: Span[] | null; identities?: undefined }
export interface RaidTally { series: RaidTallySeries[]; hours: number[]; unattributed: [hour: number, count: number][] }

export function recordedSpans(span: Span, missing: readonly Span[]): Span[] {
  const out: Span[] = [];
  let at = span.firstHour;
  for (const gap of [...missing].sort((a, b) => a.firstHour - b.firstHour)) {
    if (gap.lastHour < at || gap.firstHour > span.lastHour) continue;
    if (gap.firstHour > at) out.push({ firstHour: at, lastHour: gap.firstHour - 1 });
    at = Math.max(at, gap.lastHour + 1);
  }
  if (at <= span.lastHour) out.push({ firstHour: at, lastHour: span.lastHour });
  return out;
}

type TallyEvent = [number, 0 | 1, 1 | -1];

export function raidTally(episodes: readonly RaidEpisode[], span: Span, missing: readonly Span[], chosen: readonly string[],
  top: { count: number; metric: RaidMetric } | null): RaidTally {
  const events = new Map<string, TallyEvent[]>();
  const chosenKeys = new Set(chosen);
  const count = (key: string, hour: number, kind: 0 | 1, delta: 1 | -1) => {
    const list = events.get(key) ?? [];
    list.push([hour, kind, delta]);
    events.set(key, list);
  };
  const unnamed: [number, 1 | -1][] = [];
  for (const episode of episodes) {
    const slot = episode.raiderSlot === null ? null : countryKey(episode.raiderSlot);
    const owner = (tag: string | null) => slot !== null && chosenKeys.has(slot) ? slot : tag;
    let launchUnnamed: number | null = null;
    const success: [number, boolean][] = [];
    if (episode.launchedHour !== null) {
      const key = owner(episode.launchedRaider);
      if (key === null) launchUnnamed = episode.launchedHour; else count(key, episode.launchedHour, 0, 1);
    }
    if (episode.succeededHour !== null) {
      const succeeded = episode.succeededHour;
      const known = episode.ends.filter(end => end.hour <= succeeded);
      let held = owner(known[known.length - 1]?.raider ?? null);
      if (held !== null) count(held, succeeded, 1, 1);
      success.push([succeeded, held === null]);
      for (const end of episode.ends) {
        if (end.hour <= succeeded) continue;
        const key = owner(end.raider);
        if (key === held) continue;
        if (held !== null) count(held, end.hour, 1, -1);
        if (key !== null) count(key, end.hour, 1, 1);
        held = key;
        success.push([end.hour, held === null]);
      }
    }
    const hours = [...new Set([...launchUnnamed === null ? [] : [launchUnnamed], ...success.map(([hour]) => hour)])].sort((a, b) => a - b);
    let shown = false, missing = false, next = 0;
    for (const hour of hours) {
      while (next < success.length && success[next][0] <= hour) missing = success[next++][1];
      const now = missing || (launchUnnamed !== null && launchUnnamed <= hour);
      if (now !== shown) unnamed.push([hour, now ? 1 : -1]);
      shown = now;
    }
  }
  for (const list of events.values()) list.sort((a, b) => a[0] - b[0]);
  const hours = [...new Set([...events.values()].flatMap(list => list.map(([hour]) => hour)))].sort((a, b) => a - b);
  const segments = recordedSpans(span, missing);
  const membership = top ? topMembership(events, hours, span, top) : null;
  const tags = top ? [...membership!.keys()] : [...new Set(chosen)];
  const series = tags.sort(compareTags).map(tag => ({ tag, runs: tallyRuns(events.get(tag) ?? [], segments), segments,
    membership: membership ? membership.get(tag) ?? [] : null }));
  return { series, hours, unattributed: steps(unnamed) };
}

function steps(changes: [number, 1 | -1][]): [number, number][] {
  const out: [number, number][] = [];
  let held = 0;
  for (const [hour, delta] of changes.sort((a, b) => a[0] - b[0])) {
    held += delta;
    if (out.length && out[out.length - 1][0] === hour) out[out.length - 1][1] = held; else out.push([hour, held]);
  }
  return out;
}

export function unattributedAt(tally: Pick<RaidTally, 'unattributed'>, hour: number): number {
  let lo = 0, hi = tally.unattributed.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (tally.unattributed[mid][0] <= hour) lo = mid + 1; else hi = mid; }
  return lo === 0 ? 0 : tally.unattributed[lo - 1][1];
}

function tallyRuns(events: readonly TallyEvent[], segments: readonly Span[]): RaidTallyRun[] {
  const runs: RaidTallyRun[] = [];
  let launched = 0, succeeded = 0, next = 0;
  segments.forEach((segment, index) => {
    const partial = index > 0 ? RAID_GAP_REASON : null;
    let first = segment.firstHour;
    const take = (through: number) => { while (next < events.length && events[next][0] <= through) { if (events[next][1] === 0) launched += events[next][2]; else succeeded += events[next][2]; next++; } };
    take(first);
    while (first <= segment.lastHour) {
      const change = next < events.length ? events[next][0] : Infinity;
      const last = Math.min(segment.lastHour, change - 1);
      runs.push({ firstHour: first, lastHour: last, readHour: first, launched, succeeded, partial });
      first = last + 1;
      take(first);
    }
  });
  return runs;
}

function topMembership(events: ReadonlyMap<string, readonly TallyEvent[]>, hours: readonly number[], span: Span,
  top: { count: number; metric: RaidMetric }): Map<string, Span[]> {
  const counts = new Map<string, [number, number]>();
  const by = new Map<number, [string, 0 | 1, 1 | -1][]>();
  for (const [tag, list] of events) for (const [hour, kind, delta] of list) by.set(hour, [...by.get(hour) ?? [], [tag, kind, delta]]);
  const out = new Map<string, Span[]>();
  let members: string[] = [];
  const close = (tag: string, last: number) => { const spans = out.get(tag)!; spans[spans.length - 1].lastHour = last; };
  for (const hour of hours) {
    for (const [tag, kind, delta] of by.get(hour)!) { const count = counts.get(tag) ?? [0, 0]; count[kind] += delta; counts.set(tag, count); }
    if (hour > span.lastHour) break;
    const value = ([launched, succeeded]: [number, number]) => top.metric === 'succeeded' ? succeeded : launched;
    const next = [...counts].filter(([, count]) => value(count) !== 0 || (top.metric === 'both' && count[1] !== 0))
      .sort((a, b) => value(b[1]) - value(a[1]) || compareTags(a[0], b[0])).slice(0, Math.max(0, top.count)).map(([tag]) => tag);
    const start = Math.max(hour, span.firstHour);
    for (const tag of members) if (!next.includes(tag)) close(tag, start - 1);
    for (const tag of next) if (!members.includes(tag)) out.set(tag, [...out.get(tag) ?? [], { firstHour: start, lastHour: span.lastHour }]);
    members = next;
  }
  for (const spans of out.values()) for (let at = spans.length - 1; at >= 0; at--) if (spans[at].lastHour < spans[at].firstHour) spans.splice(at, 1);
  return out;
}

export const tallyRunAt = (series: RaidTallySeries | undefined, hour: number) => runAt(series?.runs ?? [], Math.floor(hour));
