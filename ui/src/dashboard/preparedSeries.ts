import type { DashboardClient, DashboardSource } from './client';
import { validatePreparationReply } from '../shell/preparation/contract.ts';
import { newId } from '../shell/ids.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContentRevision } from './contentRevision.ts';

export type PreparedSeriesKind = 'series-state' | 'series-summary' | 'series-next-change' | 'events-page' | 'events-clusters';

const PREPARED_PRODUCTS = ['land-battles', 'land-battles-manifest', 'land-battles-ranking', 'land-battle-course',
  'naval-battles', 'naval-battles-manifest', 'naval-battles-ranking', 'naval-battle-details',
  'division-log', 'division-log-manifest', 'division-log-totals', 'ships-sunk', 'ships-sunk-manifest', 'ships-sunk-battle-titles', 'naval-losses', 'special-projects', 'event-feed'] as const;
export type PreparedProduct = typeof PREPARED_PRODUCTS[number];
export const isPreparedProduct = (product: string): product is PreparedProduct => (PREPARED_PRODUCTS as readonly string[]).includes(product);

const POLL_MS = 500;
const pause = (signal: AbortSignal) => new Promise<void>((resolve, reject) => {
  if (signal.aborted) { reject(new DOMException('Campaign preparation cancelled', 'AbortError')); return; }
  const aborted = () => { clearTimeout(timer); reject(new DOMException('Campaign preparation cancelled', 'AbortError')); };
  const timer = setTimeout(() => { signal.removeEventListener('abort', aborted); resolve(); }, POLL_MS);
  signal.addEventListener('abort', aborted, { once: true });
});

/** A null query means pending coverage, never an empty balance or event page.
 * Status polling retries reads only when accepted progress changes, and stops
 * when an unchanged status follows a wait that took no real time. */
