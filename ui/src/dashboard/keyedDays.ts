import { keyedKnown, keyedRead } from './keyedHistory.ts';
import type { KeyedWindow } from './keyedHistory';

/** One calendar day of a daily keyed window, clipped to the extent: the hour its values
 * are looked up at (the day's last hour shown) and the read that supplied them, or null
 * when the day was not recorded. */
export interface KeyedDay { firstHour: number; lastHour: number; at: number; readHour: number | null }

/** The days of `extent` as heat periods. A day's value is the one its last hour holds, so a
 * daily window (whose runs cover each day whole) is read once per day. */
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
