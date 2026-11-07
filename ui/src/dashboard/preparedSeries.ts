import { type Failure, asFailure } from './queryError.ts';
import { WALK_GATE, type DashboardClient, type DashboardSource } from './client.ts';
import { validatePreparationReply, type PreparationReply, type PreparedCoverage } from '../shell/preparation/contract.ts';
import { newId } from '../shell/ids.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContentRevision } from './contentRevision.ts';
import { finalPreparedReply, preparationCoverage } from './preparedPrefix.ts';
import { inFrame } from './frameUpdates.ts';
import { useViewRead } from './viewRead.ts';
import { replyDigest, keepDigest } from './replyDigest.ts';
import { countWork, measuredWidget } from './widgetPerf.ts';

export type PreparedSeriesKind = 'series-state' | 'series-summary' | 'series-next-change' | 'events-page' | 'events-clusters';

const PREPARED_PRODUCTS = ['production-lines', 'production-line-detail', 'production-line-breakdown', 'faction-state', 'land-battles', 'land-battles-manifest', 'land-battles-ranking', 'land-battle-course',
  'naval-battles', 'naval-battles-manifest', 'naval-battles-ranking', 'naval-battle-details',
  'division-log', 'division-log-manifest', 'division-log-totals', 'ships-sunk', 'ships-sunk-manifest', 'ships-sunk-battle-titles', 'naval-losses', 'special-projects', 'event-feed', 'air-missions', 'air-statistics', 'doctrine-mastery', 'intel-network-coverage', 'occupation-reserve-manpower', 'regional-weather', 'naval-supremacy', 'minefields', 'naval-repairs', 'naval-refits', 'country-facts', 'event-choices', 'decisions', 'fuel-branches', 'market-history', 'market-ledger', 'lend-lease-balance', 'production-catalogue', 'convoy-health', 'diplomacy-state', 'volunteers-expeditionaries', 'division-equipment-history',
  'equipment-stockpile-at', 'equipment-stockpile-catalogue', 'equipment-stockpile-chart', 'place-names', 'country-people'] as const;
export type PreparedProduct = typeof PREPARED_PRODUCTS[number];
export const isPreparedProduct = (product: string): product is PreparedProduct => (PREPARED_PRODUCTS as readonly string[]).includes(product);

/** Delay after the latest shown reply: 2 s, then 4 s, then 8 s until exact. */
export const preparedRefreshMs = (refreshes: number) => refreshes === 0 ? 2000 : refreshes === 1 ? 4000 : 8000;

const POLL_MS = 500;
const NO_HOURS: readonly number[] = [];
const pause = (signal: AbortSignal) => new Promise<void>((resolve, reject) => {
  if (signal.aborted) { reject(new DOMException('Campaign preparation cancelled', 'AbortError')); return; }
  const aborted = () => { clearTimeout(timer); reject(new DOMException('Campaign preparation cancelled', 'AbortError')); };
  const timer = setTimeout(() => { signal.removeEventListener('abort', aborted); resolve(); }, POLL_MS);
  signal.addEventListener('abort', aborted, { once: true });
});

/** A null query means pending coverage, never an empty balance or event page.
 * Status polling retries shown replies at 2/4/8 s after their last reply,
 * only when accepted progress changes; gate completion and the final read before
 * a terminal state bypass the delay. It stops
 * when an unchanged status follows a wait that took no real time. The progress
 * followed is the coverage of what the latest reply waits for (its gate): each
 * family prepared on its own it names, from the status's `families`, and the
 * hourly walk's `coverage` for `walk` or for a reply that names none. */
