import { useEffect, useMemo, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import type { DashboardClient, DashboardSource } from './client';
import type { PeaceDealsReply } from '../generated/dashboard';
import { useContentRevision } from './contentRevision.ts';
import { preparedSeriesQuery } from './preparedSeries.ts';
import { validatePeaceDealsReply } from './peaceDealsData.ts';

/** The branch's peace deals. Before preparation covers the branch, `publish`
 * receives each reply answered from its prepared prefix as it grows. */
export async function loadPreparedPeaceDeals(client: DashboardClient, source: DashboardSource, branchId: string, signal: AbortSignal, restart = false,
  pending: () => void = () => {}, publish?: (reply: PeaceDealsReply) => void): Promise<PeaceDealsReply> {
  return validatePeaceDealsReply(await preparedSeriesQuery(client, source, 'events-page', { branchId, family: { kind: 'peace-deals' } }, signal, restart, pending,
    undefined, publish && (value => publish(validatePeaceDealsReply(value)))));
}

export function usePeaceDeals(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const content = useContentRevision(runtime.client);
  const key = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, content]);
  const owner = useMemo(() => ({ client: runtime.client, key }), [runtime.client, key]);
  const [state, setState] = useState<{ owner: typeof owner; data?: PeaceDealsReply; error?: string; preparing?: boolean }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track) return;
    const controller = new AbortController();
    void loadPreparedPeaceDeals(runtime.client, track.source, track.branch.id, controller.signal, attempt > 0,
      () => { if (!controller.signal.aborted) setState(previous => previous?.owner === owner && previous.data ? previous : { owner, preparing: true }); },
      data => { if (!controller.signal.aborted) setState({ owner, data }); }).then(
      data => { if (!controller.signal.aborted) setState({ owner, data }); },
      error => { if (!controller.signal.aborted) setState(previous => ({ owner, data: previous?.owner === owner ? previous.data : undefined, error: String(error) })); },
    );
    return () => controller.abort();
  }, [owner, active, attempt]);
  const current = state?.owner === owner ? state : undefined;
  return { track, data: current?.data, error: current?.error, preparing: current?.preparing ?? false,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
