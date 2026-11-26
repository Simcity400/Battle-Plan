import { entryAt } from '../../dashboard/reads.ts';
import type { WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { labelObservationAt } from '../wars/warsData.ts';

export function chosenWar(reply: WarsReply, saved: string | null, at: number): WarTimeline | null {
  if (saved) {
    const war = reply.wars.find(item => item.id === saved);
    if (war) return war.firstHour <= at ? war : null;
  }
  let best: { war: WarTimeline; active: boolean; count: number; last: number } | null = null;
  for (const war of reply.wars) {
    const observation = entryAt(war.observations, at);
    if (!observation) continue;
    const active = at <= observation.lastHour;
    const last = Math.min(at, observation.lastHour);
    const count = new Set([...observation.viewpointSide, ...observation.opposingSide]).size;
    const better = !best || (active !== best.active ? active : count !== best.count ? count > best.count
      : last !== best.last ? last > best.last : Number(war.id) < Number(best.war.id));
    if (better) best = { war, active, count, last };
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

export function warChoicesAt(wars: WarsReply | undefined, at: number) {
  return (wars?.wars ?? []).flatMap(war => {
    const observation = entryAt(war.observations, at);
    return observation ? [{ id: war.id, firstHour: war.firstHour, name: warName(observation) }] : [];
  });
}