export async function preparedSeriesQuery(client: DashboardClient, source: DashboardSource, product: PreparedSeriesKind | 'focus-hour' | 'army-size' | 'supply-health' | 'country-groups' | 'idea-history' | 'subjects' | 'wars' | 'casualty-pairs' | 'equipment-losses' | 'politics-ledger' | 'occupied-states' | 'occupied-states-manifest' | 'land-owners' | PreparedProduct | 'party-history' | 'scoreboard' | 'military-air-ledger' | 'air-aces' | 'air-ace-portraits' | 'raids' | 'air-superiority' | 'unit-leaders' | 'espionage-log' | 'operative-placements' | 'operatives-roster' | 'agency-comparison' | 'decryption' | 'construction-log' | 'construction-log-manifest' | 'construction-spending' | 'resource-balance' | 'trade-flows' | 'production-spending' | 'dockyard-spending' | 'military-factory-spending' | 'casualty-breakdown', params: unknown,
  signal: AbortSignal, restart = false, pending: () => void = () => {},
  wait: (signal: AbortSignal) => Promise<void> = pause, publish?: (value: unknown) => void, now: () => number = () => performance.now()): Promise<unknown> {
  if (!client.preparedQuery) throw new Error('Prepared series queries are unavailable.');
  const preparedQuery = client.preparedQuery.bind(client);
  // The status entries the latest reply waits for; undefined for a reply that names none.
  let gate: readonly string[] | undefined;
  let shown = false, replies = 0, lastReply = 0;
  const ask = async () => {
    const reply = await preparedQuery<unknown>(source, product, params, { signal, refresh: shown, gate: entries => { gate = entries?.length ? entries : undefined; } });
    lastReply = now();
    if (shown) replies++;
    shown ||= reply !== null && !!publish;
    return reply;
  };
  let value = await ask();
  if (signal.aborted) throw new DOMException('Campaign preparation cancelled', 'AbortError');
  let published: string | undefined;
  const accept = (reply: unknown) => {
    if (reply === null) return false;
    // A refreshed reply the same as the one shown is not shown again.
    const digest = replyDigest(reply);
    if (digest === undefined || digest !== published) publish?.(reply);
    published = digest;
    // Focus has answered this point once its dated fields are prepared. Its
    // gate can keep preparing the rest of the branch without reloading the page.
    if (product === 'focus-hour' && reply && typeof reply === 'object') {
      const point = reply as { coverage?: unknown; preparedCoverage?: unknown; pending?: unknown[]; preparing?: unknown[]; approximate?: unknown[] };
      if (point.coverage !== null && point.preparedCoverage !== null && point.pending?.length === 0 && !point.preparing?.length && !point.approximate?.length) return true;
    }
    // One-shot consumers (navigation/picks) return immediately. Mounted views
    // continue replacing partial replies when committed coverage changes.
    return !publish || finalPreparedReply(reply);
  };
  if (accept(value)) return value;
  pending();
  if (!client.preparationStatus) throw new Error('Campaign preparation status is unavailable.');
  const selection = params && typeof params === 'object' && 'selection' in params ? params.selection : params;
  const branch = selection && typeof selection === 'object' && 'branchId' in selection && typeof selection.branchId === 'string' ? selection.branchId : undefined;
  const coverage = (reply: unknown): { firstHour: number; lastHour: number; complete: boolean } | null => {
    return preparationCoverage(reply) ?? null;
  };
  const coverageKey = (value: ReturnType<typeof coverage>) => value ? [value.firstHour, value.lastHour, value.complete] : null;
  const presentationKey = (reply: unknown) => {
    const fields = reply as { exact?: unknown; pending?: unknown; approximate?: unknown; preparing?: unknown; selectionCoverage?: unknown };
    // Selection can gain holders while numeric coverage stays put. Compare its
    // published extent without walking potentially transport-sized row arrays.
    return JSON.stringify([fields.exact, fields.pending ?? [], fields.approximate ?? [], fields.preparing ?? [], fields.selectionCoverage]);
  };
  const branchesKey = (map: Record<string, PreparedCoverage>) => branch ? coverageKey(map[branch] ?? null)
    : Object.entries(map).sort(([a], [b]) => a.localeCompare(b)).map(([id, value]) => [id, coverageKey(value)]);
  // The accepted progress the latest reply follows: its gate's coverage, the
  // walk's for a reply that names none, or section progress from a status
  // that lists no coverage for it.
  const progressKey = (status: PreparationReply) => {
    const maps = (gate ?? [WALK_GATE]).map(entry => entry === WALK_GATE ? status.coverage
      : status.families && (status.families[entry] ?? {}));
    if (maps.some(map => map === undefined))
      return JSON.stringify([status.state === 'ready', status.sections.map(section => [section.id, section.done, section.total])]);
    const keys = maps.map(map => branchesKey(map!));
    return JSON.stringify(gate ? gate.map((entry, index) => [entry, keys[index]]) : keys[0]);
  };
  // A partial reply already witnesses this branch's accepted coverage. Decoded
  // block counters and phase changes cannot justify reading that same prefix.
  // A reply waiting on several entries witnesses only their joint coverage.
  const witnessed = (reply: unknown) => {
    const covered = coverageKey(coverage(reply));
    return !gate ? JSON.stringify(covered) : gate.length === 1 ? JSON.stringify([[gate[0], covered]]) : undefined;
  };
  const gateComplete = (status: PreparationReply) => status.state === 'ready' || (gate ?? [WALK_GATE]).every(entry => {
    const map = entry === WALK_GATE ? status.coverage : status.families?.[entry];
    if (!map) return false;
    const values = branch ? [map[branch]] : Object.values(map);
    return values.length > 0 && values.every(value => value?.complete === true);
  });
  let progress = branch && value !== null ? witnessed(value) : undefined, started = false, completed = false;
  let observed = progress;
  let deferred: PreparationReply | undefined;
  const refresh = async (status: PreparationReply) => {
    deferred = undefined;
    const nextValue = await ask();
    if (signal.aborted) throw new DOMException('Campaign preparation cancelled', 'AbortError');
    // Followed under the gate the new reply names.
    progress = progressKey(status);
    completed = gateComplete(status);
    observed = progress;
    const before = coverage(value), after = coverage(nextValue);
    const unmoved = value !== null && publish && !finalPreparedReply(nextValue) && (nextValue === null
      || presentationKey(value) === presentationKey(nextValue) && (!after || before && after.firstHour >= before.firstHour
        && after.lastHour <= before.lastHour && (!after.complete || before.complete)));
    // Keep the prefix only while its extent and presentation stay unchanged.
    // Exact windows can grow before forward coverage moves. The next status
    // follows the new reply's gate, even if it shrank.
    if (unmoved) return false;
    value = nextValue;
    return accept(value);
  };
  try {
    while (!signal.aborted) {
      let changed = false;
      const request = { sourceId: source.id, revision: source.revision, requestId: newId() };
      const status = validatePreparationReply(await client.preparationStatus(request, signal), request);
      if (signal.aborted) break;
      // A pause resumes on its own (another recording first, disk space): wait it out.
      const terminal = ['failed', 'cancelled', 'idle'].includes(status.state);
      if (!started && client.prepareSource && restart && (terminal || status.state === 'paused')) {
        deferred = progressKey(status) !== progress ? status : undefined;
        started = true;
        validatePreparationReply(await client.prepareSource(request, signal), request);
        progress = undefined;
        observed = undefined;
        changed = true;
      } else {
        const next = progressKey(status);
        const complete = gateComplete(status);
        const moved = next !== progress || complete && !completed;
        changed = next !== observed || complete && !completed;
        observed = next;
        deferred = moved ? status : undefined;
        // A stopped preparation gets one last read even with an unchanged gate:
        // null can become unavailable, and a reached point can already be exact.
        if (terminal || status.state === 'ready' || moved && (!shown || complete || now() - lastReply >= preparedRefreshMs(replies))) {
          if (await refresh(status)) return value;
        }
        if (terminal) throw new Error(status.error ?? (status.state === 'idle' ? (status.backgroundEnabled ? 'Prepare this recording from Preparation and storage.' : 'Background preparation is off. Prepare this recording or enable it in Settings.') : status.state === 'cancelled' ? 'Campaign preparation cancelled.' : 'Campaign preparation failed.'));
        if (status.state === 'ready') throw new Error('Prepared series coverage is unavailable.');
      }
      const before = now();
      await wait(signal);
      if (signal.aborted) break;
      // A real wait can measure a fraction of a millisecond short of its delay.
      if (!changed && now() - before < POLL_MS / 2) throw new Error('Campaign preparation made no progress.');
    }
  } catch (error) {
    // Deliver progress held by the interval before a status/wait failure stops
    // this mounted read. Departed owners must never receive another publication.
    if (!signal.aborted && deferred && await refresh(deferred)) return value;
    throw error;
  }
  throw new DOMException('Campaign preparation cancelled', 'AbortError');
}

