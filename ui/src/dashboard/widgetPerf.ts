import { createContext, use, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { StructureRead } from '../shell/perf';
import type { PlaybackController } from './playback';
import type { Live } from '../charts/live';
import { perfLog, type WidgetWork } from '../shell/perf.ts';
import { noteCause, widgetZones } from '../shell/commitCount.ts';

/** The widget whose components render below it, for a measured run's counts. */
export const MeasuredWidget = createContext<string | null>(null);

/** The widget the rendering component works for while measuring, null otherwise; call while rendering. */
export function measuredWidget(): string | null {
  // `use` may be called conditionally, so runs that measure nothing never read the context.
  return perfLog ? use(MeasuredWidget) : null;
}

export function useRecordedRows(source: string, read: StructureRead | undefined) {
  const widget = measuredWidget(), previous = useRef<StructureRead>(undefined);
  useLayoutEffect(() => {
    if (widget && read && previous.current && previous.current.hour !== read.hour)
      perfLog?.structure(widget, source, previous.current, read);
    previous.current = read;
  }, [widget, source, read]);
}

/** Counts one piece of `widget`'s work toward the next frame record. */
export function countWork(widget: string | null, work: WidgetWork): void {
  if (widget !== null) perfLog?.count(widget, work);
}

/** Counts `request` asked by `widget`: a read, or one ahead when it asks for a block wholly ahead of the head. */
export function countRead(widget: string | null, request: { params: unknown; asksFrom?: number }): void {
  if (widget !== null && perfLog) perfLog.count(widget, perfLog.ahead(request) ? 'ahead' : 'read');
}

/** While measuring, names why `component` of the widget rendering it renders: each of `values` (its props, state and
 * hook results by their source names) that is another value than at its last render, written with the commit's causes
 * (`why` of the frame record). Not measuring, it only holds a ref. Call while rendering. */
export function useWhyCommit(component: string, values: Readonly<Record<string, unknown>>): void {
  const before = useRef<Readonly<Record<string, unknown>> | null>(null);
  if (!perfLog) return;
  const widget = measuredWidget();
  const prior = before.current;
  before.current = values;
  if (widget === null) return;
  const changed = prior ? Object.keys(values).filter(key => !Object.is(values[key], prior[key])) : ['mounted'];
  noteCause(`widget:${widget}`, `${component}: ${changed.join(', ') || 'nothing named'}`);
}

/** Tells a measured run the hour whose values `widget` now shows (null: none, loading or out of view); only a
 * change is written. Called from where the values are painted, never from a render. */
export function shownReporter(widget: string | null): (hour: number | null) => void {
  if (widget === null || !perfLog) return () => {};
  let last: number | null | undefined;
  return hour => {
    const whole = hour === null ? null : Math.floor(hour);
    if (whole === last) return;
    last = whole;
    perfLog?.shown(widget, whole);
  };
}

/** The reporter of the widget rendering this component: `const shown = useShownHour();` while rendering, then
 * `shown(hour)` in the painter with the hour of the values it painted. */
export function useShownHour(): (hour: number | null) => void {
  const widget = measuredWidget();
  return useMemo(() => shownReporter(widget), [widget]);
}

/** For a widget whose values are what it rendered (a pushed panel, a reply read once): reports `at()`, the hour
 * those values are of, at each frame of the clock, so a measured run sees the hour move without a render; after each
 * commit, so values that landed while paused are reported; and on each change of `live` (a panel's followed cells),
 * which moves the hour with neither a frame nor a render. */
export function useShownEachFrame(clock: Pick<PlaybackController, 'subscribeFrame'>, at: (() => number | null) | null,
  live?: Pick<Live<unknown>, 'subscribe'>): void {
  const shown = useShownHour();
  const latest = useRef(at);
  latest.current = at;
  useEffect(() => { if (perfLog && latest.current) shown(latest.current()); });
  const subscribeLive = live?.subscribe, enabled = !!at;
  useEffect(() => {
    if (!perfLog || !enabled) return undefined;
    const report = () => shown(latest.current?.() ?? null);
    const stops = [clock.subscribeFrame(report), ...subscribeLive ? [subscribeLive(report)] : []];
    return () => { for (const stop of stops) stop(); };
  }, [clock, shown, subscribeLive, enabled]);
}

widgetZones(MeasuredWidget);
