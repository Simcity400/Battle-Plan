import type { WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { labelObservationAt, observationAt } from '../wars/warsData.ts';

export function chosenWar(reply: WarsReply, saved: string | null, at: number): WarTimeline | null {
  if (saved) {
    const war = reply.wars.find(item => item.id === saved);
    if (war) return war;
  }
  let best: { war: WarTimeline; active: boolean; count: number } | null = null;
  for (const war of reply.wars) {
    if (war.firstHour > at) continue;
    const active = observationAt(war, at) != null;
    const observation = labelObservationAt(war, Math.min(at, war.lastHour));
    const count = observation ? new Set([...observation.viewpointSide, ...observation.opposingSide]).size : 0;
    const better = !best || (active !== best.active ? active : count !== best.count ? count > best.count
      : war.lastHour !== best.war.lastHour ? war.lastHour > best.war.lastHour : Number(war.id) < Number(best.war.id));
    if (better) best = { war, active, count };
  }
  return best?.war ?? null;
}

export const readHourFor = (war: WarTimeline, at: number) => Math.min(Math.floor(at), war.lastHour);

export function warChipAt(wars: WarsReply | undefined, saved: string | null, at: number | null) {
  const war = wars && at !== null ? chosenWar(wars, saved, at) : null;
  const observation = war && at !== null ? labelObservationAt(war, at) : null;
  return { at, observation, key: `${war?.id ?? ''}:${observation?.firstHour ?? ''}` };
}

export const warName = (observation: WarObservation | null) => observation?.name.known ? observation.name.text : 'Name not recorded';
