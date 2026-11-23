import { createContext, use, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { StructureRead } from '../shell/perf';
import type { PlaybackController } from './playback';
import type { Live } from '../charts/live';
import { perfLog, type WidgetWork } from '../shell/perf.ts';
import { noteCause, widgetZones } from '../shell/commitCount.ts';

export const MeasuredWidget = createContext<string | null>(null);

export function measuredWidget(): string | null {
  return perfLog ? use(MeasuredWidget) : null;
}

export function useRecordedRows(source: string, read: StructureRead | undefined, placement?: number) {
  const widget = measuredWidget(), previous = useRef<StructureRead>(undefined);
  useLayoutEffect(() => {
    if (widget && read && previous.current && previous.current.hour !== read.hour)
      perfLog?.structure(widget, source, previous.current, read, placement);
    previous.current = read;
  }, [widget, source, read, placement]);
}

export function countWork(widget: string | null, work: WidgetWork): void {
  if (widget !== null) perfLog?.count(widget, work);
}

export function countRead(widget: string | null, request: { params: unknown; asksFrom?: number }): void {
  if (widget !== null && perfLog) perfLog.count(widget, perfLog.ahead(request) ? 'ahead' : 'read');
}

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

export function useShownHour(): (hour: number | null) => void {
  const widget = measuredWidget();
  return useMemo(() => shownReporter(widget), [widget]);
}

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
