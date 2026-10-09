import { pickLines } from '../../charts/linePick.ts';
import type { ChartPick } from '../../charts/TimeChart';
import type { DrillAction } from '../../charts/drill/drillState';

export interface PowerActorPick { hour: number; candidates: readonly string[]; tag: string | null }

/** Only geometry witnessed by a drawn frame can supply actor candidates. */
export function pickPowerActor(lines: Parameters<typeof pickLines>[0], hour: number, context?: ChartPick): PowerActorPick | null {
  if (!context?.frame) return null;
  const candidates = pickLines(lines, hour, context);
  return candidates.length ? { hour: Math.floor(hour), candidates, tag: candidates.length === 1 ? candidates[0] : null } : null;
}

/** Choosing an actor retains the witnessed hour even while playback moves. */
export function choosePowerActor(pick: PowerActorPick, tag: string | null): PowerActorPick {
  return tag && pick.candidates.includes(tag) ? { ...pick, tag } : pick;
}

export function powerActorDrillAction(pick: PowerActorPick, open: boolean, playing: boolean, playhead: number | null): DrillAction {
  return open ? { type: 'pick', hour: pick.hour }
    : { type: 'open', breakdown: 'parts', hour: pick.hour, playing: pick.tag !== null && playing,
      playhead: pick.tag === null ? null : playhead };
}
