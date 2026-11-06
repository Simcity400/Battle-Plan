import { useEffect, useMemo, useRef, useState } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { asFailure } from '../../dashboard/queryError';
import type { Failure } from '../../dashboard/queryError';
import type { LeaderKinds, LeaderSelection, LeaderView, UnitLeadersHourReply } from '../../generated/dashboard';
import { joinLeaderHour, leaderRequestSelection, validateLeaderHourReply, validateUnitLeadersReply } from './generalsData';
import type { HeldLeaders, UnitLeadersData } from './generalsData';

export function useLeaderCountries(widget: WidgetSpec, runtime: WidgetRuntime, hour: number, active: boolean) {
  const selected = useCountriesSelectionAt(widget, runtime, hour, active);
  const requestSelection = useMemo(() => leaderRequestSelection(selected.requestTags), [selected.requestTags]);
  return { ...selected, requestSelection };
}
/** Leaders at `hour` follow playback as a leaderboard's rows (`useLeaderboardHour`); without an hour, the view whole. */
export function useLeaders(runtime: WidgetRuntime, trackId: string, selection: LeaderSelection | undefined, view: LeaderView, active: boolean, leaders: LeaderKinds = 'all', hour?: number) {
  const track = runtime.tracks.find(t => t.id === trackId);
  const request = track && selection && hour === undefined ? { sourceId: track.source.id, revision: track.source.revision, product: 'unit-leaders',
    params: { branchId: track.branch.id, selection, view, leaders } } : null;
  const fixed = useProduct<UnitLeadersData>(runtime.client, request, active, validateUnitLeadersReply, track?.source);
  const rows = useLeaderboardHour(runtime, track, view === 'leaderboard' ? selection : undefined, leaders, hour, active);
  if (hour === undefined) return { ...fixed, answered: undefined };
  return { data: rows.data, error: rows.error, retry: rows.retry, answered: rows.answered ?? hour };
}

/** A leaderboard at `hour` while playback runs: every row at the first hour read, then only the rows that changed
 * since the hour held, joined to those held. The rows held stay shown while the newest hour is read; `answered` is
 * the hour they are for. */
export function useLeaderboardHour(runtime: WidgetRuntime, track: BoundTrack | undefined, selection: LeaderSelection | undefined,
  leaders: LeaderKinds, hour: number | undefined, active: boolean) {
  const content = useContentRevision(runtime.client);
  const lane = track && selection && hour !== undefined
    ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, selection, leaders, content]) : null;
  const [held, setHeld] = useState<(HeldLeaders & { lane: string }) | null>(null);
  const [failure, setFailure] = useState<{ lane: string; error: Failure } | null>(null);
  const holding = held && held.lane === lane ? held : null;
  const latest = useRef(holding);
  latest.current = holding;
  // Rows numbered by an index are read against the hour held; rows numbered on their own are read whole.
  const against = holding && holding.reply.index !== null ? holding.reply : null;
  const request = track && selection && lane && hour !== undefined ? { sourceId: track.source.id, revision: track.source.revision, product: 'unit-leaders',
    params: { branchId: track.branch.id, selection, view: 'leaderboard', leaders, hour, rows: true,
      ...(against ? { since: against.hour, held: against.index } : {}) } } : null;
  const read = useFollowedProduct<UnitLeadersHourReply>(runtime.client, request, ['hour', 'since', 'held'], active, validateLeaderHourReply, track?.source);
  useEffect(() => {
    const reply = read.data;
    if (!reply || !lane) return;
    const kept = latest.current?.lane === lane ? latest.current : null;
    if (kept?.reply === reply) return;
    let joined: HeldLeaders | null;
    try { joined = joinLeaderHour(kept, reply); } catch (error) { setFailure({ lane, error: asFailure(error) }); return; }
    // Rows read against another hour than the one held are not joined to it.
    if (joined) { latest.current = { ...joined, lane }; setHeld(latest.current); }
  }, [read.data, lane]);
  const retry = () => { setHeld(null); setFailure(null); read.retry(); };
  return { data: holding?.data, answered: holding?.reply.hour, error: failure?.lane === lane ? failure.error : read.error, retry };
}
