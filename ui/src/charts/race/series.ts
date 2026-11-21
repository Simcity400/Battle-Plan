import { IntervalIndex } from '../intervalIndex.ts';
import { runAt, stretchAt, stretchStarts } from '../series.ts';
import type { Run } from '../series';
import type { CountryRaceEntry } from './countryRace';
import type { TooltipContent } from '../tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { compareTags } from '../../dashboard/tagOrder.ts';

export interface RaceSeries { tag: string; runs: readonly Run[] }

export function raceSeriesIndex(lines: readonly RaceSeries[]) {
  const series = new Map(lines.map(line => [line.tag, { ...line, starts: stretchStarts(line.runs) }]));
  const intervals = lines.flatMap(line => line.runs.map(run => ({ ...run, tag: line.tag })));
  const index = new IntervalIndex(intervals);
  return { series, index };
}

export function seriesRaceEntries(model: ReturnType<typeof raceSeriesIndex>, hour: number, count: number,
  format: (value: number) => string): CountryRaceEntry[] {
  return seriesAt(model, hour).flatMap(tag => {
    const line = model.series.get(tag)!;
    const run = runAt(line.runs, Math.floor(hour));
    return run ? [{ tag, value: run.value, label: format(run.value), stretch: stretchAt(line.runs, line.starts, hour)! }] : [];
  }).sort((a, b) => b.value - a.value || compareTags(a.tag, b.tag)).slice(0, Math.max(0, Math.floor(count)));
}

export function seriesAt(model: ReturnType<typeof raceSeriesIndex>, hour: number): string[] {
  return [...new Set(model.index.at(Math.floor(hour)).map(run => run.tag))];
}

export function seriesTooltip(model: ReturnType<typeof raceSeriesIndex>, hour: number,
  label: CountryLabel, color: (tag: string) => string,
  format: (value: number) => string, title: string, subtitle: string, countryFlags = true): TooltipContent {
  const values = seriesAt(model, hour).map(tag => ({ tag, value: runAt(model.series.get(tag)!.runs, Math.floor(hour))!.value }))
    .sort((a, b) => b.value - a.value || compareTags(a.tag, b.tag));
  return { title, subtitle, rows: values.length ? values.map(({ tag, value }) => {
    const country = label(tag);
    return { label: country.name, countryFlag: countryFlags ? country : undefined, swatch: color(tag), value: format(value), short: true };
  }) : [{ label: 'Not recorded', tone: 'muted' }] };
}
