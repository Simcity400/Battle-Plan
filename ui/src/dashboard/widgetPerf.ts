import { createContext, use } from 'react';
import { perfLog, type WidgetWork } from '../shell/perf.ts';
import { widgetZones } from '../shell/commitCount.ts';

/** The widget whose components render below it, for a measured run's counts. */
export const MeasuredWidget = createContext<string | null>(null);

/** The widget the rendering component works for while measuring, null otherwise; call while rendering. */
export function measuredWidget(): string | null {
  // `use` may be called conditionally, so runs that measure nothing never read the context.
  return perfLog ? use(MeasuredWidget) : null;
}

/** Counts one piece of `widget`'s work toward the next frame record. */
export function countWork(widget: string | null, work: WidgetWork): void {
  if (widget !== null) perfLog?.count(widget, work);
}

widgetZones(MeasuredWidget);
