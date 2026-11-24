import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useGate } from './regionGate.ts';
import { SPEC, UNRECORDED_MESSAGE } from './regionTime.ts';
import { useProduct } from './products.ts';
import { contentTitle } from './contentTitles.ts';
import { drawableIcon } from './artwork.ts';
import { compareTags } from './tagOrder.ts';
import { isOrderedSpans, runAt } from '../charts/series.ts';
import type { Span } from '../charts/series';
import type { RaidEpisode, RaidLogReply, RaidPhaseRun, RaidPreparationRun, RaidTypeInfo, RaidVictimAt, RaidVictimSource } from '../generated/dashboard';
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

export function validateRaidLog(value: unknown): RaidLogReply {
  const ok = object(value) && Array.isArray(value.episodes) && Array.isArray(value.missing) && Array.isArray(value.warnings)
    && maybe(value.available, item => typeof item === 'boolean') && maybe(value.unavailable, text)
    && [value.phasePreparing, value.phaseEnded, value.outcomeNone, value.outcomeFailure, value.outcomeLimitedSuccess, value.outcomeSuccess, value.outcomeCriticalSuccess, value.outcomeCanceled].every(integer)
    && Array.isArray(value.types) && value.types.every(type => object(type) && text(type.token) && maybe(type.name, text) && maybe(type.missing, text))
    && Array.isArray(value.artwork) && value.artwork.every(art => object(art) && text(art.key) && maybe(art.image, text) && maybe(art.missing, text))
    && value.warnings.every(text) && value.missing.every(span => object(span) && integer(span.firstHour) && integer(span.lastHour) && text(span.reason))
    && value.episodes.every(item => object(item) && text(item.id) && integer(item.firstHour) && typeof item.firstHourExact === 'boolean' && integer(item.lastHour) && (item.firstHour as number) <= (item.lastHour as number)
      && maybe(item.raider, text) && maybe(item.raiderSlot, integer) && maybe(item.launchedRaider, text) && maybe(item.endedRaider, text)
      && maybe(item.victim, text) && (item.victim === null) !== (item.victimMissing === null)
      && maybe(item.victimSource, victimSource) && (item.victim !== null || item.victimSource === null)
      && victimObservations(item.victims)
      && maybe(item.raidType, text) && maybe(item.raidTypeName, text) && maybe(item.icon, text) && maybe(item.iconMissing, text)
      && maybe(item.province, integer) && maybe(item.state, integer)
      && maybe(item.phase, integer) && maybe(item.phaseName, text) && maybe(item.outcome, integer) && maybe(item.outcomeName, text) && maybe(item.outcomeMissing, text)
      && isOrderedSpans(item.runs) && (item.runs as unknown[]).every(run => object(run) && typeof run.firstExact === 'boolean' && maybe(run.afterHour, integer) && maybe(run.raider, text) && integer(run.phase) && integer(run.outcome) && maybe(run.phaseName, text) && maybe(run.outcomeName, text))
      && preparationRuns(item.preparation)
      && maybe(item.launchedHour, integer) && typeof item.launchedExact === 'boolean' && typeof item.launched === 'boolean'
      && maybe(item.endedHour, integer) && typeof item.endedExact === 'boolean' && maybe(item.endedSource, endedSource) && maybe(item.succeededHour, integer)
      && maybe(item.disappearedHour, integer) && typeof item.disappearedExact === 'boolean'
      && (item.launchedHour === null || item.launched) && (!item.launchedExact || item.launchedHour !== null)
      && (!item.endedExact || item.endedHour !== null) && (item.endedHour === null) === (item.endedSource === null)
      && (!item.disappearedExact || item.disappearedHour !== null)
      && (item.succeededHour === null || item.endedHour !== null));
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

export function startedBy(episodes: readonly RaidEpisode[], hour: number | null): number {
  if (hour === null) return episodes.length;
  let lo = 0, hi = episodes.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (episodes[mid].firstHour <= hour) lo = mid + 1; else hi = mid; }
  return lo;
}

