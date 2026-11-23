import { usNumber } from '../charts/numberFormat.ts';

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
    : isEngineHour(hour) ? `Hour ${usNumber.format(hour)}` : 'Unavailable date';
}

export function formatDay(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date) : isEngineHour(hour) ? `Hour ${usNumber.format(hour)}` : 'Unavailable date';
}

export function formatDateRange(firstHour: number, lastHour: number): string {
  const start = gameDate(firstHour), end = gameDate(lastHour);
  if (!start || !end) return `${formatDay(firstHour)} – ${formatDay(lastHour)}`;
  if (start.year === end.year && start.month === end.month && start.day === end.day) return formatDate(start);
  const startMonth = monthNames[start.month - 1].slice(0, 3);
  const endMonth = monthNames[end.month - 1].slice(0, 3);
  if (start.year === end.year && start.month === end.month) return `${start.day}–${end.day} ${endMonth} ${end.year}`;
  if (start.year === end.year) return `${start.day} ${startMonth} – ${end.day} ${endMonth} ${end.year}`;
  return `${formatDate(start)} – ${formatDate(end)}`;
}

export function formatHourSpan(firstHour: number, lastHour: number): string {
  const start = gameDate(firstHour), end = gameDate(lastHour);
  if (!start || !end) return `${formatDateTime(firstHour)} to ${formatDateTime(lastHour)}`;
  if (start.year === end.year && start.month === end.month && start.day === end.day) {
    return `${formatDate(start)} · ${formatTime(start.hour)} to ${formatTime(end.hour)}`;
  }
  return `${formatDateTime(firstHour)} to ${formatDateTime(lastHour)}`;
}

export function monthIndex(date: GameDate): number {
  return date.year * 12 + date.month - 1;
}

export function monthAt(index: number): { year: number; month: number } {
  const year = Math.floor(index / 12);
  return { year, month: index - year * 12 + 1 };
}

export function firstHourOfMonth(index: number): number {
  const { year, month } = monthAt(index);
  const days = year * DAYS_PER_YEAR + monthLengths.slice(0, month - 1).reduce<number>((sum, length) => sum + length, 0);
  return HOUR_EPOCH + days * HOURS_PER_DAY;
}
