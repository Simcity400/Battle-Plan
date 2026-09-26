import { firstHourOfMonth, gameDate, monthIndex, monthNames } from '../dashboard/calendar.ts';

/** Time-axis ticks from the recording's own hours: no year is ever assumed, so a
 * campaign starting in 1910 or 2000 gets the same treatment as one in 1936. */

export type TickLevel = 'decade' | 'year' | 'quarter' | 'month' | 'day';
/** A period on the axis: gridlines at its start, a label centred on its visible part. */
export interface TimePeriod { start: number; end: number; label: string; level: TickLevel }
export interface TimeTicks { level: TickLevel; grid: number[]; labels: { hour: number; text: string }[] }

const HOURS_PER_DAY = 24;
const SHORT = monthNames.map(name => name.slice(0, 3));
/** Rough period lengths, only for choosing a level. */
const TYPICAL: Record<TickLevel, number> = { decade: 87_600, year: 8_760, quarter: 2_190, month: 730, day: 24 };
const ORDER: readonly TickLevel[] = ['day', 'month', 'quarter', 'year', 'decade'];

/** The finest level whose periods are wide enough for a label. */
export function tickLevel(firstHour: number, lastHour: number, plotWidth: number, minLabelPx = 56): TickLevel {
  const perHour = plotWidth / Math.max(1, lastHour - firstHour);
  return ORDER.find(level => TYPICAL[level] * perHour >= minLabelPx) ?? 'decade';
}

function periodsFor(level: TickLevel, firstHour: number, lastHour: number): TimePeriod[] {
  // A zoomed view has fractional bounds; dates exist only for whole hours.
  const first = gameDate(Math.floor(firstHour));
  const last = gameDate(Math.ceil(lastHour));
  if (!first || !last) return [];
  const periods: TimePeriod[] = [];
  if (level === 'day') {
    const start = Math.floor(firstHour) - (first.hour - 1);
    for (let hour = start; hour <= lastHour; hour += HOURS_PER_DAY) {
      const date = gameDate(hour);
      if (date) periods.push({ start: hour, end: hour + HOURS_PER_DAY, label: `${date.day} ${SHORT[date.month - 1]}`, level });
    }
    return periods;
  }
  const months = level === 'month' ? 1 : level === 'quarter' ? 3 : level === 'year' ? 12 : 120;
  const firstMonth = monthIndex(first);
  let index = firstMonth - (level === 'decade' ? firstMonth % 120 : firstMonth % months);
  const lastMonth = monthIndex(last);
  for (; index <= lastMonth; index += months) {
    const year = Math.floor(index / 12);
    const month = index - year * 12;
    const label = level === 'month' ? (month === 0 ? `${SHORT[month]} ${year}` : SHORT[month])
      : level === 'quarter' ? `Q${month / 3 + 1} ${year}`
        : level === 'year' ? String(year) : `${year}s`;
    periods.push({ start: firstHourOfMonth(index), end: firstHourOfMonth(index + months), label, level });
  }
  return periods;
}

/** Gridlines at every period start inside the range, and a label for each period
 * whose visible part is wide enough, centred on that part. A partial first or
 * last period is still labelled when it has room. */
export function timeTicks(firstHour: number, lastHour: number, plotWidth: number, measure: (text: string) => number = text => text.length * 7, minLabelPx = 56): TimeTicks {
  const level = tickLevel(firstHour, lastHour, plotWidth, minLabelPx);
  const periods = periodsFor(level, firstHour, lastHour);
  const perHour = plotWidth / Math.max(1, lastHour - firstHour);
  const grid = periods.map(period => period.start).filter(hour => hour > firstHour && hour < lastHour);
  const labels: TimeTicks['labels'] = [];
  for (const period of periods) {
    const from = Math.max(period.start, firstHour);
    const to = Math.min(period.end, lastHour);
    if ((to - from) * perHour >= measure(period.label) + 8) labels.push({ hour: (from + to) / 2, text: period.label });
  }
  return { level, grid, labels };
}
