import type { PlotFrame } from './frame';
import type { Range } from './view';

/** Geometry actually drawn at a pick; keyboard picks have no pointer target. */
export interface ChartPick { frame: PlotFrame; until: number | null; point?: { x: number; y: number } }
type PickCallback = (hour: number, context?: ChartPick) => void;
interface PickScope { owner?: unknown; extent: Range; branchStart: number; active: boolean; onPick?: PickCallback }
interface DrawnPick { epoch: number; context: ChartPick; onPick?: PickCallback }

/** A delayed click retains its drawn callback and geometry. Playback and labels
 * may change while it waits; a different source/selection/model invalidates it. */
export class ChartPicks {
  private scope: PickScope | null = null;
  private epoch = 0;

  update(scope: PickScope): void {
    const before = this.scope;
    if (!before || !Object.is(before.owner, scope.owner) || before.branchStart !== scope.branchStart
      || before.extent[0] !== scope.extent[0] || before.extent[1] !== scope.extent[1]
      || before.active !== scope.active || !!before.onPick !== !!scope.onPick) this.epoch++;
    this.scope = { ...scope, extent: [scope.extent[0], scope.extent[1]] };
  }

  drawn(context: ChartPick, onPick?: PickCallback): DrawnPick {
    return { epoch: this.epoch, context, onPick };
  }

  pick(drawn: DrawnPick, hour: number): boolean {
    if (drawn.epoch !== this.epoch || !this.scope?.active || !this.scope.onPick || !drawn.onPick) return false;
    drawn.onPick(Math.floor(hour), drawn.context);
    return true;
  }
}
