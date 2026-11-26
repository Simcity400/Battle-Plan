import { nameReadFrom, sameNameRead, type NameFact, type NameRead } from './nameReads.ts';
import { recordedHour } from './regionTime.ts';
import type { Failure } from './queryError';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { FactRequests, factsKey, HourStore, MAX_NAME_HOURS, nameKey, readFactBatch, readFacts } from './countryFacts.ts';
import type { BatchFactSpec, FactRead, FactRequestOrder, FactSpec, FactsFound, NameAt } from './countryFacts';
import { useContentRevision } from './contentRevision.ts';
import type { BoundTrack } from './model';
import { heldFacts, heldTimeline, watchTimeline } from './countryTimeline.ts';
import { inFrame } from './frameUpdates.ts';
import type { CountryTimeline, HeldLookup } from './countryTimeline';
import { countryFromKey } from './countryKey.ts';
import { useSlotTags, type SlotTags } from './countrySlots.ts';

export { nameKey };

type AnyFactSpec<V> = FactSpec<V, FactsFound<V>> | BatchFactSpec<V>;

const sameWanted = (a: readonly NameAt[], b: readonly NameAt[]) => a === b
  || a.length === b.length && a.every((item, index) => item.hour === b[index].hour && item.tag === b[index].tag);

const NO_ANSWERS: ReadonlyMap<string, never> = new Map<string, never>();
const NO_WANTED: readonly NameAt[] = [];

export function answerHeld<V>(held: HeldLookup<V> | undefined, timeline: CountryTimeline | undefined, wanted: readonly NameAt[],
  before: ReadonlyMap<string, V | null> | undefined): { answered: ReadonlyMap<string, V | null>; rest: readonly NameAt[] } {
  if (!held || !timeline) return { answered: NO_ANSWERS, rest: wanted };
  const answered = new Map<string, V | null>();
  const missing = new Map<number, string[]>();
  for (const item of wanted) {
    const key = nameKey(item.hour, item.tag);
    if (answered.has(key)) continue;
    if (before?.has(key)) { answered.set(key, before.get(key)!); continue; }
    const tags = missing.get(item.hour);
    if (tags) tags.push(item.tag); else missing.set(item.hour, [item.tag]);
  }
  const unheld = new Set<number>();
  for (const [hour, tags] of missing) {
    const found = held(timeline, hour, tags);
    if (!found) { unheld.add(hour); continue; }
    for (const tag of tags) answered.set(nameKey(hour, tag), found.get(tag) ?? null);
  }
  return { answered, rest: !unheld.size ? NO_WANTED : unheld.size === missing.size && !answered.size ? wanted : wanted.filter(item => unheld.has(item.hour)) };
}

export function useCountryFactsRead<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: AnyFactSpec<V>, maxHours = Infinity, attempt = 0): ReadonlyMap<string, V | null | Failure> {
  const { requests, answered, rest, version } = useFactStore(client, track, wanted, active, spec, maxHours, attempt);
  return useMemo(() => {
    const picked = requests.pickRead(rest);
    for (const [key, value] of answered) picked.set(key, value);
    return picked;
  }, [requests, answered, rest, version]);
}

export function useCountryFactsByKey<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: AnyFactSpec<V>, maxHours = Infinity, attempt = 0): ReadonlyMap<string, V | null | Failure> {
  const bySlot = wanted.some(item => typeof countryFromKey(item.tag) === 'number');
  const slots = useSlotTags(client, track, active && bySlot);
  const asked = useMemo(() => bySlot ? slotFacts(wanted, slots) : null, [bySlot, wanted, slots]);
  const read = useCountryFactsRead(client, track, asked?.wanted ?? wanted, active, spec, maxHours, attempt);
  return useMemo(() => {
    if (!asked) return read;
    const out = new Map<string, V | null | Failure>();
    for (const [key, held] of asked.back) {
      if (held === null) { if (slots) out.set(key, null); continue; }
      if (read.has(held)) out.set(key, read.get(held)!);
    }
    return out;
  }, [read, asked, slots]);
}

function slotFacts(wanted: readonly NameAt[], slots: SlotTags | undefined): { wanted: NameAt[]; back: Map<string, string | null> } {
  const out: NameAt[] = [], back = new Map<string, string | null>();
  for (const item of wanted) {
    const slot = countryFromKey(item.tag);
    const tag = typeof slot === 'number' ? slots?.tagAt(slot, item.hour) ?? null : item.tag;
    back.set(nameKey(item.hour, item.tag), tag === null ? null : nameKey(item.hour, tag));
    if (tag !== null) out.push({ hour: item.hour, tag });
  }
  return { wanted: out, back };
}

export function useHeldTimeline(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, content: number,
  watching: boolean): CountryTimeline | undefined {
  const [, told] = useState(0);
  const sourceId = track?.source.id, revision = track?.source.revision, branchId = track?.branch.id;
  const timeline = sourceId !== undefined && revision !== undefined && branchId !== undefined
    ? heldTimeline(client, { sourceId, revision, branchId, content }) : undefined;
  const shown = useRef(timeline);
  shown.current = timeline;
  useEffect(() => {
    if (!watching || sourceId === undefined || revision === undefined || branchId === undefined) return;
    const changed = () => inFrame(() => told(value => value + 1));
    const stop = watchTimeline(client, { sourceId, revision, branchId }, changed);
    if (heldTimeline(client, { sourceId, revision, branchId, content }) !== shown.current) changed();
    return stop;
  }, [watching, client, sourceId, revision, branchId, content]);
  return timeline;
}