export function raidType(types: readonly RaidTypeInfo[], episode: RaidEpisode): RaidTypeInfo | null {
  return types.find(type => type.token === episode.raidType) ?? null;
}
export function raidTypeName(type: RaidTypeInfo | null): string { return contentTitle(type?.name, type?.token, 'Raid name unavailable'); }
export const raidTypeText = (data: Pick<RaidLogReply, 'types'>, episode: RaidEpisode) =>
  contentTitle(raidType(data.types, episode)?.name ?? episode.raidTypeName, episode.raidType, episode.raidType === null ? 'Raid type not recorded' : 'Raid name unavailable');
export function raidTypeChoices(types: readonly RaidTypeInfo[]): { token: string; label: string }[] {
  const counts = new Map<string, number>();
  for (const type of types) { const name = raidTypeName(type); counts.set(name, (counts.get(name) ?? 0) + 1); }
  const ordinals = new Map<string, number>();
  return types.map(type => {
    const name = raidTypeName(type), ordinal = (ordinals.get(name) ?? 0) + 1;
    ordinals.set(name, ordinal);
    return { token: type.token, label: counts.get(name)! > 1 ? `${name} (${ordinal})` : name };
  });
}
export function raidIcon(data: Pick<RaidLogReply, 'artwork'>, episode: RaidEpisode): string | null {
  return episode.icon === null ? null : data.artwork.find(art => art.key === episode.icon)?.image ?? null;
}

