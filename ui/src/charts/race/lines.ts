import { runAt } from '../series.ts';
import type { Run } from '../series';
import type { TooltipContent, TooltipRow } from '../tooltip';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';

export type LabelAt = (hour: number) => CountryLabel;

export function linesTooltip(hour: number, lines: readonly { tag: string; runs: readonly Run[] }[], labelAt: LabelAt, colour: (tag: string) => string,
  date: string, subtitle: string, format: (value: number) => string): TooltipContent {
  const label = labelAt(hour);
  const values = lines.flatMap(line => { const run = runAt(line.runs, hour); return run ? [{ tag: line.tag, value: run.value }] : []; })
    .sort((a, b) => b.value - a.value || compareTags(a.tag, b.tag));
  const rows: TooltipRow[] = values.map(entry => {
    const country = label(entry.tag);
    return { label: country.name, countryFlag: country, value: format(entry.value), swatch: colour(entry.tag), short: true };
  });
  return { title: date, subtitle, rows: rows.length ? rows : [{ label: 'Not recorded', tone: 'muted' }] };
}

export function linesAt(hour: number, lines: readonly { tag: string; runs: readonly Run[] }[]): string[] {
  return [...new Set(lines.filter(line => runAt(line.runs, hour) != null).map(line => line.tag))];
}
