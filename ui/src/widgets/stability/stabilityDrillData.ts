import type { DrillAction } from '../../charts/drill/drillState';
import type { Measure } from './stabilityData';

export interface StabilityPick {
  hour: number; candidates: readonly string[]; lines: readonly { tag: string; measure: Measure }[];
  tag: string | null; measure: Measure;
}

export function chooseStabilityActor(pick: StabilityPick, tag: string | null): StabilityPick {
  if (!tag || !pick.candidates.includes(tag)) return pick;
  const lines = pick.lines.filter(line => line.tag === tag);
  return { ...pick, tag, measure: lines.find(line => line.measure === pick.measure)?.measure ?? lines[0].measure };
}

export function stabilityPickAction(pick: StabilityPick, open: boolean, followsAt: (hour: number) => boolean): DrillAction {
  return open ? { type: 'pick', hour: pick.hour } : { type: 'open', breakdown: 'parts', hour: pick.hour, follow: pick.tag !== null && followsAt(pick.hour) };
}
