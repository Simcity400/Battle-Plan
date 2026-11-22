import type { GatedTime } from '../../dashboard/regionTime';
import type { PinnedSpec } from '../../charts/drill/drillState';
import { pickLines } from '../../charts/linePick.ts';
import type { ChartPick } from '../../charts/pick';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import type { Breakdown, DrillAction } from '../../charts/drill/drillState';
import type { Run } from '../../charts/series';
import type { WidgetSpec } from '../../dashboard/model';
import type { Measure } from './stabilityData';

export interface StabilityLine { tag: string; measure: Measure; runs: readonly Run[] }
export interface StabilityPick {
  hour: number; candidates: readonly string[]; lines: readonly { tag: string; measure: Measure }[];
  tag: string | null; measure: Measure;
}

export function pickStabilityLine(lines: readonly StabilityLine[], hour: number, context?: ChartPick,
  preferred: Measure = 'stability'): StabilityPick | null {
  if (!context?.frame) return null;
  const keyed = lines.map(line => ({ ...line, key: JSON.stringify([line.tag, line.measure]) }));
  const keys = new Set(pickLines(keyed, hour, context));
  const witnessed = keyed.filter(line => keys.has(line.key)).map(({ tag, measure }) => ({ tag, measure }));
  const candidates = [...new Set(witnessed.map(line => line.tag))];
  if (!candidates.length) return null;
  return { hour: Math.floor(hour), candidates, lines: witnessed, tag: candidates.length === 1 ? candidates[0] : null,
    measure: witnessed.find(line => line.measure === preferred)?.measure ?? witnessed[0].measure };
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
