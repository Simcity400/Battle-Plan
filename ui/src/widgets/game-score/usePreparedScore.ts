import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useContentRevision } from '../../dashboard/contentRevision';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { object } from '../../dashboard/validation';
import { isPreparationExtent } from '../../dashboard/preparedPrefix';
import { useViewRead } from '../../dashboard/viewRead';
import { keepDigest } from '../../dashboard/replyDigest';
import { validateGameScoreReply } from './gameScoreData';

export function validatePreparedScore(value: unknown) {
  if (!object(value) || typeof value.approximate !== 'boolean'
    || !isPreparationExtent(value.coverage)
    || !object(value.stretches) || !Object.values(value.stretches).every(stretch => typeof stretch === 'string')) {
    throw new Error('The prepared game score reply is malformed.');
  }
  if (value.stableThrough !== undefined && !Number.isSafeInteger(value.stableThrough)) throw new Error('The prepared game score reply is malformed.');
  return { reply: validateGameScoreReply(value.reply), stretches: value.stretches as Record<string, string>,
    approximate: value.approximate, coverage: value.coverage, stableThrough: value.stableThrough as number | undefined };
}

/** Draw the whole hour immediately, then refresh as accepted preparation advances. One read
 * runs at a time while playback moves (`useViewRead`): the last hour stays shown while the
 * newest is read, and a slow read lands instead of being cancelled every hour. */
export function usePreparedScore(runtime: Pick<WidgetRuntime, 'client'>, track: BoundTrack | undefined, hour: number, active: boolean) {
  const { client } = runtime;
  const content = useContentRevision(client);
  const owner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const read = useViewRead<number, { hour: number; data: ReturnType<typeof validatePreparedScore> }>(owner, hour, String(hour), active,
    (at, signal, publish, restart) => preparedSeriesQuery(client, track!.source, 'series-state', {
      branchId: track!.branch.id, family: { kind: 'game-score' }, reads: [{ hour: at }],
    }, signal, restart, undefined, undefined, value => publish(keepDigest(value, { hour: at, data: validatePreparedScore(value) })))
      .then(value => keepDigest(value, { hour: at, data: validatePreparedScore(value) })), client);
  return { data: read.data?.data, error: read.errorKey === String(hour) ? read.error : undefined,
    hour: read.data ? read.data.hour : hour, pending: read.viewKey !== String(hour), retry: read.retry };
}
