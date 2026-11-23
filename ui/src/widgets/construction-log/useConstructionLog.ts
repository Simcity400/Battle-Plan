import { useCallback, useMemo, useState } from 'react';
import type { LogRow } from '../../charts/log/logData';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useProductWindows, useWindowDisplay } from '../../dashboard/productWindows';
import { buildingChoices, constructionWindows, logReach, mergeWindows, validateConstructionLogManifest, validateConstructionLogWindow, windowAt } from './constructionData';
import type { ProjectAt } from './constructionData';
import type { ConstructionLogReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

function useManifest(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) {
  return useProduct(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'construction-log-manifest', params: { branchId: track.branch.id } }) : null, active, validateConstructionLogManifest, track?.source);
}

export function useConstructionLog(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean, hour: number | null, live: boolean) {
  const manifest = useManifest(runtime, track, active);
  const data = manifest.data;
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const current = data?.windows.length && hour != null ? windowAt(data.windows, hour) : -1;
  const [reach, setReach] = useState<{ identity: string | null; at: number; lower: number } | null>(null);
  const windows = useProductWindows<ConstructionLogReply>(runtime.client, identity, window => (productRequest({ sourceId: track!.source.id,
    revision: track!.source.revision, product: 'construction-log', params: { branchId: track!.branch.id, window } })),
  constructionWindows(data?.windows.length ?? 0, current, live, reach?.identity === identity ? reach : null),
  active && data?.recorded === true, validateConstructionLogWindow, track?.source);
  const display = useWindowDisplay(identity, windows.data.has(current) ? windows.data : undefined, hour ?? 0, runtime.client);
  const shown = data && display ? windowAt(data.windows, display.hour) : -1;
  const shownWindows = display?.data;
  const log = useMemo(() => data && shownWindows && shown >= 0 ? mergeWindows(data, shownWindows, shown) : null, [data, shownWindows, shown]);
  const onDrawn = useCallback((rows: readonly LogRow<ProjectAt>[], drawn: readonly LogRow<ProjectAt>[], from: number | null) => {
    if (!log) return;
    const lower = logReach(log, rows, drawn, from);
    setReach(previous => previous?.identity === identity && previous.at === log.current && previous.lower === lower ? previous
      : { identity, at: log.current, lower });
  }, [log, identity]);
  return { manifest: data, log, hour: display?.hour ?? null, error: manifest.error ?? windows.errors.values().next().value ?? null,
    retry: manifest.error ? manifest.retry : windows.retry, onDrawn };
}

export function useConstructionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  return { types: buildingChoices(useManifest(runtime, track, true).data) };
}
