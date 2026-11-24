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
import type { RaidEndAt, RaidEpisode, RaidLogReply, RaidPhaseRun, RaidPreparationRun, RaidTypeInfo, RaidVictimAt, RaidVictimSource } from '../generated/dashboard';
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

export type RaidOutcomeCodes = Pick<RaidLogReply, 'outcomeNone' | 'outcomeFailure' | 'outcomeCanceled'>;

export function raidOutcomeMissing(codes: RaidOutcomeCodes, ended: boolean, disappeared: boolean, outcome: number | null): string | null {
  if (ended) {
    if (outcome !== null && outcome >= codes.outcomeFailure && outcome <= codes.outcomeCanceled) return null;
    if (outcome !== null && outcome !== codes.outcomeNone) return `Unsupported raid outcome (${outcome})`;
    return 'raid outcome was not recorded';
  }
  return disappeared ? 'raid outcome was not recorded' : null;
}

export function raidEpisodeAt(codes: RaidOutcomeCodes, episode: RaidEpisode, hour: number): RaidEpisode | null {
  if (episode.firstHour > hour || (episode.memberHour !== null && episode.memberHour > hour)) return null;
  const runs = episode.runs.filter(run => run.firstHour <= hour).map(run => run.lastHour > hour ? { ...run, lastHour: hour } : run);
  const latest = runs[runs.length - 1];
  if (!latest) return null;
  const preparation = episode.preparation.filter(run => run[0] <= hour)
    .map((run): RaidPreparationRun => run[1] > hour ? [run[0], hour, run[2], run[0] === hour ? 0 : run[3]] : run);
  const held = episode.victims.filter(victim => victim.hour <= hour);
  const victims: RaidVictimAt[] = held.length ? held : [{ hour: episode.firstHour, tag: null, missing: "The raid's target is not recorded", source: null }];
  const summary = victims[victims.length - 1];
  const typed = episode.raidTypeHour !== null && episode.raidTypeHour <= hour;
  const iconed = typed && episode.iconHour !== null && episode.iconHour <= hour;
  const placed = episode.placeHour === null || episode.placeHour <= hour;
  const stated = episode.stateHour !== null && episode.stateHour <= hour;
  const launched = episode.launchedHour === null || episode.launchedHour <= hour;
  const ends = episode.ends.filter(end => end.hour <= hour);
  const disappeared = episode.disappearedHour !== null && episode.disappearedHour <= hour;
  const known = [...runs].reverse().find(run => run.outcome !== codes.outcomeNone) ?? latest;
  return {
    ...episode, runs, preparation, victims, lastHour: latest.lastHour,
    victim: summary.tag, victimMissing: summary.missing, victimSource: summary.source,
    raidTypeHour: typed ? episode.raidTypeHour : null, raidType: typed ? episode.raidType : null, raidTypeName: typed ? episode.raidTypeName : null,
    iconHour: iconed ? episode.iconHour : null, icon: iconed ? episode.icon : null,
    iconMissing: iconed ? episode.iconMissing : typed ? "The raid's target is not recorded in full" : "The raid's type is not recorded",
    placeHour: placed ? episode.placeHour : null, province: placed ? episode.province : null,
    stateHour: stated ? episode.stateHour : null, state: stated ? episode.state : null,
    launched: launched && episode.launched, launchedHour: launched ? episode.launchedHour : null, launchedExact: launched && episode.launchedExact,
    launchedRaider: launched ? episode.launchedRaider : null,
    ends, succeededHour: episode.succeededHour !== null && episode.succeededHour <= hour ? episode.succeededHour : null,
    disappearedHour: disappeared ? episode.disappearedHour : null, disappearedExact: disappeared && episode.disappearedExact,
    phase: latest.phase, phaseName: latest.phaseName, outcome: known.outcome, outcomeName: known.outcomeName,
    outcomeMissing: raidOutcomeMissing(codes, ends.length > 0, disappeared, known.outcome),
  };
}