function useFactStore<V>(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  spec: AnyFactSpec<V>, maxHours: number, attempt: number) {
  const content = useContentRevision(client);
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, spec.product, spec.identity, content]) : '';
  const order = 'batch' in spec ? undefined : spec.requestOrder;
  const keyed = useRef<{ trackKey: string; wanted: readonly NameAt[]; order?: FactRequestOrder; key: string } | null>(null);
  if (!keyed.current || keyed.current.trackKey !== trackKey || keyed.current.order !== order
    || !sameWanted(keyed.current.wanted, wanted)) {
    keyed.current = { trackKey, wanted, order, key: factsKey(trackKey, wanted, order) };
  }
  const key = keyed.current.key;
  const [version, setVersion] = useState(0);
  const timeline = useHeldTimeline(client, track, content, active && !!spec.held);
  const store = useRef<{ trackKey: string; client: DashboardClient; store: HourStore<V>; requests: FactRequests<V>;
    held?: { timeline: CountryTimeline | undefined; answered: ReadonlyMap<string, V | null> } } | null>(null);
  if (!store.current || store.current.trackKey !== trackKey || store.current.client !== client) {
    store.current?.requests.cancel();
    const held = new HourStore<V>(MAX_NAME_HOURS);
    const source = track ? { sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, content } : null;
    const read: FactRead<V> = (plan, signal, onChunk) => !source ? Promise.resolve()
      : 'batch' in spec ? readFactBatch(client, source, spec, plan, signal, onChunk, track?.source) : readFacts(client, source, spec, plan, signal, onChunk, track?.source);
    store.current = { trackKey, client, store: held, requests: new FactRequests(held, read, () => setVersion(value => value + 1)) };
  }
  const current = store.current;
  const { store: held, requests } = current;
  const split = useMemo(() => {
    const kept = current.held;
    const before = kept && kept.timeline === timeline ? kept.answered : undefined;
    const split = answerHeld(spec.held, timeline, keyed.current!.wanted, before);
    current.held = { timeline, answered: split.answered };
    return { ...split, key: split.rest === keyed.current!.wanted ? key : factsKey(trackKey, split.rest, order) };
  }, [current, timeline, key, spec.held]);
  const { answered, rest } = split;
  const attempted = useRef(attempt);
  useEffect(() => () => requests.cancel(), [requests]);
  useEffect(() => {
    if (!active || !track) { requests.cancel(); return; }
    if (!rest.length) { requests.cancel(); if (held.size) { held.clear(); setVersion(value => value + 1); } return; }
    if ('batch' in spec) requests.want(rest, maxHours, Infinity, 'time', true);
    else requests.want(rest, maxHours, spec.perRequest, spec.requestOrder);
    if (attempted.current !== attempt) { attempted.current = attempt; requests.retry(); }
  }, [split.key, active, attempt, requests, order]);
  return { held, requests, answered, rest, version };
}

function nameReasonOf(reason: string | null | undefined): string | null {
  if (!reason) return null;
  const cleaned = reason.replace(/;\s*no recorded map colour$/, '').trim();
  return !cleaned || cleaned === 'no recorded map colour' ? null : cleaned;
}

const NAMES: FactSpec<NameFact> = {
  product: 'country-styles', perRequest: Infinity, shared: true,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.flatMap((style): readonly (readonly [string, NameFact])[] => {
    if (typeof style.name === 'string') return [[style.tag, style.name]];
    const reason = nameReasonOf(style.reason);
    if (reason) return [[style.tag, { reason }]];
    return [];
  })),
  held: heldFacts(style => {
    if (style.name != null) return style.name;
    const reason = nameReasonOf(style.reason);
    return reason ? { reason } : null;
  }),
};

export function useCountryNamesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  maxHours = Infinity, attempt = 0, exact = false): ReadonlyMap<string, NameRead> {
  const available = useMemo(() => wanted.filter(item => Number.isSafeInteger(item.hour) && recordedHour(track, item.hour) === item.hour), [wanted, track]);
  const read = useCountryFactsByKey(client, track, available, active, NAMES, maxHours, attempt);
  const previous = useRef<ReadonlyMap<string, NameRead>>(new Map());
  const lastRead = useRef<{ client: DashboardClient | null; track: string; names: Map<string, NameRead> }>({ client: null, track: '', names: new Map() });
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : '';
  if (lastRead.current.track !== trackKey || lastRead.current.client !== client) lastRead.current = { client, track: trackKey, names: new Map() };
  return useMemo(() => {
    const requested = new Set(available.map(item => nameKey(item.hour, item.tag)));
    const held = lastRead.current.names;
    const next = new Map(wanted.map(item => {
      const key = nameKey(item.hour, item.tag), before = previous.current.get(key);
      let value = nameReadFrom(read.get(key), active && requested.has(key));
      if (value.kind === 'value' || value.kind === 'failed') held.set(item.tag, value);
      else if (value.kind === 'loading' && !exact) value = held.get(item.tag) ?? value;
      return [key, sameNameRead(before, value) ? before! : value] as const;
    }));
    if (next.size !== previous.current.size || [...next].some(([key, value]) => previous.current.get(key) !== value)) previous.current = next;
    return previous.current;
  }, [read, available, wanted, active, exact]);
}
