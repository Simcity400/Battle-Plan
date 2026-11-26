import { stackedSegments } from '../stack.ts';
import type { StackModel, StackSegment } from '../stack';

export function countryRaceSegments(stack: StackModel, from: number, to: number, hoursPerPixel: number): StackSegment[] {
  return stackedSegments(stack, from, to, hoursPerPixel / 2).map(heldStackSegment);
}

export function heldStackSegment(segment: StackSegment): StackSegment {
  const hours: number[] = [];
  const bands = segment.bands.map(() => ({ low: [] as number[], high: [] as number[] }));
  segment.hours.forEach((hour, index) => {
    if (index > 0) {
      hours.push(hour);
      bands.forEach((band, layer) => { band.low.push(segment.bands[layer].low[index - 1]); band.high.push(segment.bands[layer].high[index - 1]); });
    }
    hours.push(hour);
    bands.forEach((band, layer) => { band.low.push(segment.bands[layer].low[index]); band.high.push(segment.bands[layer].high[index]); });
  });
  return { ...segment, hours, bands };
}