export interface RaidAt {
  episode: RaidEpisode;
  latest: RaidPhaseRun;
  current: RaidPhaseRun | null;
  end: RaidEndAt | null;
  since: number;
  sinceExact: boolean;
  afterHour: number | null;
  disappeared: boolean;
  ended: boolean;
  raider: string | null;
  victim: RaidVictimAt;
  hour: number;
}

export function raidAt(codes: RaidOutcomeCodes, full: RaidEpisode, hour: number): RaidAt | null {
  const episode = raidEpisodeAt(codes, full, hour);
  if (!episode) return null;
  const runs = episode.runs;
  const latest = runs[runs.length - 1];
  let first = runs.length - 1;
  while (first > 0 && runs[first - 1].phase === latest.phase && runs[first - 1].outcome === latest.outcome) first--;
  const current = latest.lastHour >= hour ? latest : null;
  const disappeared = current === null && episode.disappearedHour !== null;
  const end = episode.ends[episode.ends.length - 1] ?? null;
  const nativeEnded = end !== null && end.source === 'nativeEndDate';
  const shown = current ?? latest;
  return {
    episode,
    latest,
    current,
    end,
    since: disappeared ? episode.disappearedHour! : nativeEnded ? end.endedHour : runs[first].firstHour,
    sinceExact: disappeared ? episode.disappearedExact : nativeEnded ? true : runs[first].firstExact,
    afterHour: disappeared ? episode.lastHour : nativeEnded ? end.endedHour - 1 : runs[first].afterHour,
    disappeared,
    ended: end !== null,
    raider: shown.raider ?? episode.raider,
    victim: episode.victims[episode.victims.length - 1],
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
    for (const hour of [episode.memberHour, episode.raidTypeHour, episode.iconHour, episode.placeHour, episode.stateHour, ...episode.ends.map(end => end.hour)]) {
      if (hour !== null && hour !== episode.firstHour && episode.runs.every(run => run.firstHour !== hour && run.lastHour + 1 !== hour)) hours.push(hour);
    }
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
  let unattributed = 0;
  for (const episode of episodes) {
    const slot = episode.raiderSlot === null ? null : countryKey(episode.raiderSlot);
    const owner = (tag: string | null) => slot !== null && chosenKeys.has(slot) ? slot : tag;
    let missingRaider = false;
    if (episode.launchedHour !== null) {
      const key = owner(episode.launchedRaider);
      if (key === null) missingRaider = true; else count(key, episode.launchedHour, 0, 1);
    }
    if (episode.succeededHour !== null) {
      const succeeded = episode.succeededHour;
      const known = episode.ends.filter(end => end.hour <= succeeded);
      let held = owner(known[known.length - 1]?.raider ?? null);
      if (held !== null) count(held, succeeded, 1, 1);
      for (const end of episode.ends) {
        if (end.hour <= succeeded) continue;
        const key = owner(end.raider);
        if (key === held) continue;
        if (held !== null) count(held, end.hour, 1, -1);
        if (key !== null) count(key, end.hour, 1, 1);
        held = key;
      }
      if (held === null) missingRaider = true;
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

function tallyRuns(events: readonly TallyEvent[], segments: readonly Span[]): RaidTallyRun[] {
  const runs: RaidTallyRun[] = [];
  let launched = 0, succeeded = 0, next = 0;
  for (const segment of segments) {
    let first = segment.firstHour;
    const take = (through: number) => { while (next < events.length && events[next][0] <= through) { if (events[next][1] === 0) launched += events[next][2]; else succeeded += events[next][2]; next++; } };
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

export function eventsThrough(hours: readonly number[], hour: number): number {
  let lo = 0, hi = hours.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (hours[mid] <= hour) lo = mid + 1; else hi = mid; }
  return lo;
}

export const tallyRunAt = (series: RaidTallySeries | undefined, hour: number) => runAt(series?.runs ?? [], Math.floor(hour));
