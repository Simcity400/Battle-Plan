import { useMemo, useRef } from 'react';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { resolveWindow } from './model';
import { windowOf, windowsOf } from './heldWindows.ts';
import { useFrameWindows } from '../widgets/army-size/frameWindows.ts';
import { airCauseName, validateMilitaryAirWindow } from './militaryAirLedger';
import { LedgerJoin, ledgerWindows } from './militaryAirJoin.ts';
import type { LedgerReach } from './militaryAirJoin.ts';
import type { MilitaryAirFamily, MilitaryAirMetric, MilitaryAirReply } from '../generated/dashboard';

/** The air ledger a widget shows, joined from its windows of hours: reaching the hour shown, the window shown (which
 * carries every total before it and the service's top N within it) and, following, the next ahead; reaching the end,
 * every window of the span from its first hour (in a range its first hour, else the branch's). Asked from the frame as
 * the hour shown moves, so following asks each window ahead once, and only a window landing renders: it is appended to
 * the join, never the windows held joined again. */
export function useMilitaryAirLedger(widget: WidgetSpec, runtime: WidgetRuntime, family: MilitaryAirFamily, active: boolean,
  query?: { regions?: readonly number[]; causes?: readonly number[]; metric: MilitaryAirMetric }, reach: LedgerReach = 'end') {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  // Only a range's first hour is asked, and a range does not move with the clock.
  const range = track && widget.time.mode === 'range' ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const top = query && widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? { count: widget.countries.group.count, metric: query.metric } : null;
  const start = range?.firstHour ?? track?.branch.firstHour ?? 0, end = range?.lastHour ?? track?.branch.lastHour ?? 0;
  const params = track ? { branchId: track.branch.id, family, regions: query?.regions ?? [], causes: query?.causes ?? [], fromHour: range?.firstHour ?? null, top } : null;
  const identity = track && params ? JSON.stringify([track.source.id, track.source.revision, params]) : null;
  const following = widget.time.mode === 'follow';
  const shownAt = (hour: number) => Math.min(end, Math.max(start, (track && resolveWindow(widget, track, runtime.tracks, hour)?.lastHour) ?? end));
  const wantedAt = (hour: number) => track ? ledgerWindows(start, end, shownAt(hour), reach, following) : [];
  const subscribe = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const windows = useFrameWindows<MilitaryAirReply>(runtime.client, identity, window => ({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'military-air-ledger', params: { ...params!, window } }), () => wantedAt(runtime.clock.getHour()), subscribe, active, validateMilitaryAirWindow, track?.source);
  // The window this render shows; reaching the end, the join starts at the span's first window and is given once it holds
  // the windows through the one shown.
  const shown = windowOf(shownAt(runtime.clock.getHour()));
  const first = reach === 'end' ? windowOf(start) : shown;
  const joiner = useRef<LedgerJoin | null>(null);
  let join = joiner.current;
  if (!track || identity === null) join = null;
  else {
    if (join && (join.key !== identity || (reach === 'end' ? join.first !== first : shown < join.first || shown > join.last))) join = null;
    // Following, the next window landing starts the join again at the window shown, so it never holds more than two.
    if (join && reach === 'shown' && shown > join.first && windows.data.has(join.next)) join = null;
    if (!join && windows.data.has(first)) join = new LedgerJoin(identity, first);
    while (join && windows.data.has(join.next) && (reach === 'end' || join.next <= shown + 1)) join.append(windows.data.get(join.next)!);
  }
  joiner.current = join;
  const data = join && join.last >= shown ? join.ledger() : null;
  const needed = track && reach === 'end' ? windowsOf(start, end) : [shown];
  const error = needed.map(window => windows.errors.get(window)).find(failure => failure !== undefined);
  // The ledger through the hour shown, and whether the span's end is still to land.
  return { track, data: data ?? undefined, pending: reach === 'end' && (!join || join.last < windowOf(end)), error, retry: windows.retry };
}

export function useAirLossChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useMilitaryAirLedger(widget, runtime, 'airLosses', true);
  return {
    regions: (data?.regions ?? []).map(region => ({ value: region.id, label: region.name ?? `Region ${region.id} (name not recorded)` })),
    causes: [...new Set(data?.parts.flatMap(part => part.cause === null ? [] : [part.cause]) ?? [])].sort((a, b) => a - b)
      .map(cause => ({ value: String(cause), label: airCauseName(cause) })),
  };
}
