import { useEffect, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct } from '../../dashboard/products';
import { splitPart, validateDivisionEquipmentHistory, WHOLE } from './divisionEquipmentHistory.ts';
import type { DivisionEquipmentHistory, HistoryPart } from './divisionEquipmentHistory';

export { partAt } from './divisionEquipmentHistory.ts';

type Held = { client: DashboardClient; identity: string; attempt: number; parts: readonly HistoryPart[] };
const currentParts = (held: Held | undefined, client: DashboardClient, identity: string, attempt: number): readonly HistoryPart[] =>
  held && held.client === client && held.identity === identity && held.attempt === attempt ? held.parts : WHOLE;

/** One country's prepared division equipment history, asked in ranges that each fit the reply budget. */
export function useDivisionEquipmentHistory(client: DashboardClient, track: BoundTrack | undefined, tag: string | null,
  countFor: 'owner' | 'host', active: boolean) {
  const content = useContentRevision(client);
  const prepared = !!client.preparedQuery;
  const identity = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, countFor, content]) : '';
  const [attempt, setAttempt] = useState(0);
  const [held, setHeld] = useState<Held>();
  const parts = currentParts(held, client, identity, attempt);
  const asked = parts.map(part => `${part.firstHour}:${part.lastHour}:${part.error === undefined ? '' : 'x'}`).join(',');
  useEffect(() => {
    if (!active || !prepared || !track || !tag) return;
    const controller = new AbortController();
    const change = (part: HistoryPart, next: (part: HistoryPart) => readonly HistoryPart[]) => setHeld(previous => ({ client, identity, attempt,
      parts: currentParts(previous, client, identity, attempt).flatMap(item => item.firstHour === part.firstHour && item.lastHour === part.lastHour ? next(item) : [item]) }));
    for (const part of parts) {
      if (part.error !== undefined) continue;
      const request = { sourceId: track.source.id, revision: track.source.revision, product: 'division-equipment-history',
        params: { branchId: track.branch.id, tag, countFor, firstHour: part.firstHour, lastHour: part.lastHour } };
      const publish = (data: DivisionEquipmentHistory | null) => { if (!controller.signal.aborted) change(part, item => [{ ...item, data }]); };
      void loadProduct<DivisionEquipmentHistory | null>(client, request, controller.signal, validateDivisionEquipmentHistory, track.source, attempt > 0, publish)
        .then(publish, error => {
          if (controller.signal.aborted) return;
          const message = error instanceof Error ? error.message : String(error);
          change(part, item => splitPart(item, message, track.branch) ?? [{ ...item, error: message }]);
        });
    }
    return () => controller.abort();
    // The identity names source, branch, country, attribution and content; `asked` names the ranges.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, client, identity, attempt, asked, prepared]);
  return { prepared, parts, retry: () => setAttempt(value => value + 1) };
}
