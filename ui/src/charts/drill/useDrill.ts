import { useCallback, useMemo, useState } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { recordedHour, regionTrack, type GatedHour, type Outside, type Region } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { useRegionGate, type Gate, type GateNeeds } from '../../dashboard/regionGate.ts';
import { drillAction, drillFollows, opensFollowing, reduceDrill, savedDrill, type DrillAction, type DrillState } from './drillState.ts';

export interface DrillControl<S extends DrillState = DrillState, A = DrillAction> {
  state: S;
  open: boolean;
  following: boolean;
  parent: Region;
  region: Region;
  gate: Gate;
  hour: GatedHour | null;
  outside: Outside | null;
  time: Gate['time'];
  hold: Gate['hold'];
  dispatch(action: A | DrillAction): void;
  followsAt(hour: number): boolean;
  show(breakdown: string, hour: number): void;
  showAt(breakdown: string, hour: number): void;
  attach(breakdown: string): void;
  pick(hour: number): void;
  follow(): void;
  pause(): void;
  close(): void;
}

export function useDrill<S extends DrillState = DrillState, A = DrillAction>(runtime: WidgetRuntime, parent: Region, saved: string | null = null,
  reduce: (state: S, action: A | DrillAction) => S = reduceDrill as never, closed?: S, needs?: GateNeeds): DrillControl<S, A> {
  const initial = useMemo(() => ({ ...closed, ...savedDrill(saved) }) as S, [saved, closed]);
  const [stored, setStored] = useState(() => ({ initial, state: initial }));
  const state = stored.initial === initial ? stored.state : initial;
  const dispatch = useCallback((action: A | DrillAction) => setStored(before => {
    const base = before.initial === initial ? before.state : initial;
    const closing = (action as { type?: string }).type === 'close';
    return { initial, state: closing && saved !== null ? initial : reduce(base, action) };
  }), [initial, reduce, saved]);
  const track = regionTrack(parent, runtime.tracks);
  const region = useRegion(parent.widget, drillAction(state));
  const open = state.breakdown !== null || !state.spec;
  const gate = useRegionGate(runtime, region, open, needs);
  const { clock } = runtime;
  return useMemo(() => {
    const playhead = () => track ? recordedHour(track, Math.floor(clock.getHour()) + track.axisOffset) : null;
    const followsAt = (hour: number) => { const head = playhead(); return head !== null && recordedHour(track, hour) !== null && opensFollowing(parent, hour, clock.getSnapshot().playing, head); };
    return {
      state, open, following: drillFollows(parent, state), parent, region, gate, hour: open ? gate.hour : null, outside: open ? gate.outside : null,
      time: gate.time, hold: gate.hold, dispatch, followsAt,
      show: (breakdown, hour) => dispatch({ type: 'open', breakdown, hour, follow: followsAt(hour) }),
      showAt: (breakdown, hour) => dispatch({ type: 'open', breakdown, hour, follow: false }),
      attach: breakdown => dispatch({ type: 'attach', breakdown }),
      pick: hour => dispatch({ type: 'pick', hour }),
      follow: () => dispatch({ type: 'follow' }),
      pause: () => { const head = Math.floor(clock.getHour()) + (track?.axisOffset ?? 0); if (track) dispatch({ type: 'pick', hour: head }); },
      close: () => dispatch({ type: 'close' }),
    };
  }, [state, open, parent, region, gate, dispatch, track, clock]);
}
