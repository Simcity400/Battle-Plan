import { curveSegments, heldSegments, runAt } from './series.ts';
import type { PathCommand, Run } from './series';
import type { ChartPick } from './TimeChart';

/** Screen ordinates of the paths drawLine uses, including held vertical edges.
 * monotonePath's cubic x controls are thirds, so x advances linearly in t. */
function ordinates(path: readonly PathCommand[], x: number, targetY: number): number[] {
  const ys: number[] = [];
  let px = 0, py = 0;
  for (const command of path) {
    const cx = command[command.length - 2] as number, cy = command[command.length - 1] as number;
    if (command[0] === 'M') {
      if (Math.abs(x - cx) < 1e-6) ys.push(cy);
    } else if (x >= Math.min(px, cx) && x <= Math.max(px, cx)) {
      const t = cx === px ? 0 : (x - px) / (cx - px);
      if (command[0] === 'L') {
        ys.push(py + (cy - py) * t);
        if (cx === px) ys.push(cy, Math.max(Math.min(py, cy), Math.min(Math.max(py, cy), targetY)));
      } else {
        const u = 1 - t;
        ys.push(u ** 3 * py + 3 * u ** 2 * t * command[2] + 3 * u * t ** 2 * command[4] + t ** 3 * cy);
      }
    }
    px = cx; py = cy;
  }
  return ys;
}

/** Nearest drawn actor at the shown hour; coincident lines require a chooser.
 * Keyboard picks offer all actors drawn there, without inventing a Y target. */
export function pickLines(lines: readonly { key: string; runs: readonly Run[]; held?: boolean }[], hour: number,
  context?: ChartPick): string[] {
  if (!context?.frame) return [];
  const distances = new Map<string, number>();
  const frame = context.frame;
  if (hour < Math.floor(frame.x[0]) || hour > Math.min(frame.x[1], context.until ?? Infinity)) return [];
  for (const line of lines) {
    const run = runAt(line.runs, hour);
    if (!run || !Number.isFinite(run.value)) continue;
    let distance = 0;
    const x = context.point?.x ?? frame.toX(Math.max(frame.x[0], hour));
    const paths = (line.held ? heldSegments : curveSegments)(line.runs, frame.x[0],
      Math.min(frame.x[1], context.until ?? Infinity), frame.toX, frame.toY);
    const ys = paths.flatMap(segment => ordinates(segment.path, x, context.point?.y ?? frame.plotTop + frame.plotHeight / 2))
      .filter(y => Number.isFinite(y) && y >= frame.plotTop && y <= frame.plotTop + frame.plotHeight);
    if (!ys.length) continue;
    if (context.point) distance = ys.reduce((best, y) => Math.min(best, Math.abs(y - context.point!.y)), Infinity);
    distances.set(line.key, Math.min(distances.get(line.key) ?? Infinity, distance));
  }
  if (!context.point) return [...distances.keys()];
  const nearest = [...distances.values()].reduce((best, distance) => Math.min(best, distance), Infinity);
  // Within one stroke's screen width, choosing an actor would be ambiguous.
  return [...distances].filter(([, distance]) => distance <= nearest + 2).map(([key]) => key);
}
