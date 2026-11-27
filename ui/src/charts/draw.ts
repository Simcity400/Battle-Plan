import { formatDate, gameDate } from '../dashboard/calendar.ts';

export const LABEL_WEIGHT = 500;
export const VALUE_WEIGHT = 600;

export function dayLabel(hour: number): string {
  const date = gameDate(Math.floor(hour));
  return date ? formatDate(date) : `Hour ${Math.floor(hour)}`;
}
