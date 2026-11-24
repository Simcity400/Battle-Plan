import type { GatedTime } from '../../dashboard/regionTime';
import type { PinnedSpec } from '../../charts/drill/drillState';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import type { Breakdown, DrillAction } from '../../charts/drill/drillState';
import type { WidgetSpec } from '../../dashboard/model';
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

export function pinStabilityDrill(widget: WidgetSpec, breakdown: Breakdown, trackId: string, tag: string, time: GatedTime): PinnedSpec {
  return pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [tag] } }, breakdown, trackId, time);
}
