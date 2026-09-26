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
/** A level is used only when its periods are at least this wide, which keeps the
 * axis quiet: years across a campaign, months only when zoomed in. */
const MIN_PERIOD_PX = 80;

/** The finest level whose periods are wide enough for a label. */
export function tickLevel(firstHour: number, lastHour: number, plotWidth: number, minLabelPx = MIN_PERIOD_PX): TickLevel {
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
    const label = level === 'month' || level === 'quarter' ? (month === 0 ? `${SHORT[month]} ${year}` : SHORT[month])
      : level === 'year' ? String(year) : `${year}s`;
    periods.push({ start: firstHourOfMonth(index), end: firstHourOfMonth(index + months), label, level });
  }
  return periods;
}

/** Gridlines at every period start inside the range, and a label for each period
 * whose visible part is wide enough, centred on that part (quarters at their start).
 * A partial first or last period is still labelled when it has room. */
export function timeTicks(firstHour: number, lastHour: number, plotWidth: number, measure: (text: string) => number = text => text.length * 7, minLabelPx = MIN_PERIOD_PX): TimeTicks {
  const level = tickLevel(firstHour, lastHour, plotWidth, minLabelPx);
  const periods = periodsFor(level, firstHour, lastHour);
  const perHour = plotWidth / Math.max(1, lastHour - firstHour);
  const grid = periods.map(period => period.start).filter(hour => hour > firstHour && hour < lastHour);
  const labels: TimeTicks['labels'] = [];
  for (const period of periods) {
    // Quarters are labelled at their first month's tick, where the name belongs.
    if (level === 'quarter') {
      const half = (measure(period.label) / 2 + 4) / perHour;
      if (period.start - half >= firstHour && period.start + half <= lastHour) labels.push({ hour: period.start, text: period.label });
      continue;
    }
    const from = Math.max(period.start, firstHour);
    const to = Math.min(period.end, lastHour);
    if ((to - from) * perHour >= measure(period.label) + 8) labels.push({ hour: (from + to) / 2, text: period.label });
  }
  // Months and quarters name the year only on January; make sure the first label shows
  // it too, so a view without a visible January still says which year it is.
  const firstLabel = labels[0];
  const year = firstLabel && gameDate(Math.floor(firstLabel.hour))?.year;
  if ((level === 'month' || level === 'quarter') && firstLabel && year != null && !firstLabel.text.endsWith(` ${year}`)) firstLabel.text = `${firstLabel.text} ${year}`;
  return { level, grid, labels };
}
