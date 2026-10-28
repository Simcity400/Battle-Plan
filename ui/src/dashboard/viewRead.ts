import { useEffect, useRef, useState } from 'react';

const owners = new WeakMap<object, number>();
let ownersSeen = 0;
const ownerId = (owner: object) => { let id = owners.get(owner); if (id === undefined) owners.set(owner, id = ++ownersSeen); return id; };

export interface ViewRead<T> {
  /** The newest read that landed for this lane, whichever view it was asked for. */
  data?: T;
  /** The failure of the view now asked for. */
  error?: string;
  /** The view key `data` answers. */
  viewKey?: string;
  /** The view key whose read failed with `error`. */
  errorKey?: string;
  /** The view key of the read in flight, as of the last commit. */
  asking?: string;
  retry: () => void;
}

/** A read that follows a chart's view (its hours and width): one at a time per lane.
 * While the view keeps moving (playback, a pan or zoom) the running read is not
 * cancelled: it lands and is shown, and then the newest view is read, so a slow
 * connection still draws the chart instead of cancelling every read. The lane is
 * what is read (source, series, selection); changing it cancels and clears. Charts
 * place a landed summary by hour (`summaryFits`), never past the hours shown. */
export function useViewRead<V, T>(reads: string | null, view: V | null | undefined, viewKey: string, active: boolean,
  read: (view: V, signal: AbortSignal, publish: (value: T) => void, restart: boolean) => Promise<T>, owner?: object): ViewRead<T> {
  // The reading client is part of the lane: another client's reply is never shown.
  const lane = reads == null ? null : owner ? `${ownerId(owner)}:${reads}` : reads;
  const [state, setState] = useState<{ lane: string; viewKey: string; data?: T; error?: string; errorKey?: string }>();
  const [attempt, setAttempt] = useState(0);
  const [landed, setLanded] = useState(0);
  const flight = useRef<{ lane: string; id: string; viewKey: string; shown: boolean; controller: AbortController } | null>(null);
  const wanted = useRef({ view, viewKey }); wanted.current = { view, viewKey };
  const reader = useRef(read); reader.current = read;
  useEffect(() => {
    if (!active || lane == null || view == null) return;
    const id = JSON.stringify([lane, attempt]);
    // A read still preparing publishes partial replies and may not end for a while:
    // once it has shown something, a moved view replaces it.
    if (flight.current?.id === id && (!flight.current.shown || flight.current.viewKey === viewKey)) return;
    flight.current?.controller.abort();
    const controller = new AbortController();
    const asked = wanted.current;
    const mine = { lane, id, viewKey: asked.viewKey, shown: false, controller };
    flight.current = mine;
    const publish = (data: T) => {
      if (controller.signal.aborted) return;
      setState({ lane, viewKey: asked.viewKey, data });
      if (!mine.shown) { mine.shown = true; if (wanted.current.viewKey !== asked.viewKey) setLanded(value => value + 1); }
    };
    // After Retry, reads may restart stopped preparation, as a retried product does.
    void reader.current(asked.view as V, controller.signal, publish, attempt > 0).then(publish, error => {
      // Kept data keeps the view it answers; only the failure is the asked view's.
      if (!controller.signal.aborted) setState(previous => ({ lane, ...(previous?.lane === lane && previous.data !== undefined
        ? { viewKey: previous.viewKey, data: previous.data } : { viewKey: asked.viewKey }), error: error instanceof Error ? error.message : String(error),
        errorKey: asked.viewKey }));
    }).finally(() => {
      if (flight.current?.controller !== controller) return;
      flight.current = null;
      if (!controller.signal.aborted && wanted.current.viewKey !== asked.viewKey) setLanded(value => value + 1);
    });
    // The view's key is its identity; the view object is recreated each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lane, viewKey, active, attempt, landed]);
  // Leaving a lane, or the screen, cancels that lane's read only.
  useEffect(() => () => { if (flight.current?.lane === lane) { flight.current.controller.abort(); flight.current = null; } }, [lane, active]);
  const current = state?.lane === lane ? state : undefined;
  // A failure belongs to the view it was asked for: a moved view is still loading.
  return { data: current?.data, error: current?.errorKey === viewKey ? current.error : undefined, viewKey: current?.viewKey, errorKey: current?.errorKey,
    asking: flight.current?.lane === lane ? flight.current.viewKey : undefined,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
