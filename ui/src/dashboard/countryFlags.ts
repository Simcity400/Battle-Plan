import { countryFromKey } from './countryKey.ts';
import { useSlotTags } from './countrySlots.ts';
import { asFailure, type Failure } from './queryError.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { CountryFlagChoice, CountryFlagImage, CountryFlagReply, CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from './client';
import { isFailedFact, nameKey, retryDelay, transientError } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import { askableAt, askTimeline, flagIn, heldTimeline, timelineFailure, retryTimelines, watchTimeline } from './countryTimeline.ts';
import { useContentRevision } from './contentRevision.ts';
import { inFrame } from './frameUpdates.ts';
import type { BoundTrack } from './model';
import { loadTransientProduct } from './products.ts';
import { busyWait } from '../shell/busy.ts';
import { PerClient } from './prepare.ts';
import { productRequest } from './request.ts';

export type FlagRead = Readonly<{ url?: string; image?: CountryFlagImage; reason?: string }>;
export interface FlagAt { tag: string; branchId: string; hour: number }
export type FlagValue = FlagRead | null | Failure;

export const FLAG_BATCH = 32;
const MAX_DRAWING = 2;

type Drawing = { read: FlagRead | null } | { failed: Failure };

interface Drawings {
  content: number;
  entries: Map<string, Drawing>;
  failures: Map<string, number>;
  asked: Set<string>;
  queue: Map<string, { recording: DashboardSource; size: CountryFlagSize; choices: Map<string, CountryFlagChoice> }>;
  running: number;
  pumpScheduled: boolean;
  controllers: Set<{ sourceId: string; controller: AbortController }>;
  watchers: Set<(drawn: ReadonlySet<string>) => void>;
  told: Set<string> | null;
}

const states = new PerClient<Drawings>();
const choiceKeys = new WeakMap<CountryFlagChoice, string>();
const unavailableReads = new PerClient<Map<string, FlagRead>>();

function unavailableFlag(client: DashboardClient, reason: string): FlagRead {
  let reasons = unavailableReads.get(client);
  if (!reasons) unavailableReads.set(client, reasons = new Map());
  let read = reasons.get(reason);
  if (!read) reasons.set(reason, read = { reason });
  return read;
}

function drawings(client: DashboardClient, content: number): Drawings {
  let state = states.get(client);
  if (state && state.content === content) return state;
  for (const { controller } of state?.controllers ?? []) controller.abort();
  const fresh: Drawings = { content, entries: new Map(), failures: new Map(), asked: new Set(), queue: new Map(), running: 0, pumpScheduled: false, controllers: new Set(),
    watchers: state?.watchers ?? new Set(), told: null };
  states.set(client, fresh);
  return fresh;
}

const groupKey = (recording: Pick<DashboardSource, 'id' | 'revision'>, size: CountryFlagSize) => JSON.stringify([recording.id, recording.revision, size]);

function drawKey(group: string, choice: CountryFlagChoice): string {
  let key = choiceKeys.get(choice);
  if (key === undefined) choiceKeys.set(choice, key = JSON.stringify([choice.keys, choice.reasons]));
  return `${group}\n${key}`;
}

function tell(state: Drawings, key: string): void {
  if (state.told) { state.told.add(key); return; }
  state.told = new Set([key]);
  inFrame(() => {
    const drawn = state.told ?? new Set<string>();
    state.told = null;
    for (const watcher of [...state.watchers]) watcher(drawn);
  });
}

export function drawnFlag(client: DashboardClient, recording: Pick<DashboardSource, 'id' | 'revision'>, content: number, size: CountryFlagSize,
  choice: CountryFlagChoice): FlagValue | undefined {
  const state = states.get(client);
  const drawn = state?.content === content ? state.entries.get(drawKey(groupKey(recording, size), choice)) : undefined;
  return drawn && ('failed' in drawn ? drawn.failed : drawn.read);
}

export function drawFlags(client: DashboardClient, recording: DashboardSource, content: number, size: CountryFlagSize, choices: Iterable<CountryFlagChoice>): void {
  const state = drawings(client, content);
  const group = groupKey(recording, size);
  for (const choice of choices) {
    const key = drawKey(group, choice);
    if (state.entries.has(key) || state.asked.has(key)) continue;
    state.asked.add(key);
    let queued = state.queue.get(group);
    if (!queued) state.queue.set(group, queued = { recording, size, choices: new Map() });
    queued.choices.set(key, choice);
  }
  if (!state.pumpScheduled) {
    state.pumpScheduled = true;
    queueMicrotask(() => {
      state.pumpScheduled = false;
      if (states.get(client) === state) pump(client, state);
    });
  }
}

