import { formatDate, formatDateTime, gameDate } from './calendar.ts';

/** The first hour of the stretch `hour` lies in: the last of `restarts`, where totals restart
 * after hours the recording lost, at or before it. Null in the branch's first stretch. */
export function stretchSince(restarts: readonly number[], hour: number): number | null {
  let since: number | null = null;
  for (const restart of restarts) if (restart <= hour && (since === null || restart > since)) since = restart;
  return since;
}

/** How a total counted from `since`, after hours the recording lost, is labelled. */
export function sinceLabel(since: number): string {
  const date = gameDate(since);
  return `since ${date ? formatDate(date) : formatDateTime(since)}`;
}
