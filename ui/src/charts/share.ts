import { stackedSegments } from './stack.ts';
import type { StackModel } from './stack';
import { heldStackSegment } from './race/countryRace.ts';

export function shareSegments(model: StackModel, from: number, to: number, hoursPerPixel = 0) {
  return stackedSegments(model, from, to, hoursPerPixel / 2, true).map(segment => {
    const last = segment.hours.at(-1)!;
    const index = heldIndex(model.hours, last);
    const end = Math.min(to, model.hours[index + 1] ?? to);
    if (model.edges[index] && end > last) {
      segment.hours.push(end);
      for (const band of segment.bands) { band.low.push(band.low.at(-1)!); band.high.push(band.high.at(-1)!); }
    }
    return heldStackSegment(segment);
  });
}

function heldIndex(hours: readonly number[], hour: number): number {
  let low = 0; let high = hours.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (hours[middle] <= hour) low = middle + 1; else high = middle;
  }
  return Math.max(0, low - 1);
}
