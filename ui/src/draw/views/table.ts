import type { LaneView } from '../lane.ts';
import { familyTableLaneView } from '../../families/table/laneView.ts';

export function tableView(_structureOf: unknown, wake: (widget: number) => void): LaneView {
  return familyTableLaneView(wake);
}
