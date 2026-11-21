import { keyedKnown, keyedRead } from './keyedHistory.ts';
import type { KeyedWindow } from './keyedHistory';

export interface KeyedDay { firstHour: number; lastHour: number; at: number; readHour: number | null }

export function keyedDays(window: KeyedWindow, extent: readonly [number, number], dayHours: number): KeyedDay[] {
  const days: KeyedDay[] = [];
  if (!(dayHours > 0) || extent[1] < extent[0]) return days;
  for (let start = Math.floor(extent[0] / dayHours) * dayHours; start <= extent[1]; start += dayHours) {
    const firstHour = Math.max(start, extent[0]), lastHour = Math.min(start + dayHours - 1, extent[1]);
    const known = keyedKnown(window, lastHour);
    days.push({ firstHour, lastHour, at: lastHour, readHour: known ? keyedRead(window, lastHour) ?? lastHour : null });
  }
  return days;
}
