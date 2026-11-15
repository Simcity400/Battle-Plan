import { type Failure, asFailure } from './queryError.ts';
import { useEffect, useRef, useState } from 'react';
import { inFrame } from './frameUpdates.ts';
import { replyDigest } from './replyDigest.ts';
import { countWork, measuredWidget } from './widgetPerf.ts';

const owners = new WeakMap<object, number>();
let ownersSeen = 0;
const ownerId = (owner: object) => { let id = owners.get(owner); if (id === undefined) owners.set(owner, id = ++ownersSeen); return id; };

export interface ViewRead<T> {
  /** The newest read that landed for this lane, whichever view it was asked for. */
  data?: T;
  /** The failure of the view now asked for. */
  error?: Failure;
  /** The view key `data` answers. */
  viewKey?: string;
  /** The view key whose read failed with `error`. */
  errorKey?: string;
  /** The view key of the read in flight, as of the last commit. */
  asking?: string;
  retry: () => void;
}

/** Whether the reply `data`, read for `asked`, answers `view` too: the data's own change hours
 * say it stays the same there, so no read is needed. */
export type Holds<V, T> = (data: T, asked: V, view: V) => boolean;

/** A read that follows a chart's view (its hours and width): one at a time per lane.
 * While the view keeps moving (playback, a pan or zoom) the running read is not
 * cancelled: it lands and is shown, and then the newest view is read, so a slow
 * connection still draws the chart instead of cancelling every read. A view the
 * shown reply already answers (`holds`) is not read at all. The lane is
 * what is read (source, series, selection); changing it cancels and clears. Charts
 * place a landed summary by hour (`summaryFits`), never past the hours shown. */
export function useViewRead<V, T>(reads: string | null, view: V | null | undefined, viewKey: string, active: boolean,
  read: (view: V, signal: AbortSignal, publish: (value: T) => void, restart: boolean) => Promise<T>, owner?: object,
  holds?: Holds<V, T>, peek?: (view: V) => T | undefined): ViewRead<T> {
  // The reading client is part of the lane: another client's reply is never shown.
  const lane = reads == null ? null : owner ? `${ownerId(owner)}:${reads}` : reads;
  const widget = measuredWidget();
  // `asked` is the view `data` was read for; `viewKey` the newest view it answers.
  const [state, setState] = useState<{ lane: string; viewKey: string; data?: T; asked?: V; final?: boolean; error?: Failure; errorKey?: string }>();
  const shown = useRef(state); shown.current = state;
  const holder = useRef(holds); holder.current = holds;
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
    const held = shown.current;
    if (held?.lane === lane && held.final && held.data !== undefined && held.asked !== undefined && !held.error
      && holder.current?.(held.data, held.asked, view as V)) {
      // A read for a view this reply now answers is no longer wanted.
      flight.current?.controller.abort();
      flight.current = null;
      // Renders in the next frame, unless a reply queued for it is newer.
      if (held.viewKey !== viewKey) inFrame(() => {
        if (shown.current !== held) return;
        const next = { ...held, viewKey };
        shown.current = next;
        setState(next);
      });
      return;
    }
    flight.current?.controller.abort();
    const controller = new AbortController();
    const asked = wanted.current;
    const mine = { lane, id, viewKey: asked.viewKey, shown: false, controller };
    flight.current = mine;
    const publish = (data: T, final = false) => {
      if (controller.signal.aborted) return;
      inFrame(() => {
        if (controller.signal.aborted) return;
        const held = shown.current, digest = replyDigest(data);
        if (digest !== undefined && held?.lane === lane && held.viewKey === asked.viewKey && !held.error
          && replyDigest(held.data) === digest) {
          // The same answer for the view shown: nothing to render. A settled one may now answer later views.
          if (final) held.final = true;
          countWork(widget, 'same');
        } else {
          // Updates of one frame render together: a later one compares with this, not with what was shown.
          const next = { lane, viewKey: asked.viewKey, data, asked: asked.view as V, final };
          shown.current = next;
          setState(next);
          countWork(widget, 'reply');
        }
        // Shown once it lands: until then a moved view waits for it rather than replacing it.
        if (!mine.shown) { mine.shown = true; if (wanted.current.viewKey !== asked.viewKey) setLanded(value => value + 1); }
      });
    };
    countWork(widget, 'read');
    // After Retry, reads may restart stopped preparation, as a retried product does.
    // Only the settled reply may answer later views: a partial one is still growing.
    void reader.current(asked.view as V, controller.signal, data => publish(data), attempt > 0).then(data => publish(data, true), error => {
      // Kept data keeps the view it answers; only the failure is the asked view's.
      if (!controller.signal.aborted) inFrame(() => {
        if (!controller.signal.aborted) setState(previous => ({ lane, ...(previous?.lane === lane && previous.data !== undefined
          ? { viewKey: previous.viewKey, data: previous.data } : { viewKey: asked.viewKey }), error: asFailure(error),
          errorKey: asked.viewKey }));
      });
    }).finally(() => {
      if (flight.current?.controller !== controller) return;
      flight.current = null;
      if (!controller.signal.aborted && wanted.current.viewKey !== asked.viewKey) inFrame(() => setLanded(value => value + 1));
    });
    // The view's key is its identity; the view object is recreated each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lane, viewKey, active, attempt, landed]);
  // Leaving a lane, or the screen, cancels that lane's read only.
  useEffect(() => () => { if (flight.current?.lane === lane) { flight.current.controller.abort(); flight.current = null; } }, [lane, active]);
  const kept = state?.lane === lane ? state : undefined;
  // Nothing shown yet in the lane: a reply already held for the view shows from the first render, never a loading state.
  // Peeked once a view: a render React bails out of keeps none of its effects, so it must show what the last one did.
  const peekAt = lane == null || view == null ? null : `${lane}\u0000${viewKey}`;
  const peeked = useRef<{ at: string; data: T | undefined } | null>(null);
  if (!kept && peekAt !== null && peeked.current?.at !== peekAt) peeked.current = { at: peekAt, data: peek?.(view as V) };
  const heldData = !kept && peekAt !== null && peeked.current?.at === peekAt ? peeked.current.data : undefined;
  const current = kept ?? (heldData === undefined ? undefined : { lane: lane!, viewKey, data: heldData, errorKey: undefined, error: undefined });
  // A failure belongs to the view it was asked for: a moved view is still loading.
  return { data: current?.data, error: current?.errorKey === viewKey ? current.error : undefined, viewKey: current?.viewKey, errorKey: current?.errorKey,
    asking: flight.current?.lane === lane ? flight.current.viewKey : undefined,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
