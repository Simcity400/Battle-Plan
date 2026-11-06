import { defaultCountryTag } from './countrySelection';
import type { DashboardClient } from './client';
import type { WidgetRuntime } from './definitions';
import { resolveWindow, type BoundTrack, type WidgetSpec } from './model';
import { useContentRevision } from './contentRevision';
import { preparedSeriesQuery } from './preparedSeries';
import type { IdeaHistoryFirstView } from './ideaHistoryReply';
import { validateIdeaHistoryReply } from './ideaHistoryReply';
import { usePlaybackHour } from './playback.ts';
import { useViewRead } from './viewRead.ts';

/** Prepared holdings with visibly approximate keyframe lanes beyond coverage;
 * the icon grid instead requests an exact held set at its displayed hour. */
export function useIdeaHistory(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const through = window ? Math.floor(window.lastHour) : undefined;
  return { track, tag, ...usePreparedIdeaHistory(runtime.client, track, tag, through, widget.kind === 'national-spirits', active) };
}

/** One country's prepared idea history through an hour: lanes, or the held set at that hour.
 * The hour follows playback (`useViewRead`): the last landed history stays shown while the newest is read. */
export function usePreparedIdeaHistory(client: DashboardClient, track: BoundTrack | undefined, tag: string | undefined, through: number | undefined, held: boolean, active: boolean) {
  const content = useContentRevision(client);
  const lane = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, held, content]) : null;
  const read = useViewRead<{ through: number | undefined }, IdeaHistoryFirstView>(lane, lane ? { through } : null, String(through), active,
    (at, signal, publish, restart) => preparedSeriesQuery(client, track!.source, 'idea-history',
      { branchId: track!.branch.id, tag: tag!, through: at.through, preview: !held, hour: held ? at.through : undefined }, signal, restart,
      undefined, undefined, value => publish(validateIdeaHistoryReply(value))).then(validateIdeaHistoryReply), client);
  // The hour the shown history was read through: a retained reply answers it, not the playhead.
  const answered = read.data === undefined || read.viewKey === undefined || read.viewKey === 'undefined' ? undefined : Number(read.viewKey);
  return { data: read.data, answered, error: read.error, retry: read.retry };
}
