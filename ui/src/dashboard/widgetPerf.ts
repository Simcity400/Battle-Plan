import { createContext, use, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { StructureRead } from '../shell/perf';
import type { PlaybackController } from './playback';
import type { Live } from '../charts/live';
import { perfLog, type WidgetWork } from '../shell/perf.ts';
import { widgetZones } from '../shell/commitCount.ts';

export const MeasuredWidget = createContext<string | null>(null);
export const DrillScope = createContext<false | 'owns' | 'quiet'>(false);

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

const drilled = new Map<string, number>();
const lastShown = new Map<string, number | null>();

export function shownSink(): ((widget: string, hour: number | null) => void) | null {
  return perfLog ? (widget, hour) => perfLog?.shown(widget, hour) : null;
}

export function shownReporter(widget: string | null, drill = false): (hour: number | null) => void {
  const sink = shownSink();
  if (!sink) return () => {};
  const key = widget ?? '';
  return hour => {
    if (!drill && drilled.has(key)) return;
    const whole = hour === null ? null : Math.floor(hour);
    if (lastShown.has(key) && lastShown.get(key) === whole) return;
    lastShown.set(key, whole);
    sink(key, whole);
  };
}

export function useShownHour(): (hour: number | null) => void {
  const widget = measuredWidget();
  const inDrill = use(DrillScope);
  return useMemo(() => inDrill === 'quiet' ? () => {} : shownReporter(widget, inDrill === 'owns'), [widget, inDrill]);
}

export function useDrillShown(on: boolean): void {
  const widget = measuredWidget();
  const shown = useMemo(() => shownReporter(widget, true), [widget]);
  const owns = on && shownSink() !== null, key = widget ?? '';
  useLayoutEffect(() => {
    if (!owns) return undefined;
    drilled.set(key, (drilled.get(key) ?? 0) + 1);
    return () => {
      const left = (drilled.get(key) ?? 1) - 1;
      if (left > 0) drilled.set(key, left); else drilled.delete(key);
    };
  }, [key, owns]);
  useEffect(() => { if (owns) shown(null); }, [owns, shown]);
}

export function ShownHour({ clock, at }: { clock: Pick<PlaybackController, 'subscribeFrame'>; at: number | null }): null {
  useShownEachFrame(clock, () => at);
  return null;
}

export function useShownEachFrame(clock: Pick<PlaybackController, 'subscribeFrame'>, at: (() => number | null) | null,
  live?: Pick<Live<unknown>, 'subscribe'>): void {
  const shown = useShownHour();
  const latest = useRef(at);
  latest.current = at;
  const measured = shownSink() !== null;
  useEffect(() => { if (measured && latest.current) shown(latest.current()); });
  const subscribeLive = live?.subscribe, enabled = measured && !!at;
  useEffect(() => {
    if (!enabled) return undefined;
    const report = () => shown(latest.current?.() ?? null);
    const stops = [clock.subscribeFrame(report), ...subscribeLive ? [subscribeLive(report)] : []];
    return () => { for (const stop of stops) stop(); };
  }, [clock, shown, subscribeLive, enabled]);
}

widgetZones(MeasuredWidget);
