import { useMemo } from 'react';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { headRead, useOutsideTime } from './headProduct.ts';
import { outsideDetail, type ReadCoverage } from './widgetTime.ts';

export function useReadCoverage(runtime: Pick<WidgetRuntime, 'clock' | 'tracks'>, widget: WidgetSpec, hoverHour: number | null = null): ReadCoverage {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { hourAt } = useMemo(() => headRead(widget, track, runtime.tracks, hoverHour), [widget, track, runtime.tracks, hoverHour]);
  const outside = useOutsideTime(runtime.clock, !!track, hourAt);
  const detail = outside ? outsideDetail(widget, hoverHour) : undefined;
  return useMemo(() => ({ outside, outsideDetail: detail }), [outside, detail]);
}
