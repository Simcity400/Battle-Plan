/** A race widget's lines over time and one-country mix, as every race widget draws
 * them (Factories, Equipment produced): runs cut to the hours shown, and the lines'
 * tooltip naming each country as recorded at the hour hovered. */
import { clipToSpans, runAt } from '../series.ts';
import type { Run, Span } from '../series';
import type { TooltipContent, TooltipRow } from '../tooltip';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';

/** Runs cut to the hours a country is shown (`membership`: a group's hours to cut
 * to; null: whenever recorded) and to the hours the widget shows (`range`: a date
 * range or the branch), so nothing outside them is drawn, fitted or hovered, even
 * when an axis is dragged past the range. */
export function shownRuns(runs: readonly Run[], membership: readonly Span[] | null, range: Span): Run[] {
  return clipToSpans(membership ? clipToSpans(runs, membership) : runs, [range]);
}

/** A country as a tooltip names it: as recorded at the hour hovered. */
export type LabelAt = (hour: number) => CountryLabel;

/** The lines' tooltip at an hour: each line recorded then, highest first (ties in the
 * product's tag order), named and flagged as recorded at that hour, its value written
 * by `format`. */
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

/** The countries a lines tooltip at `hour` names: those with a line recorded then. */
export function linesAt(hour: number, lines: readonly { tag: string; runs: readonly Run[] }[]): string[] {
  return [...new Set(lines.filter(line => runAt(line.runs, hour) != null).map(line => line.tag))];
}
