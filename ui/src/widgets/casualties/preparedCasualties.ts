import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { CasualtyParams, CasualtyReply } from '../../generated/dashboard';
import { useContentRevision } from '../../dashboard/contentRevision';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { validateCasualtyReply } from './casualtiesData';

/** One reached-hour reply, owned by its source and selection. Pending playback
 * retains the previous reply at its own hour. */
export function usePreparedCasualties(client: DashboardClient, track: BoundTrack | undefined,
  params: CasualtyParams | null, hour: number, active: boolean) {
  const content = useContentRevision(client);
  const key = JSON.stringify([track?.source.id, track?.source.revision, params, content]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const [state, setState] = useState<{ owner: typeof owner; hour: number; data?: CasualtyReply; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track || !params) return;
    const controller = new AbortController();
    void preparedSeriesQuery(client, track.source, 'series-state', {
      branchId: params.branchId, tags: params.tags, family: { kind: 'war-casualties', top: params.top },
      reads: [{ tag: '', hour }],
    }, controller.signal, attempt > 0).then(value => {
      const data = validateCasualtyReply(value);
      if (!controller.signal.aborted) setState({ owner, hour, data });
    }).catch(error => {
      if (!controller.signal.aborted) setState({ owner, hour, error: String(error) });
    });
    return () => controller.abort();
    // The owner names the source, selection and content inputs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owner, hour, active, attempt]);
  const current = state?.owner === owner ? state : undefined;
  return { data: current?.data, hour: current?.data ? current.hour : hour, error: current?.error,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