function pump(client: DashboardClient, state: Drawings): void {
  while (state.running < MAX_DRAWING && state.queue.size) {
    const [group, queued] = state.queue.entries().next().value!;
    const batch = [...queued.choices].slice(0, FLAG_BATCH);
    for (const [key] of batch) queued.choices.delete(key);
    if (!queued.choices.size) state.queue.delete(group);
    void draw(client, state, queued.recording, queued.size, batch);
  }
}

async function draw(client: DashboardClient, state: Drawings, recording: DashboardSource, size: CountryFlagSize, batch: readonly [string, CountryFlagChoice][]): Promise<void> {
  state.running++;
  const owner = { sourceId: recording.id, controller: new AbortController() };
  state.controllers.add(owner);
  let reply: CountryFlagReply | undefined, retryable = false;
  let failure: Failure = new Error('Flag drawing failed');
  try {
    reply = await loadTransientProduct<CountryFlagReply>(client, productRequest({ sourceId: recording.id, revision: recording.revision, product: 'country-flags',
      params: { flags: batch.map(([, choice]) => choice), size } }), owner.controller.signal);
  } catch (error) {
    retryable = transientError(error);
    failure = asFailure(error);
  }
  state.controllers.delete(owner);
  state.running--;
  if (states.get(client) !== state) return;
  if (owner.controller.signal.aborted) { pump(client, state); return; }
  const images = new Map(reply?.flags.map(image => [image.flag, image]));
  const unavailable = new Map(reply?.unavailable.map(item => [item.flag, item.reason]));
  batch.forEach(([key], index) => {
    state.asked.delete(key);
    if (reply) {
      const image = images.get(index), reason = unavailable.get(index);
      state.entries.set(key, { read: image ? { url: image.png, image } : reason !== undefined ? { reason } : null });
      state.failures.delete(key);
    } else {
      const count = (state.failures.get(key) ?? 0) + 1;
      state.failures.set(key, count);
      state.entries.set(key, { failed: failure });
      if (retryable) busyWait(() => {
        if (states.get(client) !== state || !('failed' in (state.entries.get(key) ?? {}))) return;
        state.entries.delete(key);
        tell(state, key);
      }, retryDelay(count));
    }
    tell(state, key);
  });
  if (!state.pumpScheduled) {
    state.pumpScheduled = true;
    queueMicrotask(() => {
      state.pumpScheduled = false;
      if (states.get(client) === state) pump(client, state);
    });
  }
}

export function watchFlagDrawings(client: DashboardClient, watcher: (drawn: ReadonlySet<string>) => void): () => void {
  const watchers = drawings(client, states.get(client)?.content ?? 0).watchers;
  watchers.add(watcher);
  return () => { watchers.delete(watcher); };
}

export function forgetFlags(client: DashboardClient, sourceId: string, settled = false): void {
  const state = states.get(client);
  if (!state) return;
  const ours = (key: string) => (JSON.parse(key.slice(0, key.indexOf('\n'))) as unknown[])[0] === sourceId;
  for (const key of [...state.entries.keys()]) if (ours(key)) { state.entries.delete(key); state.failures.delete(key); tell(state, key); }
  if (settled) return;
  for (const owner of [...state.controllers]) if (owner.sourceId === sourceId) { owner.controller.abort(); state.controllers.delete(owner); }
  for (const key of [...state.asked]) if (ours(key)) state.asked.delete(key);
  for (const [group, queued] of [...state.queue]) if (queued.recording.id === sourceId) state.queue.delete(group);
}

export function retryFlags(client: DashboardClient, sourceId: string): void {
  retryTimelines(client, sourceId);
  const state = states.get(client);
  if (!state) return;
  for (const [key, entry] of [...state.entries]) {
    if (!('failed' in entry) || (JSON.parse(key.slice(0, key.indexOf('\n'))) as unknown[])[0] !== sourceId) continue;
    state.entries.delete(key);
    state.failures.delete(key);
    tell(state, key);
  }
}

export const flagReadKey = (read: FlagAt) => JSON.stringify([read.branchId, read.hour, read.tag]);

