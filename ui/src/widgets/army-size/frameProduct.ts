import { useEffect, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from '../../dashboard/client';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { loadProduct } from '../../dashboard/products';
import type { ProductState } from '../../dashboard/products';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import type { HeadClock } from './headHour.ts';

export function useFrameProduct<T>(client: DashboardClient, clock: HeadClock, enabled: boolean,
  ask: (hour: number) => { key: string; request: ProductRequest } | null, validate: (data: unknown) => T,
  source: DashboardSource | undefined, same: (a: T, b: T) => boolean): ProductState<T> {
  const [shown, setShown] = useState<{ data?: T; error?: Failure } | null>(null);
  const latest = useRef({ ask, validate, source, same });
  latest.current = { ask, validate, source, same };
  const check = useRef<(() => void) | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!enabled) { setShown(null); return; }
    const controller = new AbortController();
    let asked: string | null = null, sent = 0, landed = 0;
    const run = () => {
      const next = latest.current.ask(clock.getHour());
      if (!next || next.key === asked) return;
      asked = next.key;
      const mine = ++sent;
      void loadProduct<T>(client, next.request, controller.signal, latest.current.validate, latest.current.source).then(data => {
        if (controller.signal.aborted || mine < landed) return;
        landed = mine;
        inFrame(() => setShown(before => before?.data !== undefined && !before.error && latest.current.same(before.data, data) ? before : { data }));
      }, error => {
        if (controller.signal.aborted || mine < landed) return;
        landed = mine;
        inFrame(() => setShown(before => ({ data: before?.data, error: asFailure(error) })));
      });
    };
    check.current = run;
    run();
    const stops = [clock.subscribeFrame(run), clock.subscribe(run)];
    return () => { controller.abort(); check.current = null; for (const stop of stops) stop(); };
  }, [enabled, client, clock, attempt]);
  useEffect(() => { check.current?.(); });
  return { data: enabled ? shown?.data : undefined, error: enabled ? shown?.error : undefined, retry: () => { setShown(null); setAttempt(value => value + 1); } };
}
