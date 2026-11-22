import { type Failure, asFailure } from './queryError.ts';
import { useEffect, useState } from 'react';
import { useContentRevision } from './contentRevision.ts';
import type { CountryGroupReply } from '../generated/dashboard';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadCountryGroups } from './products.ts';
import { inFrame } from './frameUpdates.ts';

export function useCountryGroups(runtime: Pick<WidgetRuntime, 'client'>, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, active = true) {
  const { client } = runtime;
  const content = useContentRevision(client);
  const key = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const [state, setState] = useState<{ client: typeof client; key: string; data?: CountryGroupReply; complete?: boolean; error?: Failure }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track) return;
    const controller = new AbortController();
    const publish = (complete: boolean) => (data: CountryGroupReply) => inFrame(() => { if (!controller.signal.aborted) setState({ client, key, data, complete }); });
    void loadCountryGroups(client, track.source, track.branch.id, controller.signal, attempt > 0, publish(false)).then(
      publish(true),
      error => inFrame(() => { if (!controller.signal.aborted) setState(previous => ({ client, key,
        ...(previous?.client === client && previous.key === key ? { data: previous.data, complete: previous.complete } : {}),
        error: asFailure(error) })); }),
    );
    return () => controller.abort();
  }, [active, client, key, attempt]);
  const current = key && state?.client === client && state.key === key ? state : undefined;
  return { data: current?.data, complete: current?.complete === true, error: current?.error, retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
