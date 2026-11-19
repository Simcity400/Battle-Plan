import { useCallback, useEffect, useMemo } from 'react';
import type { RefObject } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { followStore } from '../../dashboard/follow/connection.ts';
import type { FollowQuery } from '../../dashboard/follow/store';
import { useFallbackHour } from '../../dashboard/follow/useFollow.ts';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { useHeadHour } from '../army-size/headHour.ts';
import { useFollowedAll } from '../scoreboard/followedAll.ts';
import { useScoreboardColumns } from '../scoreboard/scoreboardColumns.ts';
import { rankedTags, rankingColumn, scoreboardParams, validateScoreboardReply } from '../scoreboard/scoreboardData.ts';
import type { ScoreboardReply } from '../../generated/dashboard';
import { replyTopTags } from './relationsData.ts';

const UNRANKED = { column: '', direction: 'descending' } as const;

export interface TopCountries {
  /** Greatest first; null until the ranking at the hour is read. */
  tags: readonly string[] | null;
  /** Why no ranking can be had in this recording. */
  unavailable: string | null;
  error: Error | undefined;
  retry: () => void;
}

/** A top N: the countries highest on a Scoreboard measure at the hour shown, as the Scoreboard ranks them. Following,
 * the ranking is a followed `series-state` top and the tags are structure; pinned,
 * or where the service does not follow it, one `scoreboard` reply ranks the hour. */
export function useTopCountries({ widget, runtime, active }: Pick<WidgetProps, 'widget' | 'runtime' | 'active'>, track: BoundTrack | undefined,
  top: number | null, measure: OptionValue, root: RefObject<Element | null>): TopCountries {
  const asked = top != null;
  const registry = useScoreboardColumns(runtime, track, active && asked);
  const columns = registry.data;
  const rank = asked && columns ? rankingColumn(columns.columns, UNRANKED, measure, columns.defaultSort) : undefined;
  const following = active && widget.time.mode === 'follow';
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]); // eslint-disable-line react-hooks/exhaustive-deps
  const branchId = track?.branch.id;
  const family = rank?.family ?? null;
  const rankQuery = useMemo<FollowQuery | null>(() => following && branchId && top != null && family
    ? { product: 'series-state', params: { branchId, family, top } } : null,
  [following, branchId, top, family?.kind, family?.measure]); // eslint-disable-line react-hooks/exhaustive-deps
  const ranking = useFollowedAll(store, useMemo(() => [rankQuery], [rankQuery]), root);
  const entry = ranking.entries[0] ?? null;
  const woken = useMemo(() => new Set<() => void>(), []);
  useEffect(() => {
    if (!store || !entry) return undefined;
    return store.watch(entry, () => { for (const wake of [...woken]) wake(); });
  }, [store, entry, woken]);
  const followed = following && rankQuery !== null && !ranking.reason;
  const structure = useCallback(() => entry?.answer ? rankedTags(entry.answer).join('\u0000') : '\u0001', [entry]);
  useHeadHour(runtime.clock, followed, structure, woken);
  const followedText = followed ? structure() : '\u0001';

  const playbackHour = useFallbackHour(runtime.clock, following && !followed);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const moment = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const params = !followed && track && rank ? scoreboardParams(track.branch.id, moment, [rank], [], top, rank) : null;
  const request = params && track ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard', params } : null;
  const validate = useCallback((value: unknown) => validateScoreboardReply(value, 1), []);
  const reply = useFollowedProduct<ScoreboardReply>(runtime.client, request, ['hour'], active, validate, track?.source);

  const replyData = followed ? undefined : reply.data;
  const tags = useMemo(() => followed ? (followedText === '\u0001' ? null : followedText ? followedText.split('\u0000') : [])
    : replyData ? replyTopTags(replyData) : null, [followed, followedText, replyData]);
  const unavailable = asked && columns && !rank ? 'No measure to rank the top countries by is in this recording.' : null;
  const retry = useCallback(() => { registry.retry(); reply.retry(); }, [registry.retry, reply.retry]); // eslint-disable-line react-hooks/exhaustive-deps
  return { tags: asked ? tags : null, unavailable, error: asked ? registry.error ?? (followed ? undefined : reply.error) : undefined, retry };
}
