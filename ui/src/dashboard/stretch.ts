import { formatDate, formatDateTime, gameDate } from './calendar.ts';

export function stretchSince(restarts: readonly number[], hour: number): number | null {
  let since: number | null = null;
  for (const restart of restarts) if (restart <= hour && (since === null || restart > since)) since = restart;
  return since;
}

export function sinceLabel(since: number): string {
  const date = gameDate(since);
  return `since ${date ? formatDate(date) : formatDateTime(since)}`;
}