export interface PreparedBinding {
  id: string;
  spans: { tag: string; firstHour: number; lastHour: number; startsSegment: boolean }[];
}
const membership = (series: readonly PreparedBinding[]) => Object.fromEntries(series.map(({id,spans}) =>
  [id,spans.map(span=>[span.tag,span.firstHour,span.lastHour,span.startsSegment])]));

/** Reply-budget refusals split I/O, never the set of requested actors. */
export async function preparedWindowQuery<T>(client: DashboardClient,source: DashboardSource,family: 'autonomy'|'world-tension'|'naval-losses',params: object,
  view: { from: number; to: number; width: number; series: PreparedBinding[] },signal: AbortSignal,
  validate: (value: unknown) => T,merge: (a:T,b:T) => T,restart = false, publish?: (value: T) => void): Promise<T> {
  const read = async (series: PreparedBinding[], publish?: (value: T) => void): Promise<T> => {
    try { return validate(await preparedSeriesQuery(client,source,'series-summary',{
      ...params,family:{...(params as { family?: object }).family,kind:family},from:view.from,to:view.to,width:view.width,within:membership(series),
    },signal,restart,undefined,undefined,publish && (value => publish(validate(value))))); }
    catch (error) {
      if (signal.aborted || series.length < 2 || !/exceeds.*transport.*budget/i.test(String(error))) throw error;
      const middle = Math.ceil(series.length/2);
      if (!publish) {
        const first = await read(series.slice(0,middle));
        return merge(first,await read(series.slice(middle)));
      }
      const parts: { value?: T }[] = [{}, {}];
      const values = await Promise.all([series.slice(0,middle), series.slice(middle)].map((part, index) => read(part, value => {
        parts[index].value = value;
        publish(parts.every(part => part.value !== undefined) ? merge(parts[0].value!, parts[1].value!) : value);
      })));
      return merge(values[0], values[1]);
    }
  };
  return read(view.series, publish);
}

