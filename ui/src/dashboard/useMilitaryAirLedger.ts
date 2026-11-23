import { useMemo, useRef } from 'react';
import type { OptionChoices, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { resolveWindow } from './model';
import { useGate } from './regionGate.ts';
import { SPEC } from './regionTime.ts';
import { windowOf, windowsOf } from './heldWindows.ts';
import { useFrameWindows } from '../widgets/army-size/frameWindows.ts';
import { airCauseName, validateMilitaryAirWindow } from './militaryAirLedger';
import { LedgerJoin, ledgerWindows } from './militaryAirJoin.ts';
import type { LedgerReach } from './militaryAirJoin.ts';
import type { MilitaryAirFamily, MilitaryAirMetric, MilitaryAirReply } from '../generated/dashboard';
import { productRequest } from './request.ts';

export function useMilitaryAirLedger(widget: WidgetSpec, runtime: WidgetRuntime, family: MilitaryAirFamily, active: boolean,
  query?: { regions?: readonly number[]; causes?: readonly number[]; metric: MilitaryAirMetric }, reach: LedgerReach = 'end') {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
  const ranged = widget.time.mode === 'range';
  const range = ranged ? gate.window : null;
  const top = query && widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? { count: widget.countries.group.count, metric: query.metric } : null;
  const start = range?.firstHour ?? track?.branch.firstHour ?? 0, end = range?.lastHour ?? track?.branch.lastHour ?? 0;
  const params = track && (!ranged || range) ? { branchId: track.branch.id, family, regions: query?.regions ?? [], causes: query?.causes ?? [], fromHour: range?.firstHour ?? null, top } : null;
  const identity = track && params ? JSON.stringify([track.source.id, track.source.revision, params]) : null;
  const following = widget.time.mode === 'follow';
  const shownAt = (hour: number) => Math.min(end, Math.max(start, (track && resolveWindow(widget, track, runtime.tracks, hour)?.lastHour) ?? end));
  const wantedAt = (hour: number) => track ? ledgerWindows(start, end, shownAt(hour), reach, following) : [];
  const subscribe = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const windows = useFrameWindows<MilitaryAirReply>(runtime.client, identity, window => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'military-air-ledger', params: { ...params!, window } })), () => wantedAt(runtime.clock.getHour()), subscribe, active, validateMilitaryAirWindow, track?.source);
  const shown = windowOf(shownAt(runtime.clock.getHour()));
  const first = reach === 'end' ? windowOf(start) : shown;
  const joiner = useRef<LedgerJoin | null>(null);
  let join = joiner.current;
  if (!track || identity === null) join = null;
  else {
    if (join && (join.key !== identity || (reach === 'end' ? join.first !== first : shown < join.first || shown > join.last))) join = null;
    if (join && reach === 'shown' && shown > join.first && windows.data.has(join.next)) join = null;
    if (!join && windows.data.has(first)) join = new LedgerJoin(identity, first);
    while (join && windows.data.has(join.next) && (reach === 'end' || join.next <= shown + 1)) join.append(windows.data.get(join.next)!);
  }
  joiner.current = join;
  const data = join && join.last >= shown ? join.ledger() : null;
  const needed = track && reach === 'end' ? windowsOf(start, end) : [shown];
  const error = needed.map(window => windows.errors.get(window)).find(failure => failure !== undefined);
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
