import { useCallback, useMemo, useState } from 'react';
import type { LogRow } from '../../charts/log/logData';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useProductWindows, useWindowDisplay } from '../../dashboard/productWindows';
import type { ConstructionLogManifestReply, ConstructionLogReply } from '../../generated/dashboard';
import { buildingChoices, constructionWindows, logReach, mergeWindows, validateConstructionLogManifest, validateConstructionLogWindow, windowAt } from './constructionData';
import type { ProjectAt } from './constructionData';

/** The branch's construction log manifest: its windows and the building types its projects name. */
function useManifest(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean) {
  return useProduct<ConstructionLogManifestReply>(runtime.client, track ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'construction-log-manifest', params: { branchId: track.branch.id } } : null, active, validateConstructionLogManifest);
}

/** The construction log at `hour` (null outside the branch): the windows of the
 * branch's blocks it holds, merged. It holds the hour's window, the next during
 * playback, and back as far as the drawn rows need (`onDrawn`); while the hour's
 * window loads it keeps showing the last hour it held. */
export function useConstructionLog(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean, hour: number | null, live: boolean) {
  const manifest = useManifest(runtime, track, active);
  const data = manifest.data;
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const current = data?.windows.length && hour != null ? windowAt(data.windows, hour) : -1;
  const [reach, setReach] = useState<{ identity: string | null; at: number; lower: number } | null>(null);
  const types = data?.types.length ?? 0;
  const windows = useProductWindows<ConstructionLogReply>(runtime.client, identity, window => ({ sourceId: track!.source.id,
    revision: track!.source.revision, product: 'construction-log', params: { branchId: track!.branch.id, window } }),
  constructionWindows(data?.windows.length ?? 0, current, live, reach?.identity === identity ? reach : null),
  active && data?.recorded === true, reply => validateConstructionLogWindow(reply, types));
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
  // A window it holds failing leaves rows unknown, so it shows the failure.
  return { manifest: data, log, hour: display?.hour ?? null, error: manifest.error ?? windows.errors.values().next().value ?? null,
    retry: manifest.error ? manifest.retry : windows.retry, onDrawn };
}

/** The Building types setting's choices: the types this campaign's projects name, as
 * the game names and pictures them (never a fixed list). */
export function useConstructionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  return { types: buildingChoices(useManifest(runtime, track, true).data) };
}