export function mergeReachedRows<R extends { firstHour: number; lastHour: number },T extends { runs:R[] }>(a: readonly T[],b: readonly T[],key: (row:T) => string): T[] {
  if (a.length !== b.length || a.some((row,index) => key(row) !== key(b[index]))) throw new Error('The prepared catalogue changed during the request.');
  return a.map((row,index) => ({...row,runs:[...new Map([...row.runs,...b[index].runs].map(run => [JSON.stringify(run),run])).values()]
    .sort((a,b) => a.firstHour-b.firstHour)}));
}

/** What every reply of one complete publication repeats whole, which a reader holding such a
 * reply asks to be left out (`omit`) and restores from it (`carry`) before validating. */
export interface Repeated<T> { omit: object; carry: (value: object, held: T) => object; whole: (reply: T) => boolean }

/** One source/content owner, one viewport and one set of exact reached hours.
 * Generic summaries page on the transport budget; exact reads are independent
 * of geometry. Page merging combines peers; an optional reached overlay can
 * reconcile point evidence differently. Departed source/view owners cannot
 * publish data or navigation. */
export function usePreparedWindow<T>(client: DashboardClient, source: DashboardSource | undefined, family: 'autonomy' | 'world-tension', params: object,
  bounds: readonly [number, number], hours: readonly number[], active: boolean, validate: (value: unknown) => T,
  merge: (a:T,b:T) => T, overlayReached: (geometry:T,reached:T) => T = merge, repeated?: Repeated<T>) {
  const content = useContentRevision(client);
  const widget = measuredWidget();
  const key = JSON.stringify([source?.id, source?.revision, family, params, content]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const current = useRef(owner); current.current = owner;
  // A reply of the complete publication, whose repeated parts later reads leave out.
  const whole = useRef<{ owner: typeof owner; reply: T } | null>(null);
  const readingFor = () => {
    const held = whole.current?.owner === owner && repeated ? whole.current.reply : undefined;
    return {
      params: held === undefined ? params : { ...params, ...repeated!.omit },
      validate: (value: unknown) => {
        const reply = keepDigest(value, validate(held !== undefined && value !== null && typeof value === 'object' ? repeated!.carry(value, held) : value));
        if (repeated?.whole(reply) && whole.current?.owner !== owner) whole.current = { owner, reply };
        return reply;
      },
    };
  };
  const navigation = useRef(new Set<AbortController>());
  const [view, setView] = useState<{ owner: typeof owner; from: number; to: number; width: number; series: PreparedBinding[] }>();
  const [summaries, setSummaries] = useState<{ owner: typeof owner; geometryKey: string; seriesKey: string; data?: T; error?: Failure }>();
  const [navigationError, setNavigationError] = useState<{ owner: typeof owner; error: Failure }>();
  const [attempt, setAttempt] = useState(0);
  const viewport = view?.owner === owner ? view : { from: bounds[0], to: bounds[1], width: 1, series: [] };
  const seriesKey = JSON.stringify(viewport.series);
  const geometryKey = JSON.stringify([viewport.from, viewport.to, viewport.width, viewport.series]);
  const pickOwner = useMemo(() => ({ owner,geometryKey }),[owner,geometryKey]);
  const currentPick = useRef(pickOwner); currentPick.current = pickOwner;
  const [keyboard, setKeyboard] = useState<{ owner: typeof pickOwner; hour: number | null; revision: number }>();
  const sequence = useRef(0);
  const readHours = [...new Set([...hours,...(keyboard?.owner===pickOwner && keyboard.hour!==null ? [keyboard.hour] : [])].map(Math.floor))].sort((a,b) => a-b);
  const readKey = JSON.stringify(readHours);
  // One summary read at a time per series selection: a view that moves on (playback,
  // a pan) lets the running read finish and be drawn, then asks for the newest view,
  // so a slow connection still shows the chart instead of cancelling every read.
  const flight = useRef<{ owner: typeof owner; lane: string; geometryKey: string; shown: boolean; controller: AbortController } | null>(null);
  const wanted = useRef({ viewport, geometryKey }); wanted.current = { viewport, geometryKey };
  const [landed, setLanded] = useState(0);
  useEffect(() => {
    if (!source || !active) return;
    const lane = JSON.stringify([seriesKey, attempt]);
    // A read still preparing publishes partial replies: once one is shown, a moved view replaces it.
    if (flight.current?.owner === owner && flight.current.lane === lane && (!flight.current.shown || flight.current.geometryKey === geometryKey)) return;
    flight.current?.controller.abort();
    const controller = new AbortController();
    const asked = wanted.current;
    const mine = { owner, lane, geometryKey: asked.geometryKey, shown: false, controller };
    flight.current = mine;
    const live = () => !controller.signal.aborted && current.current === owner;
    const publish = (data: T) => {
      if (!live()) return;
      inFrame(() => {
        if (!live()) return;
        setSummaries({ owner, geometryKey: asked.geometryKey, seriesKey, data });
        countWork(widget, 'reply');
        // Shown once it lands: until then a moved view waits for it rather than replacing it.
        if (!mine.shown) { mine.shown = true; if (wanted.current.geometryKey !== asked.geometryKey) setLanded(value => value + 1); }
      });
    };
    const reading = readingFor();
    countWork(widget, 'read');
    void preparedWindowQuery(client,source,family,reading.params,asked.viewport,controller.signal,reading.validate,merge,attempt > 0,publish).then(publish)
      .catch(error => { if (live()) inFrame(() => live() && setSummaries(previous => ({ ...(previous?.owner === owner ? previous : {}), owner, geometryKey: asked.geometryKey, seriesKey, error: asFailure(error) }))); })
      .finally(() => {
        if (flight.current?.controller !== controller) return;
        flight.current = null;
        if (!controller.signal.aborted && wanted.current.geometryKey !== asked.geometryKey) inFrame(() => setLanded(value => value + 1));
      });
  }, [owner, geometryKey, seriesKey, active, attempt, landed]);
  useEffect(() => () => { if (flight.current?.owner === owner) { flight.current.controller.abort(); flight.current = null; } }, [owner, active]);
  // Exact reads at the reached hours follow playback one at a time (`useViewRead`): each names
  // its hours, so a landed read stays drawn while the newest hours are read.
  const states = useViewRead<readonly number[], T>(source ? key : null, readHours, readKey, active,
    (at, signal, publish, restart) => {
      const reading = readingFor();
      return preparedSeriesQuery(client, source!, 'series-state', { ...reading.params, family: { kind: family }, reads: at.map(hour => ({ hour })) },
        signal, restart, undefined, undefined, value => publish(reading.validate(value))).then(reading.validate);
    }, client);
  const onSummaryView = useCallback((next: { from: number; to: number; width: number }, series: PreparedBinding[] = []) => setView(previous => previous?.owner === owner
    && previous.from === next.from && previous.to === next.to && previous.width === next.width && JSON.stringify(previous.series) === JSON.stringify(series) ? previous : { owner, ...next, series }), [owner]);
  const change = useCallback(async (tag:string,action:'ArrowLeft'|'ArrowRight'|'Home'|'End',from:number|null,first:number,last:number,binding?:PreparedBinding) => {
    if (!source || !active || first>last) return null;
    const controller = new AbortController(); navigation.current.add(controller);
    try {
      const series = tag === 'global' ? [] : binding ? [binding] : viewport.series.filter(series => series.id === tag);
      const value = await preparedSeriesQuery(client, source, 'series-next-change', { ...params,family:{kind:family},tag,
        within:membership(series),key:action,from,firstHour:first,lastHour:last }, controller.signal, attempt > 0) as { hour: number | null };
      if (!value || !(value.hour === null || Number.isSafeInteger(value.hour))) throw new Error('The prepared change reply is malformed.');
      return !controller.signal.aborted && current.current === owner ? value.hour : null;
    } finally { navigation.current.delete(controller); }
  }, [owner, geometryKey, active, attempt]);
  const navigate = useCallback((tag:string,from:number,forward:boolean,binding?:PreparedBinding) =>
    change(tag,forward ? 'ArrowRight' : 'ArrowLeft',from,viewport.from,viewport.to,binding),[change,geometryKey]);
  useEffect(() => () => { for (const controller of navigation.current) controller.abort(); navigation.current.clear(); }, [owner, geometryKey, active]);
  const onNavigateHour = useCallback((action: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: readonly [number,number], cutoff: number | null) => {
    const request = ++sequence.current;
    void change('global',action,from,Math.ceil(x[0]),Math.floor(Math.min(x[1],cutoff ?? x[1]))).then(hour => {
      if (request === sequence.current && currentPick.current === pickOwner) setKeyboard(previous => ({ owner: pickOwner,hour,revision: (previous?.revision ?? 0)+1 }));
    }).catch(error=> {if(!(error instanceof DOMException && error.name==='AbortError') && request===sequence.current && currentPick.current===pickOwner)setNavigationError({ owner, error: asFailure(error) });});
  },[change,pickOwner]);
  const geometry=summaries?.owner===owner ? summaries.data : undefined;
  const exact=states.data;
  const data=useMemo(()=>geometry && exact ? overlayReached(geometry,exact) : exact ?? geometry,[geometry,exact,overlayReached]);
  // The hours the shown exact read answers, ascending; the newest asked may still be reading.
  const answered=useMemo<readonly number[]>(()=>exact !== undefined && states.viewKey ? JSON.parse(states.viewKey) as number[] : NO_HOURS,[exact,states.viewKey]);
  return { data, answered, error: (summaries?.owner === owner ? summaries.error : undefined) ?? states.error ?? (navigationError?.owner === owner ? navigationError.error : undefined),
    pickOwner,onNavigateHour,keyboardHour: keyboard?.owner === pickOwner ? keyboard.hour : undefined, keyboardRevision: keyboard?.owner===pickOwner ? keyboard.revision : undefined,
    // Drawn while the selected series are the same; the chart places it by hour (`summaryFits`).
    geometryReady: !!geometry && summaries!.seriesKey === seriesKey,
    retry: () => { setNavigationError(undefined); states.retry(); setAttempt(value => value + 1); }, onSummaryView, navigate };
}
