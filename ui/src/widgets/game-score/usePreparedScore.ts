import { useEffect, useState } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useContentRevision } from '../../dashboard/contentRevision';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { object } from '../../dashboard/validation';
import { isPreparedCoverage } from '../../shell/preparation/contract';
import { validateGameScoreReply } from './gameScoreData';

export function validatePreparedScore(value: unknown) {
  if (!object(value) || typeof value.approximate !== 'boolean'
    || !(value.coverage === null || isPreparedCoverage(value.coverage))
    || !object(value.stretches) || !Object.values(value.stretches).every(stretch => typeof stretch === 'string')) {
    throw new Error('The prepared game score reply is malformed.');
  }
  return { reply: validateGameScoreReply(value.reply), stretches: value.stretches as Record<string, string>,
    approximate: value.approximate, coverage: value.coverage };
}

/** Draw the whole hour immediately, then refresh as accepted preparation advances. */
export function usePreparedScore(runtime: Pick<WidgetRuntime, 'client'>, track: BoundTrack | undefined, hour: number, active: boolean) {
  const { client } = runtime;
  const content = useContentRevision(client);
  const owner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const key = JSON.stringify([owner, hour]);
  const [state, setState] = useState<{ client: typeof client; owner: string; key: string; hour: number;
    data?: ReturnType<typeof validatePreparedScore>; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track) return;
    const controller = new AbortController();
    const publish = (value: unknown) => {
      if (!controller.signal.aborted) setState({ client, owner, key, hour, data: validatePreparedScore(value) });
    };
    void preparedSeriesQuery(client, track.source, 'series-state', {
      branchId: track.branch.id, family: { kind: 'game-score' }, reads: [{ hour }],
    }, controller.signal, attempt > 0, undefined, undefined, publish).catch(error => {
      if (!controller.signal.aborted) setState(previous => ({ client, owner, key,
        hour: previous?.client === client && previous.owner === owner && previous.data ? previous.hour : hour,
        data: previous?.client === client && previous.owner === owner ? previous.data : undefined,
        error: error instanceof Error ? error.message : String(error) }));
    });
    return () => controller.abort();
    // The key includes the full query identity, including the requested hour.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, client, key, attempt]);
  const current = owner && state?.client === client && state.owner === owner ? state : undefined;
  return { data: current?.data, error: current?.key === key ? current.error : undefined,
    hour: current?.data ? current.hour : hour, pending: current?.key !== key || current?.hour !== hour,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
