import { type Failure, asFailure } from './queryError.ts';
import { useEffect, useState } from 'react';
import { useContentRevision } from './contentRevision';
import type { CountryGroupReply } from '../generated/dashboard';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadCountryGroups } from './products';

/** The branch's smart-group history, loaded once and shared through the product cache.
 * Before preparation covers the branch it is the prepared prefix's, carried on by
 * later keyframes (marked `approximate`), replaced as preparation goes on; `complete`
 * says the reply is the finished one, not a published provisional one. */
export function useCountryGroups(runtime: Pick<WidgetRuntime, 'client'>, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, active = true) {
  const { client } = runtime;
  const content = useContentRevision(client);
  const key = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const [state, setState] = useState<{ client: typeof client; key: string; data?: CountryGroupReply; complete?: boolean; error?: Failure }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track) return;
    const controller = new AbortController();
    const publish = (complete: boolean) => (data: CountryGroupReply) => { if (!controller.signal.aborted) setState({ client, key, data, complete }); };
    void loadCountryGroups(client, track.source, track.branch.id, controller.signal, attempt > 0, publish(false)).then(
      publish(true),
      error => { if (!controller.signal.aborted) setState(previous => ({ client, key,
        ...(previous?.client === client && previous.key === key ? { data: previous.data, complete: previous.complete } : {}),
        error: asFailure(error) })); },
    );
    return () => controller.abort();
    // The structured key owns source, branch and content revision.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, client, key, attempt]);
  const current = key && state?.client === client && state.key === key ? state : undefined;
  return { data: current?.data, complete: current?.complete === true, error: current?.error, retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
