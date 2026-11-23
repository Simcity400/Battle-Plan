import { countryFromKey } from '../../dashboard/countryKey.ts';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { HourSpan, MissingSpan, SeriesState } from '../../generated/dashboard';
import type { Range } from '../../charts/view';
import type { ShareRun } from '../../charts/share';
import type { TimeChartProps } from '../../charts/TimeChart';
import { loadProduct, loadTransientProduct } from '../../dashboard/products';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';
import type { CoveredBranch, GatedHour } from '../../dashboard/regionTime';
import type { SeriesGate } from '../../dashboard/keyedSeries';
import { useTiles } from '../../dashboard/tiles/useTiles';
import { validateMeasureStates } from '../country-measure/measureSeriesData';
import type { LiveShares } from '../../charts/share';
import { followStore } from '../../dashboard/follow/store.ts';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { followFailure, followRefusal, followsHead, useFollow } from '../../dashboard/follow/useFollow.ts';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import type { PlaybackController } from '../../dashboard/playback';
import { DIVISIONS, SHARE_MEASURES, STATES, SUPPLY_FAMILY, liveShareRuns, mostDivisions, shareRunsFromLines, supplyAt } from './supplyHealthData';
import type { SupplyAt } from './supplyHealthData';
import { forwardRequest, type DeclaredProduct } from '../../dashboard/request.ts';
import type { ParamsFor } from '../../dashboard/regionTime';
import { withinParams } from '../../dashboard/keyedSeriesData.ts';

const NO_RUNS: ShareRun[] = [];

