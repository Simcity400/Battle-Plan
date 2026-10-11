import type { RaidMetric, RaidTallyRun, RaidTallySeries, RaidsReply } from '../../generated/dashboard';
import type { CountryRaceEntry, CountryRaceLine } from '../../charts/race/countryRace';
import type { Span } from '../../charts/series';
import { clipToSpans, runAt, runIndexAt, stretchStarts } from '../../charts/series.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { shownRuns } from '../../charts/race/lines.ts';
import type { LabelAt } from '../../charts/race/lines';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { SECONDARY_DASH } from '../../charts/palette.ts';

export type TallyMetric = RaidMetric;
export const tallyLabel = (metric: TallyMetric) => metric === 'both' ? 'Observed raids launched and succeeded'
  : metric === 'launched' ? 'Observed raid launches' : 'Observed raid successes';
export const tallyValue = (value: number) => usNumber.format(value);
export const tallyCount = (value: number) => `${tallyValue(value)}+`;
/** Scan::observe extends a tally run only after that country's successful read,
 * at the immediately following hour. Every included hour is its own exact read. */
export const tallyReadHour = (run: RaidTallyRun, hour: number) => Math.max(run.firstHour, Math.min(Math.floor(hour), run.lastHour));
export const tallyModel = (series: readonly RaidTallySeries[]) => new Map(series.map(item => [item.tag, { ...item, starts: stretchStarts(item.runs) }]));
export const tallyIdentities = (series: readonly RaidTallySeries[]) => series.every(item => item.identities !== undefined)
  ? countryIdentityIndex(series.map(item => ({ tag: item.tag, identities: item.identities! }))) : null;
type TallySeries = RaidTallySeries & { starts: number[] };
export const tallySegmentAt = (series: ReadonlyMap<string, TallySeries>, tag: string, hour: number): number | null => {
  const item = series.get(tag);
  return runAt(item?.identities ?? item?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
};

export function tallyEntries(series: ReadonlyMap<string, TallySeries>, candidates: readonly string[], hour: number, metric: TallyMetric, limit?: number,
  omitZero = false): CountryRaceEntry[] {
  const rankBy = metric === 'succeeded' ? 'succeeded' : 'launched';
  return candidates.flatMap(tag => {
    const item = series.get(tag);
    const index = runIndexAt(item?.runs ?? [], hour);
    const run = index < 0 ? null : item!.runs[index];
    const identity = runAt(item?.identities ?? item?.segments ?? [], hour);
    if (!run || !identity || (omitZero && run[rankBy] === 0 && (metric !== 'both' || run.succeeded === 0))) return [];
    return [{ tag, value: run[rankBy], label: metric === 'both' ? tallyBoth(run) : tallyCount(run[rankBy]), stretch: JSON.stringify([identity.firstHour, item!.starts[index]]),
      ...(metric === 'both' ? { inset: run.succeeded } : {}) }];
  }).sort((a, b) => b.value - a.value || compareTags(a.tag, b.tag)).slice(0, limit);
}

export function tallyLines(series: readonly RaidTallySeries[], metric: TallyMetric, range: Span, membership: ReadonlyMap<string, readonly Span[]> | null): CountryRaceLine[] {
  return series.flatMap(item => {
    const spans = item.identities ?? item.segments;
    const starts = new Set(spans.map(segment => segment.firstHour));
    // Equal counts may compress across a holder change; split at the recorded
    // identity spans before group/range clipping so those countries stay apart.
    const measures = metric === 'both' ? ['launched', 'succeeded'] as const : [metric];
    return measures.flatMap(measure => {
      const identityRuns = clipToSpans(item.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: run[measure] })), spans)
        .map(run => ({ ...run, breakBefore: starts.has(run.firstHour) }));
      const runs = shownRuns(identityRuns, membership ? membership.get(item.tag) ?? [] : item.membership, range);
      return runs.length ? [{ tag: item.tag, runs, ...(metric === 'both' ? {
        id: JSON.stringify([item.tag, measure]), measure, ...(measure === 'succeeded' ? { dash: SECONDARY_DASH } : {}),
      } : {}) }] : [];
    });
  });
}
const tallyBoth = (run: RaidTallyRun) => `${tallyCount(run.launched)} launched · ${tallyCount(run.succeeded)} succeeded`;

export function tallyTooltip(data: RaidsReply, tag: string, hour: number, name: string): TooltipContent {
  const run = runAt(data.series.find(item => item.tag === tag)?.runs ?? [], Math.floor(hour));
  return { title: name, subtitle: formatDateTime(hour), rows: run ? [
    { label: 'Observed launches', value: tallyCount(run.launched) },
    { label: 'Observed successes', value: tallyCount(run.succeeded) },
    { label: 'Read', value: formatDateTime(tallyReadHour(run, hour)) },
  ] : [{ label: 'Not recorded at this time', tone: 'muted' }],
  note: 'Counts enter at the read that proves them, so their dates are upper bounds. An observed in-progress phase or nonzero recorded flight distance proves launch; terminal outcomes alone do not. Success means the game’s success or critical-success outcome. A + means earlier or between-read events may be absent; gaps stay unknown.' };
}

export function tallyLineTooltip(series: ReadonlyMap<string, RaidTallySeries>, hour: number, tags: readonly string[],
  labelAt: LabelAt, country: (tag: string) => string, metric: TallyMetric): TooltipContent {
  const entries = [...new Set(tags)].flatMap(tag => { const run = runAt(series.get(tag)?.runs ?? [], Math.floor(hour)); return run ? [{ tag, run }] : []; });
  return { title: tallyLabel(metric), subtitle: formatDateTime(hour), rows: entries.length ? entries.map(({ tag, run }) => {
    const label = labelAt(hour)(tag);
    return { label: label.name, countryFlag: label, swatch: country(tag), value: metric === 'both' ? tallyBoth(run) : tallyCount(run[metric]) };
  }) : [{ label: 'Not recorded at this time', tone: 'muted' }],
  note: entries.length ? `Read ${formatDateTime(tallyReadHour(entries[0].run, hour))}. A + is an observed lower bound. Events enter by the read proving them; between-read launches may be unknown. Success excludes limited success.` : undefined };
}