export async function preparedSeriesQuery(client: DashboardClient, source: DashboardSource, product: PreparedSeriesKind | 'army-size' | 'supply-health' | 'country-groups' | 'idea-history' | 'subjects' | 'wars' | 'casualty-pairs' | 'equipment-losses' | 'politics-ledger' | 'occupied-states' | 'occupied-states-manifest' | 'land-owners' | PreparedProduct | 'party-history' | 'scoreboard' | 'military-air-ledger' | 'air-aces' | 'air-ace-portraits' | 'raids' | 'air-superiority' | 'unit-leaders' | 'espionage-log' | 'operative-placements' | 'operatives-roster' | 'agency-comparison' | 'decryption' | 'construction-log' | 'construction-log-manifest' | 'construction-spending' | 'resource-balance' | 'trade-flows' | 'production-spending' | 'casualty-breakdown' | 'equipment-stockpile', params: unknown,
  signal: AbortSignal, restart = false, pending: () => void = () => {},
  wait: (signal: AbortSignal) => Promise<void> = pause, publish?: (value: unknown) => void): Promise<unknown> {
  if (!client.preparedQuery) throw new Error('Prepared series queries are unavailable.');
  let value = await client.preparedQuery(source, product, params, { signal });
  if (signal.aborted) throw new DOMException('Campaign preparation cancelled', 'AbortError');
  const accept = (reply: unknown) => {
    if (reply === null) return false;
    publish?.(reply);
    // One-shot consumers (navigation/picks) return immediately. Mounted views
    // continue replacing partial replies when committed coverage changes.
    return !publish || !reply || typeof reply !== 'object' || !('coverage' in reply)
      || (reply as { coverage?: { complete: boolean } | null }).coverage?.complete === true;
  };
  if (accept(value)) return value;
  pending();
  if (!client.preparationStatus) throw new Error('Campaign preparation status is unavailable.');
  const branch = params && typeof params === 'object' && 'branchId' in params && typeof params.branchId === 'string' ? params.branchId : undefined;
  const coverage = (reply: unknown): { firstHour: number; lastHour: number; complete: boolean } | null => {
    if (!reply || typeof reply !== 'object' || !('coverage' in reply) || !reply.coverage) return null;
    return reply.coverage as { firstHour: number; lastHour: number; complete: boolean };
  };
  const coverageKey = (value: ReturnType<typeof coverage>) => value ? [value.firstHour, value.lastHour, value.complete] : null;
  // A partial reply already witnesses this branch's accepted coverage. Decoded
  // block counters and phase changes cannot justify reading that same prefix.
  let progress = branch && value !== null ? JSON.stringify(coverageKey(coverage(value))) : undefined, started = false;
  while (!signal.aborted) {
    let changed = false;
    const request = { sourceId: source.id, revision: source.revision, requestId: newId() };
    const status = validatePreparationReply(await client.preparationStatus(request, signal), request);
    if (signal.aborted) break;
    if (!started && client.prepareSource && restart && ['idle', 'failed', 'cancelled'].includes(status.state)) {
      started = true;
      validatePreparationReply(await client.prepareSource(request, signal), request);
      progress = undefined;
      changed = true;
    } else if (status.state === 'failed' || status.state === 'cancelled' || status.state === 'idle') {
      throw new Error(status.error ?? (status.state === 'idle' ? (status.backgroundEnabled ? 'Prepare this recording from Preparation and storage.' : 'Background preparation is off. Prepare this recording or enable it in Settings.') : status.state === 'cancelled' ? 'Campaign preparation cancelled.' : 'Campaign preparation failed.'));
    } else {
      const next = status.coverage !== undefined
        ? JSON.stringify(branch ? coverageKey(status.coverage[branch] ?? null)
          : Object.entries(status.coverage).sort(([a], [b]) => a.localeCompare(b)).map(([id, value]) => [id, coverageKey(value)]))
        : JSON.stringify([status.state === 'ready', status.sections.map(section => [section.id, section.done, section.total])]);
      if (next !== progress) {
        progress = next;
        changed = true;
        const nextValue = await client.preparedQuery(source, product, params, { signal });
        if (signal.aborted) break;
        if (value !== null && publish) {
          const before = coverage(value), after = coverage(nextValue);
          if (nextValue === null || !after || before && after.firstHour >= before.firstHour && after.lastHour <= before.lastHour && (!after.complete || before.complete))
            return value; // Retain the published partial reply; no repeated publication or retry cycle.
        }
        value = nextValue;
        if (accept(value)) return value;
      }
      if (status.state === 'ready') throw new Error('Prepared series coverage is unavailable.');
    }
    const before = Date.now();
    await wait(signal);
    if (signal.aborted) break;
    if (!changed && Date.now() - before < POLL_MS) throw new Error('Campaign preparation made no progress.');
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
  validate: (value: unknown) => T,merge: (a:T,b:T) => T,restart = false): Promise<T> {
  const read = async (series: PreparedBinding[]): Promise<T> => {
    try { return validate(await preparedSeriesQuery(client,source,'series-summary',{
      ...params,family:{...(params as { family?: object }).family,kind:family},from:view.from,to:view.to,width:view.width,within:membership(series),
    },signal,restart)); }
    catch (error) {
      if (signal.aborted || series.length < 2 || !/exceeds.*transport.*budget/i.test(String(error))) throw error;
      const middle = Math.ceil(series.length/2);
      const first = await read(series.slice(0,middle));
      return merge(first,await read(series.slice(middle)));
    }
  };
  return read(view.series);
}

export function mergeReachedRows<R extends { firstHour: number; lastHour: number },T extends { runs:R[] }>(a: readonly T[],b: readonly T[],key: (row:T) => string): T[] {
  if (a.length !== b.length || a.some((row,index) => key(row) !== key(b[index]))) throw new Error('The prepared catalogue changed during the request.');
  return a.map((row,index) => ({...row,runs:[...new Map([...row.runs,...b[index].runs].map(run => [JSON.stringify(run),run])).values()]
    .sort((a,b) => a.firstHour-b.firstHour)}));
}

/** One source/content owner, one viewport and one set of exact reached hours.
 * Generic summaries page on the transport budget; exact reads are independent
 * of geometry. Departed source/view owners cannot publish data or navigation. */
export function usePreparedWindow<T>(client: DashboardClient, source: DashboardSource | undefined, family: 'autonomy' | 'world-tension', params: object,
  bounds: readonly [number, number], hours: readonly number[], active: boolean, validate: (value: unknown) => T, merge: (a:T,b:T) => T) {
  const content = useContentRevision(client);
  const key = JSON.stringify([source?.id, source?.revision, family, params, content]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const current = useRef(owner); current.current = owner;
  const navigation = useRef(new Set<AbortController>());
  const [view, setView] = useState<{ owner: typeof owner; from: number; to: number; width: number; series: PreparedBinding[] }>();
  const [summaries, setSummaries] = useState<{ owner: typeof owner; geometryKey: string; data?: T; error?: string }>();
  const [states, setStates] = useState<{ owner: typeof owner; data?: T; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  const viewport = view?.owner === owner ? view : { from: bounds[0], to: bounds[1], width: 1, series: [] };
  const geometryKey = JSON.stringify([viewport.from, viewport.to, viewport.width, viewport.series]);
  const pickOwner = useMemo(() => ({ owner,geometryKey }),[owner,geometryKey]);
  const currentPick = useRef(pickOwner); currentPick.current = pickOwner;
  const [keyboard, setKeyboard] = useState<{ owner: typeof pickOwner; hour: number | null; revision: number }>();
  const sequence = useRef(0);
  const readHours = [...new Set([...hours,...(keyboard?.owner===pickOwner && keyboard.hour!==null ? [keyboard.hour] : [])].map(Math.floor))].sort((a,b) => a-b);
  const readKey = JSON.stringify(readHours);
  useEffect(() => {
    if (!source || !active) return;
    const controller = new AbortController();
    void preparedWindowQuery(client,source,family,params,viewport,controller.signal,validate,merge,attempt > 0).then(data => {
      if (!controller.signal.aborted && current.current === owner) setSummaries({ owner, geometryKey, data });
    }).catch(error => { if (!controller.signal.aborted && current.current === owner) setSummaries({ owner, geometryKey, error: String(error) }); });
    return () => controller.abort();
  }, [owner, geometryKey, active, attempt]);
  useEffect(() => {
    if (!source || !active) return;
    const controller = new AbortController();
    void preparedSeriesQuery(client,source,'series-state',{...params,family:{kind:family},reads:readHours.map(hour=>({hour}))},controller.signal,attempt>0)
      .then(value=> {if(!controller.signal.aborted && current.current===owner)setStates({owner,data:validate(value)});})
      .catch(error=> {if(!controller.signal.aborted && current.current===owner)setStates({owner,error:String(error)});});
    return ()=>controller.abort();
  }, [owner, readKey, active, attempt]);
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
    }).catch(error=> {if(!(error instanceof DOMException && error.name==='AbortError') && request===sequence.current && currentPick.current===pickOwner)setStates(previous=>({...previous,owner,error:String(error)}));});
  },[change,pickOwner]);
  const geometry=summaries?.owner===owner ? summaries.data : undefined;
  const exact=states?.owner===owner ? states.data : undefined;
  const data=useMemo(()=>geometry && exact ? merge(geometry,exact) : exact ?? geometry,[geometry,exact]);
  return { data, error: (summaries?.owner === owner ? summaries.error : undefined) ?? (states?.owner===owner ? states.error : undefined),
    pickOwner,onNavigateHour,keyboardHour: keyboard?.owner === pickOwner ? keyboard.hour : undefined, keyboardRevision: keyboard?.owner===pickOwner ? keyboard.revision : undefined,
    geometryReady: !!geometry && summaries!.geometryKey === geometryKey,
    retry: () => setAttempt(value => value + 1), onSummaryView, navigate };
}
