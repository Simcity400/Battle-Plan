import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';
import type { ResearchReply } from '../../generated/dashboard';
import { validateResearchReply } from './researchData.ts';

/** The country's research history. Before preparation covers the branch,
 * `publish` receives each reply answered from its prepared prefix as it grows. */
export async function loadPreparedResearch(client: DashboardClient, source: DashboardSource, branchId: string, tag: string,
  signal: AbortSignal, restart = false, pending: () => void = () => {}, publish?: (reply: ResearchReply) => void): Promise<ResearchReply> {
  return validateResearchReply(await preparedSeriesQuery(client, source, 'events-page', {
    branchId, tag, family: { kind: 'research' },
  }, signal, restart, pending, undefined, publish && (value => publish(validateResearchReply(value)))));
}

/** Complete selected-country rows keep the established list and column models.
 * Pending coverage, cancellation and source ownership use the shared query path;
 * before coverage is complete the rows are its prepared prefix's. */
export function usePreparedResearch(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  tag: string | undefined, active: boolean) {
  const content = useContentRevision(client);
  const key = JSON.stringify([source?.id, source?.revision, branchId, tag, content]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const [state, setState] = useState<{ owner: typeof owner; data?: ResearchReply; error?: string; preparing?: boolean }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !source || !branchId || !tag) return;
    const controller = new AbortController();
    void loadPreparedResearch(client, source, branchId, tag, controller.signal, attempt > 0,
      () => { if (!controller.signal.aborted) setState(previous => previous?.owner === owner && previous.data ? previous : { owner, preparing: true }); },
      data => { if (!controller.signal.aborted) setState({ owner, data }); }).then(
      data => { if (!controller.signal.aborted) setState({ owner, data }); },
      error => { if (!controller.signal.aborted) setState({ owner, error: String(error) }); },
    );
    return () => controller.abort();
  }, [owner, active, attempt]);
  const current = state?.owner === owner ? state : undefined;
  return { data: current?.data, error: current?.error, preparing: current?.preparing ?? false,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