export function useSupplySeries(client: DashboardClient, clock: PlaybackController, gate: SeriesGate, source: DashboardSource | undefined, branch: (CoveredBranch & { id: string }) | undefined,
  tags: readonly string[], within: ReadonlyMap<string, readonly HourSpan[]> | undefined, lastHour: number, active: boolean,
  reads: readonly { tag: string; hour: GatedHour }[], following = false, root: { readonly current: Element | null } = { current: null }) {
  const branchId = branch?.id;
  const enabled = !!source && !!branchId && tags.length > 0;
  const ownerKey = JSON.stringify([source?.id, source?.revision, branchId, tags, within && [...within]]);
  const owner = useMemo(() => ({ client, key: ownerKey }), [client, ownerKey]);
  const alive = useRef(owner); alive.current = owner;
  const lane = (measure: string) => enabled ? { source: source!, branchId: branchId!, family: { kind: SUPPLY_FAMILY, measure }, tags, within, span: gate.span, lastHour } : null;
  const none = useTiles(client, lane(SHARE_MEASURES[0]), active, following);
  const strained = useTiles(client, lane(SHARE_MEASURES[1]), active, following);
  const held = useTiles(client, lane(SHARE_MEASURES[2]), active, following);
  const total = useTiles(client, lane(DIVISIONS), active, following);
  const track = gate.track;
  const store = useMemo(() => source && track ? followStore(client, track, clock) : null, [client, clock, source, track]);
  const headQuery = (measure: string) => following && active && enabled ? gate.follow({ product: 'series-state', params: { branchId,
    family: { kind: SUPPLY_FAMILY, measure }, reads: tags.map(tag => ({ country: countryFromKey(tag) })) } }) : null;
  const queries = useMemo(() => SHARE_MEASURES.map(headQuery), [following, active, enabled, branchId, tags, gate.follow]);
  const answered = useRef(new Set<() => void>()).current;
  const told = useCallback(() => { for (const listener of [...answered]) listener(); }, [answered]);
  const followed = [useFollow(store, queries[0], root, told), useFollow(store, queries[1], root, told), useFollow(store, queries[2], root, told)];
  const entries = followed.map(follow => following && followsHead(follow) ? follow.entry : null);
  const heldEntries = useRef(entries); heldEntries.current = entries;
  const answers = useMemo(() => ({ read: () => heldEntries.current.map(entry => entry && `${entry.id}:${entry.version}`),
    subscribe: (listener: () => void) => { answered.add(listener); return () => { answered.delete(listener); }; } }), [answered]);
  useEffect(() => {
    const shareTiles = [none, strained, held];
    const stops = entries.flatMap((entry, at) => entry ? tags.map((tag, row) => followAtHead(shareTiles[at].head(tag), () => {
      const decimals = shareTiles[at].read()?.get(tag)?.decimals;
      return decimals === undefined ? null : rowAmount(entry.answer, row, decimals) ?? null;
    })) : []);
    return () => stops.forEach(stop => stop());
  }, [entries[0], entries[1], entries[2], tags, none.head, strained.head, held.head]);
  const liveShares = useCallback((tag: string, clip: (runs: ShareRun[]) => ShareRun[]): LiveShares =>
    liveShareRuns(tag, clip, [none.live, strained.live, held.live], answers), [none.live, strained.live, held.live, answers]);
  const views = [none.onView, strained.onView, held.onView, total.onView];
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    if (view.to < view.from) return;
    for (const onView of views) onView(view);
  }, views);
  const shares = useMemo(() => new Map(tags.map(tag => [tag, shareRunsFromLines([none.lines, strained.lines, held.lines]
    .map(lines => lines?.get(tag)?.summary))])), [tags, none.lines, strained.lines, held.lines]);
  const missing = useMemo(() => new Map(tags.map(tag => [tag, none.lines?.get(tag)?.missing ?? [] as MissingSpan[]])), [tags, none.lines]);
  const drawn = none.lines !== null && strained.lines !== null && held.lines !== null && total.lines !== null;
  const divisions = useMemo(() => new Map(tags.map(tag => [tag, mostDivisions(total.lines?.get(tag)?.summary)])), [tags, total.lines]);
  const request = <K extends DeclaredProduct>(product: K, measure: string, params: ParamsFor<K>) => forwardRequest(source!, product,
    { branchId, family: { kind: SUPPLY_FAMILY, measure }, ...params } as unknown as ParamsFor<K>);
  const content = useContentRevision(client);
  const asked = useMemo(() => [...new Map(reads.map(read => [`${read.tag}\u0000${read.hour}`, read] as const)).values()], [JSON.stringify(reads)]);
  const stateRead = useViewRead<typeof asked, readonly (readonly SeriesState[])[]>(enabled && asked.length ? JSON.stringify([ownerKey, content]) : null,
    asked, JSON.stringify(asked), active, (at, signal, _publish, restart) => Promise.all(STATES.map(measure =>
      loadProduct(client, request('series-state', measure, { reads: at.map(read => ({ country: countryFromKey(read.tag), hour: read.hour })) }), signal, validateMeasureStates, source, restart)
        .then(reply => reply.states))), client);
  const [failure, setFailure] = useState<{ owner: typeof owner; error: Failure }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const navigation = useRef<AbortController | null>(null);
  useEffect(() => () => navigation.current?.abort(), [owner, active]);
  const onNavigateHour = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => {
    if (!enabled || !active) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    const range = gate.span(x[0], Math.min(x[1], until ?? x[1]));
    if (!range) return;
    void loadTransientProduct<{ hour: unknown }>(client, request('series-next-change', STATES[0], { countries: tags.map(countryFromKey), ...withinParams(tags, within, gate.span), measures: STATES.slice(1), key,
      from: gate.within(from), ...range }), controller.signal).then(reply => {
      if (controller.signal.aborted || alive.current !== owner) return;
      if (reply.hour !== null && (typeof reply.hour !== 'number' || !Number.isFinite(reply.hour))) throw new Error('The keyboard reply is malformed.');
      setKeyboard(previous => ({ owner, hour: reply.hour as number | null, revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && alive.current === owner) setFailure({ owner, error: asFailure(error) }); });
  }, [owner, active, enabled, gate.span, gate.within]);
  const states = stateRead.data;
  const at = useCallback((tag: string, hour: number): SupplyAt => supplyAt(states, tag, Math.floor(hour)), [states]);
  const retry = () => { setFailure(undefined); none.retry(); strained.retry(); held.retry(); total.retry(); stateRead.retry(); };
  const stateError = stateRead.errorKey === JSON.stringify(asked) ? stateRead.error : undefined;
  const tileError = none.error ?? strained.error ?? held.error ?? total.error;
  const followErr = following ? followFailure(...followed) : null;
  const retryAll = () => { followErr?.retry(); retry(); };
  const error = useHeldFailure(followErr?.error ?? (failure?.owner === owner ? failure.error : undefined) ?? stateError ?? (tileError ? asFailure(tileError) : undefined),
    states !== undefined || drawn, retryAll);
  return { refused: following ? followRefusal(...followed) : null, enabled, owner, drawn, shares, liveShares, sharesOf: (tag: string) => shares.get(tag) ?? NO_RUNS, missing, divisions, at, onSummaryView, onNavigateHour,
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined, keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    error, retry: retryAll, notRecorded: none.unrecorded && strained.unrecorded && held.unrecorded ? none.unrecorded : null };
}
