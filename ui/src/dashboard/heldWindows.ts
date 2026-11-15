/** The windowed shape of a family's reply over time (`crates/dashboard/src/products/windows.rs`): window `index` holds
 * the hours `index * WINDOW_HOURS` on, on one fixed grid. A widget asks the windows it shows and the next ahead,
 * never the campaign; the windows held join into what it draws. */

/** Hours one window holds: four blocks of the recording. */
export const WINDOW_HOURS = 4 * 720;

/** The window holding `hour`. */
export const windowOf = (hour: number) => Math.floor(hour / WINDOW_HOURS);

/** The windows holding `firstHour..=lastHour`, in order; with `ahead`, the one after them too. */
export function windowsOf(firstHour: number, lastHour: number, ahead = false): number[] {
  if (!Number.isFinite(firstHour) || !Number.isFinite(lastHour) || lastHour < firstHour) return [];
  const first = windowOf(firstHour), last = windowOf(lastHour) + (ahead ? 1 : 0);
  return Array.from({ length: last - first + 1 }, (_, index) => first + index);
}