export function raidVictimAt(episode: RaidEpisode, hour: number): RaidVictimAt {
  const list = episode.victims;
  let found = -1;
  for (let low = 0, high = list.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (list[middle].hour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  if (found >= 0) return list[found];
  return list[0] ?? { hour: episode.firstHour, tag: episode.victim, missing: episode.victimMissing, source: episode.victimSource };
}

export interface RaidAt {
  episode: RaidEpisode;
  latest: RaidPhaseRun;
  current: RaidPhaseRun | null;
  since: number;
  sinceExact: boolean;
  afterHour: number | null;
  disappeared: boolean;
  ended: boolean;
  raider: string | null;
  victim: RaidVictimAt;
  hour: number;
}

export function raidAt(episode: RaidEpisode, hour: number): RaidAt | null {
  const runs = episode.runs;
  let index = -1;
  for (let low = 0, high = runs.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (runs[middle].firstHour <= hour) { index = middle; low = middle + 1; } else high = middle - 1;
  }
  if (index < 0) return null;
  const latest = runs[index];
  let first = index;
  while (first > 0 && runs[first - 1].phase === latest.phase && runs[first - 1].outcome === latest.outcome) first--;
  const current = latest.lastHour >= hour ? latest : null;
  const disappeared = current === null && episode.disappearedHour !== null && hour >= episode.disappearedHour;
  const ended = episode.endedHour !== null && episode.phase !== null && latest.phase === episode.phase;
  const nativeEnded = ended && episode.endedSource === 'nativeEndDate' && episode.endedHour !== null;
  const shown = current ?? latest;
  return {
    episode,
    latest,
    current,
    since: disappeared ? episode.disappearedHour! : nativeEnded ? episode.endedHour! : runs[first].firstHour,
    sinceExact: disappeared ? episode.disappearedExact : nativeEnded ? true : runs[first].firstExact,
    afterHour: disappeared ? episode.lastHour : nativeEnded ? episode.endedHour! - 1 : runs[first].afterHour,
    disappeared,
    ended,
    raider: shown.raider ?? episode.raider,
    victim: raidVictimAt(episode, hour),
    hour,
  };
}

export function raidChangeHours(episodes: readonly RaidEpisode[]): number[] {
  const hours: number[] = [];
  for (const episode of episodes) {
    for (const run of episode.runs) hours.push(run.firstHour, run.lastHour + 1);
    for (let at = 1; at < episode.victims.length; at++) {
      const hour = episode.victims[at].hour;
      if (episode.runs.every(run => run.firstHour !== hour && run.lastHour + 1 !== hour)) hours.push(hour);
    }
    if (episode.disappearedHour !== null) hours.push(episode.disappearedHour);
  }
  return hours.sort((a, b) => a - b);
}

export function raidPreparationAt(episode: RaidEpisode, hour: number): number | null {
  const runs = episode.preparation, at = Math.floor(hour);
  let found: RaidPreparationRun | null = null;
  for (let low = 0, high = runs.length - 1; low <= high;) {
    const middle = (low + high) >> 1;
    if (runs[middle][0] <= at) { found = runs[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found && at <= found[1] ? found[2] + (at - found[0]) * found[3] : null;
}

export function raidStatus(data: Pick<RaidLogReply, 'phaseEnded' | 'outcomeNone' | 'outcomeCanceled'>, at: RaidAt): string {
  const value = at.current ?? at.latest;
  const phase = value.phaseName ?? (value.phase < 0 || value.phase > data.phaseEnded ? `Unsupported phase (${value.phase})` : `Phase ${value.phase} (name unavailable)`);
  if (at.disappeared) return `Last known: ${phase} · ${at.episode.outcomeMissing ?? 'raid outcome was not recorded'}`;
  let outcome: string | null = null;
  if (value.phase === data.phaseEnded) {
    if (value.outcome === data.outcomeNone) outcome = at.episode.outcomeMissing ?? 'raid outcome was not recorded';
    else if (value.outcome < data.outcomeNone || value.outcome > data.outcomeCanceled) outcome = value.outcomeName ?? `Unsupported outcome (${value.outcome})`;
    else outcome = value.outcomeName ?? `Outcome ${value.outcome} (name unavailable)`;
  }
  const status = outcome === null ? phase : `${phase} · ${outcome}`;
  return at.current ? status : `Last known: ${status}`;
}

export function raidVictimLabel(source: RaidVictimSource | null): string {
  if (source === 'completionCountry') return 'Country at completion';
  if (source === 'fixedTarget') return 'Fixed target country';
  if (source === 'controllerAtFirstEndedRead') return 'Target controller at first ended read';
  if (source === 'controllerAtObservedRead') return 'Target controller';
  if (source === 'captorAtFirstEndedRead') return 'Leader captor at first ended read';
  if (source === 'captorAtObservedRead') return 'Leader captor';
  return 'Target country';
}

export function raidVictimRole(source: RaidVictimSource | null): string | null {
  if (source === 'fixedTarget') return null;
  if (source === 'completionCountry') return 'country at completion';
  if (source === 'controllerAtFirstEndedRead') return 'controller at first ended read';
  if (source === 'controllerAtObservedRead') return 'target controller';
  if (source === 'captorAtFirstEndedRead') return 'captor at first ended read';
  if (source === 'captorAtObservedRead') return 'captor';
  return 'target country';
}

export const raidPreparing = (data: Pick<RaidLogReply, 'phasePreparing'>, at: RaidAt) => (at.current ?? at.latest).phase === data.phasePreparing;

export function raidPreparationText(data: Pick<RaidLogReply, 'phasePreparing'>, at: RaidAt, hour: number): string | null {
  if (!raidPreparing(data, at)) return null;
  const hours = at.current ? raidPreparationAt(at.episode, hour) : null;
  return hours === null ? 'Preparation not known' : `${hours.toLocaleString('en-US')} h prepared`;
}

export function raidPlaceId(episode: RaidEpisode): string {
  return episode.province !== null ? `Province ${episode.province}` : episode.state !== null ? `State ${episode.state}` : 'Target not known';
}

export type RaidMetric = 'both' | 'launched' | 'succeeded';
export interface RaidTallyRun { firstHour: number; lastHour: number; readHour: number; launched: number; succeeded: number }
export interface RaidTallySeries { tag: string; runs: RaidTallyRun[]; segments: Span[]; membership: Span[] | null; identities?: undefined }
export interface RaidTally { series: RaidTallySeries[]; hours: number[]; unattributed: number }

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

export function raidTally(episodes: readonly RaidEpisode[], span: Span, missing: readonly Span[], chosen: readonly string[],
  top: { count: number; metric: RaidMetric } | null): RaidTally {
  const events = new Map<string, [number, 0 | 1][]>();
  const chosenKeys = new Set(chosen);
  let unattributed = 0;
  for (const episode of episodes) {
    const marks: [number, 0 | 1, string | null][] = [];
    if (episode.launchedHour !== null) marks.push([episode.launchedHour, 0, episode.launchedRaider]);
    if (episode.succeededHour !== null) marks.push([episode.succeededHour, 1, episode.endedRaider]);
    if (!marks.length) continue;
    const slot = episode.raiderSlot === null ? null : countryKey(episode.raiderSlot);
    let missingRaider = false;
    for (const [hour, kind, raider] of marks) {
      const key = slot !== null && chosenKeys.has(slot) ? slot : raider;
      if (key === null) { missingRaider = true; continue; }
      const list = events.get(key) ?? [];
      list.push([hour, kind]);
      events.set(key, list);
    }
    if (missingRaider) unattributed++;
  }
  for (const list of events.values()) list.sort((a, b) => a[0] - b[0]);
  const hours = [...new Set([...events.values()].flatMap(list => list.map(([hour]) => hour)))].sort((a, b) => a - b);
  const segments = recordedSpans(span, missing);
  const membership = top ? topMembership(events, hours, span, top) : null;
  const tags = top ? [...membership!.keys()] : [...new Set(chosen)];
  const series = tags.sort(compareTags).map(tag => ({ tag, runs: tallyRuns(events.get(tag) ?? [], segments), segments,
    membership: membership ? membership.get(tag) ?? [] : null }));
  return { series, hours, unattributed };
}

function tallyRuns(events: readonly [number, 0 | 1][], segments: readonly Span[]): RaidTallyRun[] {
  const runs: RaidTallyRun[] = [];
  let launched = 0, succeeded = 0, next = 0;
  for (const segment of segments) {
    let first = segment.firstHour;
    const take = (through: number) => { while (next < events.length && events[next][0] <= through) { if (events[next][1] === 0) launched++; else succeeded++; next++; } };
    take(first);
    while (first <= segment.lastHour) {
      const change = next < events.length ? events[next][0] : Infinity;
      const last = Math.min(segment.lastHour, change - 1);
      runs.push({ firstHour: first, lastHour: last, readHour: first, launched, succeeded });
      first = last + 1;
      take(first);
    }
  }
  return runs;
}

function topMembership(events: ReadonlyMap<string, readonly [number, 0 | 1][]>, hours: readonly number[], span: Span,
  top: { count: number; metric: RaidMetric }): Map<string, Span[]> {
  const counts = new Map<string, [number, number]>();
  const by = new Map<number, [string, 0 | 1][]>();
  for (const [tag, list] of events) for (const [hour, kind] of list) by.set(hour, [...by.get(hour) ?? [], [tag, kind]]);
  const out = new Map<string, Span[]>();
  let members: string[] = [];
  const close = (tag: string, last: number) => { const spans = out.get(tag)!; spans[spans.length - 1].lastHour = last; };
  for (const hour of hours) {
    for (const [tag, kind] of by.get(hour)!) { const count = counts.get(tag) ?? [0, 0]; count[kind]++; counts.set(tag, count); }
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

export function eventsThrough(hours: readonly number[], hour: number): number {
  let lo = 0, hi = hours.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (hours[mid] <= hour) lo = mid + 1; else hi = mid; }
  return lo;
}

export const tallyRunAt = (series: RaidTallySeries | undefined, hour: number) => runAt(series?.runs ?? [], Math.floor(hour));
