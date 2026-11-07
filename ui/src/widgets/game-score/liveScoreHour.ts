import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { finalPreparedReply } from '../../dashboard/preparedPrefix';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { sharedRead } from '../../live/sharedReads';
import { gameScoreModel, gameScoreRows } from './gameScoreData';
import type { ScoreRow } from './gameScoreData';
import { validatePreparedScore } from './usePreparedScore';

/** Every country's game score at `hour`, ranked by total, its rows carrying the stretch they glide
 * within: read and shaped once however many live views (table, race) show it. */
let restarts = 0;

export function liveScoreHour(runtime: WidgetRuntime, track: BoundTrack, content: number, hour: number, signal: AbortSignal, restart = false) {
  // A retry's read is its own: it neither joins nor leaves a read that may not restart preparation.
  const key = JSON.stringify(['game-score', track.source.id, track.source.revision, track.branch.id, content, hour, restart ? ++restarts : 0]);
  return sharedRead(key, signal, async shared => {
    const raw = await preparedSeriesQuery(runtime.client, track.source, 'series-state',
      { branchId: track.branch.id, family: { kind: 'game-score' }, reads: [{ hour }] }, shared, restart);
    const prepared = validatePreparedScore(raw);
    const model = gameScoreModel(prepared.reply);
    const rows: ScoreRow[] = gameScoreRows(model, model.tags, hour, { column: 'total', direction: 'descending' })
      .map(row => ({ ...row, stretch: prepared.stretches[row.tag] ?? row.stretch }));
    return { prepared, model, rows, through: Math.max(hour, prepared.stableThrough ?? hour), partial: !finalPreparedReply(raw) };
  }, shaped => !shaped.partial);
}
