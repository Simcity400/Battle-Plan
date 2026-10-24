import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { Range } from '../../charts/view';
import type { HourSpan } from '../../generated/dashboard';
import type { TimeChartProps } from '../../charts/TimeChart';
import { reachedMeasureReply, validateMeasureStates, validateMeasureSummaries } from './preparedMeasureData';
import type { MeasureStates, MeasureSummaries } from './preparedMeasureData';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';

/** One viewport reply and one set of reached exact hours; no campaign history
 * cache. Request ownership masks source changes before passive cleanup runs. */
export function usePreparedMeasure<M extends string>(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  measure: M, tags: readonly string[], headHour: number, hoverHour: number | null, active: boolean, withMarks: boolean, family: 'country-measure' | 'country-stability' | 'country-experience' | 'stability-parts' = 'country-measure', navigationMeasures?: readonly string[], within?: ReadonlyMap<string, readonly HourSpan[]>, withStateReads = true) {
  const query = client.preparedQuery;
  const enabled = !!query && !!source && !!branchId;
  const ownerKey = JSON.stringify([source?.id, source?.revision, branchId, family, measure, tags, navigationMeasures, within && [...within], withStateReads]);
  const owner = useMemo(() => ({ client, key: ownerKey }), [client, ownerKey]);
  const alive = useRef(owner); alive.current = owner;
  const [viewport, setViewport] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const [states, setStates] = useState<{ owner: typeof owner; data?: MeasureStates; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  const [pendingSummary, setPendingSummary] = useState<{ owner: typeof owner; pending: boolean }>();
  const [pendingStates, setPendingStates] = useState<{ owner: typeof owner; pending: boolean }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const navigation = useRef<AbortController | null>(null);
  const picks = useRef(new Set<AbortController>());
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    if (view.to < view.from) return;
    setViewport(previous => previous?.owner === owner && previous.from === view.from && previous.to === view.to && previous.width === view.width
      ? previous : { owner, ...view });
  }, [owner]);
  const view = viewport?.owner === owner ? viewport : undefined;
  // One summary read at a time while the chart's view moves (`useViewRead`).
  const content = useContentRevision(client);
  const summaries = useViewRead<{ from: number; to: number; width: number }, MeasureSummaries>(enabled ? JSON.stringify([ownerKey, withMarks, content]) : null,
    view, JSON.stringify([view?.from, view?.to, view?.width]), active, (at, signal, publish) => preparedSeriesQuery(client, source!, 'series-summary',
      { branchId, family: { kind: family, measure }, tags, from: at.from, to: at.to, width: at.width, withMarks, within: within && Object.fromEntries(within) }, signal, attempt > 0,
      () => { if (!signal.aborted && alive.current === owner) setPendingSummary({ owner, pending: true }); }, undefined,
      value => publish(validateMeasureSummaries(value))).then(value => {
      if (!signal.aborted && alive.current === owner) setPendingSummary({ owner, pending: false });
      return validateMeasureSummaries(value);
    }), client);
  const currentSummaries = summaries.data;
  const summaryByTag = useMemo(() => new Map(currentSummaries?.series.map(series => [series.tag, series]) ?? []), [currentSummaries]);
  const reads = withStateReads ? tags.flatMap(tag => {
    const line = summaryByTag.get(tag)?.summary;
    let end: number | undefined;
    if (line) for (let at = line.buckets.length - 1; at >= 0; at--) if (line.buckets[at]) { end = line.buckets[at]!.lastHour; break; }
    return [...new Set([Math.floor(headHour), ...(hoverHour === null ? [] : [Math.floor(hoverHour)]), ...(end === undefined ? [] : [end])])].map(hour => ({ tag, hour }));
  }) : [];
  const readKey = JSON.stringify(reads);
  useEffect(() => {
    if (!enabled || !active || !reads.length) return;
    const controller = new AbortController();
    void preparedSeriesQuery(client, source!, 'series-state', { branchId, family: { kind: family, measure }, reads }, controller.signal, attempt > 0,
      () => { if (!controller.signal.aborted && alive.current === owner) setPendingStates({ owner, pending: true }); }, undefined,
      value => { if (!controller.signal.aborted && alive.current === owner) setStates({ owner, data: validateMeasureStates(value) }); }).then(
      value => { if (!controller.signal.aborted && alive.current === owner) { setStates({ owner, data: validateMeasureStates(value) }); setPendingStates({ owner, pending: false }); } }).catch(
      error => { if (!controller.signal.aborted && alive.current === owner) setStates(previous => ({ ...previous, owner, error: String(error) })); },
    );
    return () => controller.abort();
  }, [owner, readKey, active, enabled, attempt]);
  useEffect(() => () => {
    navigation.current?.abort();
    for (const controller of picks.current) controller.abort(); picks.current.clear();
  }, [owner, active]);
  const onNavigateHour = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => {
    if (!enabled || !active) return;
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    void preparedSeriesQuery(client, source!, 'series-next-change', { branchId, family: { kind: family, measure }, tags, measures: navigationMeasures, within: within && Object.fromEntries(within), key, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) }, controller.signal, attempt > 0).then(value => {
      const reply = value as { hour?: unknown };
      if (controller.signal.aborted || alive.current !== owner) return;
      if (reply.hour !== null && (typeof reply.hour !== 'number' || !Number.isFinite(reply.hour))) throw new Error('The prepared keyboard reply is malformed.');
      setKeyboard(previous => ({ owner, hour: reply.hour as number | null, revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && alive.current === owner) setStates(previous => ({ ...previous, owner, error: String(error) })); });
  }, [owner, active, enabled, attempt]);
  const exactAt = useCallback(async (hour: number): Promise<MeasureStates | null> => {
    if (!enabled || !active) return null;
    const controller = new AbortController(); picks.current.add(controller);
    try {
      const value = await preparedSeriesQuery(client, source!, 'series-state', { branchId, family: { kind: family, measure }, reads: tags.map(tag => ({ tag, hour: Math.floor(hour) })) }, controller.signal, attempt > 0);
      return controller.signal.aborted || alive.current !== owner ? null : validateMeasureStates(value);
    } catch (error) {
      if (!controller.signal.aborted && alive.current === owner) setStates(previous => ({ ...previous, owner, error: String(error) }));
      return null;
    } finally { picks.current.delete(controller); }
  }, [owner, active, enabled, attempt]);
  const currentStates = states?.owner === owner ? states.data : undefined;
  const data = useMemo(() => enabled && currentStates ? reachedMeasureReply(branchId!, measure, tags, currentStates) : undefined,
    [owner, enabled, currentStates]);
  const retry = () => { setAttempt(previous => previous + 1); summaries.retry(); };
  const error = useHeldFailure((states?.owner === owner ? states.error : undefined) ?? summaries.error,
    data !== undefined || currentSummaries !== undefined, retry);
  return { enabled, owner, data, states: currentStates, summaries: currentSummaries, summaryByTag, onSummaryView, onNavigateHour, exactAt,
    pending: currentSummaries?.pending,
    preparing: active && (pendingSummary?.owner === owner && pendingSummary.pending || pendingStates?.owner === owner && pendingStates.pending),
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    hourLoaded: (tag: string, hour: number) => currentStates?.states.some(state => state.tag === tag && state.hour === hour && !state.approximate) ?? false,
    error,
    retry,
  };
}
