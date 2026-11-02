import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { useParityRows } from '../../dashboard/parityRows';
import type { ParityList } from '../../dashboard/parityRows';
import { isMember } from '../../dashboard/groups';
import type { HourSpan } from '../../generated/dashboard';
import type { PartyHistoryReply } from '../../generated/dashboard';
import { contentTitle } from '../../dashboard/contentTitles';
import { partyBadges, partyNameAt, rawAt, rulingAt, leaderAt } from './partyData';
import { validatePreparedParty } from './preparedParty';

export function partyParityRows(reply: PartyHistoryReply, extent: readonly [number, number], markers: boolean, member: (hour: number) => boolean = () => true): unknown[] {
  if (markers) return partyBadges(reply, token => contentTitle(reply.ideologies.find(ideology => ideology.token === token)?.name, token, 'Ideology name unavailable'))
    .filter(badge => badge.glyph !== '✓' && badge.hour >= extent[0] && badge.hour <= extent[1] && member(badge.hour)).map(badge => ({...badge, glyph:'⚙'}));
  const rows: unknown[] = [];
  for (let hour = Math.ceil(extent[0]); hour <= extent[1]; hour++) {
    const leader = leaderAt(reply, hour);
    rows.push({ hour, values: reply.parties.map(party => ({ token: party.token, valueRaw: rawAt(party, hour) })), ruling: rulingAt(reply, hour),
      name: partyNameAt(reply, hour) ?? null, leader: leader ? { ideology: leader.ideology, person: leader.person == null ? null : reply.people[leader.person] } : null,
      election: reply.elections.includes(hour), unavailable: reply.unobserved.some(span => span.firstHour <= hour && span.lastHour >= hour) });
  }
  return rows;
}
/** Campaign-wide data is gathered only for the explicit parity handshake. */
export function usePartyParity(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined, tag: string | undefined,
  extent: readonly [number, number], label: 'Party history values' | 'Stability party markers', legacy?: PartyHistoryReply, within?: readonly HourSpan[]) {
  const root = useRef<HTMLDivElement>(null), { requested, prepare } = useParityPreparation();
  const key = JSON.stringify([source?.id, source?.revision, branchId, tag, extent, label, within]);
  const [answer, setAnswer] = useState<{ key: string; client: DashboardClient; model?: ParityList; error?: Failure }>();
  useEffect(() => {
    if (!requested) return;
    if (!source || !branchId || !tag) {setAnswer({key,client,model:{kind:'chart',label,complete:true,total:0,rows:[]}});return;}
    const controller = new AbortController();
    void (async () => {
      const reply = client.preparedQuery ? validatePreparedParty(await preparedSeriesQuery(client, source, 'party-history', { branchId, tag, mode: 'history' }, controller.signal, true)).reply : legacy;
      if (!reply) return;
      const rows = partyParityRows(reply, extent, label === 'Stability party markers', hour=>isMember(within,hour));
      if (!controller.signal.aborted) setAnswer({ key, client, model: { kind: 'chart', label, complete: true, total: rows.length, rows } });
    })().catch(error => { if (!controller.signal.aborted) setAnswer({ key, client, error: asFailure(error) }); });
    return () => controller.abort();
  }, [requested, key, client, legacy]);
  const current = answer?.key === key && answer.client === client ? answer : undefined;
  useParityRows(root, () => current?.model ?? { kind: 'chart', label, complete: false, total: 0, rows: [] },
    { prepare, ready: () => !!current?.model, error: () => current?.error ?? null });
  return root;
}
