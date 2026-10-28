import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { CasualtyParams, CasualtyReply } from '../../generated/dashboard';
import { useContentRevision } from '../../dashboard/contentRevision';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { preparedThrough } from '../../dashboard/preparedPrefix';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useViewRead } from '../../dashboard/viewRead';
import { validateCasualtyReply } from './casualtiesData';

/** One reached-hour reply, owned by its source and selection. Pending playback
 * retains the previous reply at its own hour. Before preparation reaches the
 * requested hour, the reply is the prepared prefix's totals at its last hour,
 * republished as the prefix grows: `hour` is then that last hour and `prefix`
 * is set. */
export function usePreparedCasualties(client: DashboardClient, track: BoundTrack | undefined,
  params: CasualtyParams | null, hour: number, active: boolean) {
  const content = useContentRevision(client);
  const key = track && params ? JSON.stringify([track.source.id, track.source.revision, params, content]) : null;
  // One read at a time while playback moves (`useViewRead`), so a slow read lands instead of being cancelled.
  const read = useViewRead<number, { hour: number; data: CasualtyReply }>(key, hour, String(hour), active, (at, signal, publish, restart) => {
    const reply = (value: unknown) => ({ hour: at, data: validateCasualtyReply(value) });
    return preparedSeriesQuery(client, track!.source, 'series-state', {
      branchId: params!.branchId, tags: params!.tags, family: { kind: 'war-casualties', top: params!.top },
      reads: [{ tag: '', hour: at }],
    }, signal, restart, undefined, undefined, value => publish(reply(value))).then(reply);
  }, client);
  const current = read.data;
  const answered = current ? preparedThrough([current.data], current.hour) : hour;
  // A failure belongs to the hour it was asked for; a newer hour is still loading.
  const error = useHeldFailure(read.errorKey === String(hour) ? read.error : undefined, current !== undefined, read.retry);
  return { data: current?.data, hour: answered, prefix: !!current && answered < current.hour, error, retry: read.retry };
}
