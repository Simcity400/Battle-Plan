import { createContext, use, useLayoutEffect, useMemo, useRef } from 'react';
import type { StructureRead } from '../shell/perf';
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

const lastShown = new Map<string, number | null>();

export function shownSink(): ((widget: string, hour: number | null) => void) | null {
  return perfLog ? (widget, hour) => perfLog?.shown(widget, hour) : null;
}

export function shownReporter(widget: string | null): (hour: number | null) => void {
  const sink = shownSink();
  if (!sink) return () => {};
  const key = widget ?? '';
  return hour => {
    const whole = hour === null ? null : Math.floor(hour);
    if (lastShown.has(key) && lastShown.get(key) === whole) return;
    lastShown.set(key, whole);
    sink(key, whole);
  };
}

export function useShownHour(): (hour: number | null) => void {
  const widget = measuredWidget();
  const inDrill = use(DrillScope);
  return useMemo(() => inDrill === 'quiet' ? () => {} : shownReporter(widget), [widget, inDrill]);
}

widgetZones(MeasuredWidget);