export function useFlagReads(client: DashboardClient, recording: DashboardSource | undefined, reads: readonly FlagAt[], active: boolean,
  size: CountryFlagSize, exact = false): ReadonlyMap<string, FlagValue> {
  const content = useContentRevision(client);
  const readsKey = JSON.stringify(reads.map(read => [read.branchId, read.hour, read.tag]));
  const wanted = useMemo(() => JSON.parse(readsKey) as [string, number, string][], [readsKey]);
  const [version, setVersion] = useState(0);
  const sourceId = recording?.id, revision = recording?.revision;
  const shown = useMemo(() => {
    const values = new Map<string, FlagValue>();
    const unheld = new Map<string, number>(), undrawn: CountryFlagChoice[] = [], awaited = new Set<string>();
    if (sourceId === undefined || revision === undefined) return { values, unheld, undrawn, awaited };
    const state = states.get(client);
    const group = groupKey({ id: sourceId, revision }, size);
    for (const [branchId, hour, tag] of wanted) {
      const timeline = heldTimeline(client, { sourceId, revision, branchId, content });
      const flag = timeline && flagIn(timeline, hour, tag);
      if (!flag) {
        const error = timelineFailure(client, { sourceId, revision, branchId, content });
        if (error) values.set(JSON.stringify([branchId, hour, tag]), error);
        if (!unheld.has(branchId)) unheld.set(branchId, hour);
        continue;
      }
      const key = JSON.stringify([branchId, hour, tag]);
      if ('reason' in flag) { values.set(key, unavailableFlag(client, flag.reason)); continue; }
      const drawing = drawKey(group, flag.choice);
      const drawn = state?.content === content ? state.entries.get(drawing) : undefined;
      if (!drawn) { undrawn.push(flag.choice); awaited.add(drawing); continue; }
      if ('failed' in drawn) { values.set(key, drawn.failed); awaited.add(drawing); } else values.set(key, drawn.read);
    }
    return { values, unheld, undrawn, awaited };
  }, [client, sourceId, revision, content, size, wanted, version]);
  const awaiting = useRef(shown);
  useEffect(() => {
    awaiting.current = shown;
    if (!active || !recording) return;
    let wait: number | undefined;
    for (const [branchId, hour] of shown.unheld) {
      const source = { sourceId: recording.id, revision: recording.revision, branchId, content };
      askTimeline(client, source, recording, hour);
      const at = askableAt(client, source);
      if (at !== undefined && at > Date.now()) wait = Math.min(wait ?? Infinity, at - Date.now());
    }
    if (shown.undrawn.length) drawFlags(client, recording, content, size, shown.undrawn);
    if (wait === undefined) return;
    return busyWait(() => setVersion(value => value + 1), wait);
  }, [shown, active]);
  const branches = useMemo(() => [...new Set(wanted.map(([branchId]) => branchId))], [wanted]);
  useEffect(() => {
    if (!active || sourceId === undefined || revision === undefined) return;
    const stops = branches.map(branchId => watchTimeline(client, { sourceId, revision, branchId }, () => {
      if (awaiting.current.unheld.has(branchId)) inFrame(() => setVersion(value => value + 1));
    }));
    stops.push(watchFlagDrawings(client, drawn => {
      for (const key of awaiting.current.awaited) if (drawn.has(key)) { setVersion(value => value + 1); return; }
    }));
    return () => { for (const stop of stops) stop(); };
  }, [active, client, sourceId, revision, branches]);
  const lastDrawn = useRef({ group: '', flags: new Map<string, FlagRead>() });
  const group = sourceId === undefined || revision === undefined ? '' : JSON.stringify([groupKey({ id: sourceId, revision }, size), content]);
  if (lastDrawn.current.group !== group) lastDrawn.current = { group, flags: new Map() };
  return useMemo(() => {
    const held = lastDrawn.current.flags;
    let values: Map<string, FlagValue> | undefined;
    for (const [branchId, hour, tag] of wanted) {
      const key = JSON.stringify([branchId, hour, tag]), country = JSON.stringify([branchId, tag]);
      const value = shown.values.get(key);
      if (value && !isFailedFact(value) && value.url) held.set(country, value);
      else if (value === undefined && !exact && held.has(country)) (values ??= new Map(shown.values)).set(key, held.get(country)!);
    }
    return values ?? shown.values;
  }, [shown, wanted, exact]);
}

export function useCountryFlagsRead(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, wanted: readonly NameAt[], active: boolean,
  size: CountryFlagSize = 'panel', exact = false): ReadonlyMap<string, FlagValue> {
  const branchId = track?.branch.id ?? '';
  const bySlot = wanted.some(item => typeof countryFromKey(item.tag) === 'number');
  const slots = useSlotTags(client, track, active && bySlot);
  const asked = useMemo(() => wanted.flatMap(({ tag, hour }) => {
    const slot = countryFromKey(tag);
    const shown = typeof slot === 'number' ? slots?.tagAt(slot, hour) ?? null : tag;
    return shown === null ? [] : [{ asked: tag, tag: shown, hour }];
  }), [wanted, slots]);
  const flags = useFlagReads(client, track?.source, asked.map(({ tag, hour }) => ({ tag, hour, branchId })), active && !!track, size, exact);
  return useMemo(() => {
    const keyed = new Map<string, FlagValue>();
    for (const [key, value] of flags) {
      const [, hour, tag] = JSON.parse(key) as [string, number, string];
      keyed.set(nameKey(hour, tag), value);
    }
    if (bySlot) for (const item of asked) if (item.asked !== item.tag && keyed.has(nameKey(item.hour, item.tag))) keyed.set(nameKey(item.hour, item.asked), keyed.get(nameKey(item.hour, item.tag))!);
    return keyed;
  }, [flags, asked, bySlot]);
}
