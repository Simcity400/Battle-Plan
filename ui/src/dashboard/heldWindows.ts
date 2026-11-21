
export const WINDOW_HOURS = 4 * 720;

export const windowOf = (hour: number) => Math.floor(hour / WINDOW_HOURS);

export function windowsOf(firstHour: number, lastHour: number, ahead = false): number[] {
  if (!Number.isFinite(firstHour) || !Number.isFinite(lastHour) || lastHour < firstHour) return [];
  const first = windowOf(firstHour), last = windowOf(lastHour) + (ahead ? 1 : 0);
  return Array.from({ length: last - first + 1 }, (_, index) => first + index);
}
