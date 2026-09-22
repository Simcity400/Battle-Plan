// Keep this presentation conversion aligned with engine::cdate,
// installed_content::save::GameDate::engine_hour and viewer::app::date. Game dates have
// 365 days every year and display hours 01:00 through 24:00, not local time.
export const HOUR_EPOCH = 43_800_000;
const HOURS_PER_DAY = 24;
const DAYS_PER_YEAR = 365;
const MIN_HOUR = -2_147_483_648;
const MAX_HOUR = 2_147_483_647;

export const monthLengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] as const;
export const monthNames = [
  'January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December',
] as const;

export interface GameDate { year: number; month: number; day: number; hour: number }

export function isEngineHour(hour: number): boolean {
  return Number.isInteger(hour) && hour >= MIN_HOUR && hour <= MAX_HOUR;
}

export function gameDate(engineHour: number): GameDate | null {
  // The native viewer has no calendar interpretation before this epoch.
  if (!isEngineHour(engineHour) || engineHour < HOUR_EPOCH) return null;
  const elapsed = engineHour - HOUR_EPOCH;
  const days = Math.floor(elapsed / HOURS_PER_DAY);
  const year = Math.floor(days / DAYS_PER_YEAR);
  let day = days - year * DAYS_PER_YEAR;
  let month = 0;
  while (day >= monthLengths[month]) day -= monthLengths[month++];
  return { year, month: month + 1, day: day + 1, hour: elapsed - days * HOURS_PER_DAY + 1 };
}

export function engineHour(date: GameDate): number | null {
  const { year, month, day, hour } = date;
  if (![year, month, day, hour].every(Number.isSafeInteger) || year < 0
    || month < 1 || month > 12 || day < 1 || day > monthLengths[month - 1]
    || hour < 1 || hour > 24) return null;
  const days = year * DAYS_PER_YEAR + monthLengths.slice(0, month - 1).reduce<number>((sum, length) => sum + length, 0) + day - 1;
  const result = HOUR_EPOCH + days * HOURS_PER_DAY + hour - 1;
  return Number.isSafeInteger(result) && result >= MIN_HOUR && result <= MAX_HOUR ? result : null;
}

export function formatTime(hour: number): string {
  return `${String(hour).padStart(2, '0')}:00`;
}

export function formatDate(date: GameDate, longMonth = false): string {
  const month = monthNames[date.month - 1];
  return `${date.day} ${longMonth ? month : month.slice(0, 3)} ${date.year}`;
}

export function formatDateTime(hour: number): string {
  const date = gameDate(hour);
  return date ? `${formatDate(date)} · ${formatTime(date.hour)}`
    : isEngineHour(hour) ? `Hour ${hour.toLocaleString('en-US')}` : 'Unavailable date';
}

export function formatChartDate(hour: number): string {
  const date = gameDate(hour);
  return date ? `${formatDate(date)}\n${formatTime(date.hour)}` : formatDateTime(hour);
}

export function monthIndex(date: GameDate): number {
  return date.year * 12 + date.month - 1;
}

export function monthAt(index: number): { year: number; month: number } {
  const year = Math.floor(index / 12);
  return { year, month: index - year * 12 + 1 };
}

// Month starts may fall just beyond i32 at the very first/last representable
// date. Keep the grid arithmetic wide; only a validated hour is selectable.
export function firstHourOfMonth(index: number): number {
  const { year, month } = monthAt(index);
  const days = year * DAYS_PER_YEAR + monthLengths.slice(0, month - 1).reduce<number>((sum, length) => sum + length, 0);
  return HOUR_EPOCH + days * HOURS_PER_DAY;
}
