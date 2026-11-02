import { asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct } from '../../dashboard/products';
import type { FactionGoalsReply } from '../../generated/dashboard';
import { currentParts, rangeKey, splitPart } from './goalParts';
import type { GoalPart, HeldParts } from './goalParts';

export { partAt } from './goalParts';

/** A faction's whole-branch goal history, asked in ranges that each fit the reply budget. */
export function useGoalHistory(client: DashboardClient, track: BoundTrack | undefined, faction: string | null, active: boolean) {
  const content = useContentRevision(client);
  const identity = track && faction ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, faction, content]) : '';
  const [attempt, setAttempt] = useState(0);
  const [held, setHeld] = useState<HeldParts>();
  const parts = currentParts(held, client, identity, attempt);
  const asked = parts.filter(part => part.error === undefined).map(rangeKey).join(',');
  // Each range keeps its own request until its range is replaced or the history is.
  const requests = useRef(new Map<string, AbortController>());
  useEffect(() => {
    const running = requests.current;
    return () => { for (const controller of running.values()) controller.abort(); running.clear(); };
  }, [active, client, identity, attempt]);
  useEffect(() => {
    if (!active || !track || !faction) return;
    const running = requests.current;
    const wanted = new Map(parts.filter(part => part.error === undefined).map(part => [rangeKey(part), part]));
    for (const [key, controller] of running) if (!wanted.has(key)) { controller.abort(); running.delete(key); }
    for (const [key, part] of wanted) {
      if (running.has(key)) continue;
      const controller = new AbortController();
      running.set(key, controller);
      const change = (next: (part: GoalPart) => readonly GoalPart[]) => setHeld(previous => ({ client, identity, attempt,
        parts: currentParts(previous, client, identity, attempt).flatMap(item => rangeKey(item) === key ? next(item) : [item]) }));
      const request = { sourceId: track.source.id, revision: track.source.revision, product: 'faction-state',
        params: { view: 'goals', branchId: track.branch.id, faction, firstHour: part.firstHour, lastHour: part.lastHour } };
      const publish = (data: FactionGoalsReply) => { if (!controller.signal.aborted) change(item => [{ ...item, data }]); };
      void loadProduct<FactionGoalsReply>(client, request, controller.signal, undefined, track.source, attempt > 0, publish).then(publish, error => {
        if (controller.signal.aborted) return;
        running.delete(key);
        const message = asFailure(error);
        change(item => splitPart(item, message, track.branch) ?? [{ ...item, error: message }]);
      });
    }
    // The identity names source, branch, faction and content; `asked` names the ranges.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, client, identity, attempt, asked]);
  return { parts, error: parts.find(part => part.error !== undefined)?.error, retry: () => setAttempt(value => value + 1) };
}
