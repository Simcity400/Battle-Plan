import { formatDate, gameDate } from '../../dashboard/calendar.ts';
import type { ResearchCoverage } from '../../generated/dashboard';

export function coverageAt(coverage: readonly ResearchCoverage[], hour: number): ResearchCoverage[] {
  return coverage.filter(span => span.firstHour <= hour).map(span => ({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, hour) }));
}

export function researchDate(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date) : 'Unavailable date';
}
