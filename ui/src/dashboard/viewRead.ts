import { type Failure, asFailure } from './queryError.ts';
import { useEffect, useRef, useState } from 'react';
import { inFrame } from './frameUpdates.ts';
import { replyDigest } from './replyDigest.ts';
import { countWork, measuredWidget } from './widgetPerf.ts';

const owners = new WeakMap<object, number>();
let ownersSeen = 0;
const ownerId = (owner: object) => { let id = owners.get(owner); if (id === undefined) owners.set(owner, id = ++ownersSeen); return id; };

export interface ViewRead<T> {
  data?: T;
  error?: Failure;
  viewKey?: string;
  errorKey?: string;
  asking?: string;
  retry: () => void;
}

export type Holds<V, T> = (data: T, asked: V, view: V) => boolean;

export function useViewRead<V, T>(reads: string | null, view: V | null | undefined, viewKey: string, active: boolean,
  read: (view: V, signal: AbortSignal, publish: (value: T) => void, restart: boolean) => Promise<T>, owner?: object,
  holds?: Holds<V, T>, peek?: (view: V) => T | undefined): ViewRead<T> {
  const lane = reads == null ? null : owner ? `${ownerId(owner)}:${reads}` : reads;
  const widget = measuredWidget();
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
    if (flight.current?.id === id && (!flight.current.shown || flight.current.viewKey === viewKey)) return;
    const held = shown.current;
    if (held?.lane === lane && held.final && held.data !== undefined && held.asked !== undefined && !held.error
      && holder.current?.(held.data, held.asked, view as V)) {
      flight.current?.controller.abort();
      flight.current = null;
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
          if (final) held.final = true;
          countWork(widget, 'same');
        } else {
          const next = { lane, viewKey: asked.viewKey, data, asked: asked.view as V, final };
          shown.current = next;
          setState(next);
          countWork(widget, 'reply');
        }
        if (!mine.shown) { mine.shown = true; if (wanted.current.viewKey !== asked.viewKey) setLanded(value => value + 1); }
      });
    };
    countWork(widget, 'read');
    void reader.current(asked.view as V, controller.signal, data => publish(data), attempt > 0).then(data => publish(data, true), error => {
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
  }, [lane, viewKey, active, attempt, landed]);
  useEffect(() => () => { if (flight.current?.lane === lane) { flight.current.controller.abort(); flight.current = null; } }, [lane, active]);
  const kept = state?.lane === lane ? state : undefined;
  const peekAt = lane == null || view == null ? null : `${lane}\u0000${viewKey}`;
  const peeked = useRef<{ at: string; data: T | undefined } | null>(null);
  if (!kept && peekAt !== null && peeked.current?.at !== peekAt) peeked.current = { at: peekAt, data: peek?.(view as V) };
  const heldData = !kept && peekAt !== null && peeked.current?.at === peekAt ? peeked.current.data : undefined;
  const current = kept ?? (heldData === undefined ? undefined : { lane: lane!, viewKey, data: heldData, errorKey: undefined, error: undefined });
  return { data: current?.data, error: current?.errorKey === viewKey ? current.error : undefined, viewKey: current?.viewKey, errorKey: current?.errorKey,
    asking: flight.current?.lane === lane ? flight.current.viewKey : undefined,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
