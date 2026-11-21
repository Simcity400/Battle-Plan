import { firstHourOfMonth, gameDate, monthIndex, monthNames } from '../dashboard/calendar.ts';


export type TickLevel = 'decade' | 'year' | 'quarter' | 'month' | 'day';
export interface TimePeriod { start: number; end: number; label: string; level: TickLevel }
export interface TimeTicks { level: TickLevel; grid: number[]; labels: { hour: number; text: string }[] }

const HOURS_PER_DAY = 24;
const SHORT = monthNames.map(name => name.slice(0, 3));
const TYPICAL: Record<TickLevel, number> = { decade: 87_600, year: 8_760, quarter: 2_190, month: 730, day: 24 };
const ORDER: readonly TickLevel[] = ['day', 'month', 'quarter', 'year', 'decade'];
const MIN_PERIOD_PX = 80;

export function tickLevel(firstHour: number, lastHour: number, plotWidth: number, minLabelPx = MIN_PERIOD_PX): TickLevel {
  const perHour = plotWidth / Math.max(1, lastHour - firstHour);
  return ORDER.find(level => TYPICAL[level] * perHour >= minLabelPx) ?? 'decade';
}

function periodsFor(level: TickLevel, firstHour: number, lastHour: number): TimePeriod[] {
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

export function timeTicks(firstHour: number, lastHour: number, plotWidth: number, measure: (text: string) => number = text => text.length * 7, minLabelPx = MIN_PERIOD_PX): TimeTicks {
  const level = tickLevel(firstHour, lastHour, plotWidth, minLabelPx);
  const periods = periodsFor(level, firstHour, lastHour);
  const perHour = plotWidth / Math.max(1, lastHour - firstHour);
  const grid = periods.map(period => period.start).filter(hour => hour > firstHour && hour < lastHour);
  const labels: TimeTicks['labels'] = [];
  const named = (period: TimePeriod, hour: number) => {
    const year = !labels.length && (level === 'month' || level === 'quarter') ? gameDate(Math.floor(hour))?.year : undefined;
    return year != null && !period.label.endsWith(` ${year}`) ? `${period.label} ${year}` : period.label;
  };
  for (const period of periods) {
    if (level === 'quarter') {
      const text = named(period, period.start);
      const half = (measure(text) / 2 + 4) / perHour;
      if (period.start - half >= firstHour && period.start + half <= lastHour) labels.push({ hour: period.start, text });
      continue;
    }
    const from = Math.max(period.start, firstHour);
    const to = Math.min(period.end, lastHour);
    const text = named(period, (from + to) / 2);
    if ((to - from) * perHour >= measure(text) + 8) labels.push({ hour: (from + to) / 2, text });
  }
  return { level, grid, labels };
}
