import type { BoundTrack } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { validateColumnsReply } from './scoreboardData.ts';
import type { ScoreboardColumnsReply } from '../../generated/dashboard';

/** The column registry of `track`'s branch: one request, kept while the recording and its content are. */
export function useScoreboardColumns(runtime: Pick<WidgetRuntime, 'client'>, track: BoundTrack | undefined, active: boolean) {
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard-columns', params: { branchId: track.branch.id } } : null;
  return useProduct<ScoreboardColumnsReply>(runtime.client, request, active, validateColumnsReply, track?.source);
}
