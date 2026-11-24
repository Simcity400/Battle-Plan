import type { RaidMetric, RaidTallyRun, RaidTallySeries } from '../../dashboard/raids';
import type { CountryRaceLine } from '../../charts/race/countryRace';
import type { Span } from '../../charts/series';
import { clipToSpans, runAt, stretchStarts } from '../../charts/series.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { shownRuns } from '../../charts/race/lines.ts';
import type { LabelAt } from '../../charts/race/lines';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { SECONDARY_DASH } from '../../charts/palette.ts';

export type TallyMetric = RaidMetric;
export const tallyLabel = (metric: TallyMetric) => metric === 'both' ? 'Raid launches and successes'
  : metric === 'launched' ? 'Raid launches' : 'Raid successes';
export const tallyValue = (value: number) => usNumber.format(value);
const tallyCount = (value: number) => `${tallyValue(value)}+`;
export const tallyModel = (series: readonly RaidTallySeries[]) => new Map(series.map(item => [item.tag, { ...item, starts: stretchStarts(item.runs) }]));
export const tallyIdentities = (series: readonly RaidTallySeries[]) => series.every(item => item.identities !== undefined)
  ? countryIdentityIndex(series.map(item => ({ tag: item.tag, identities: item.identities! }))) : null;
type TallySeries = RaidTallySeries & { starts: number[] };
export const tallySegmentAt = (series: ReadonlyMap<string, TallySeries>, tag: string, hour: number): number | null => {
  const item = series.get(tag);
  return runAt(item?.identities ?? item?.segments ?? [], Math.floor(hour))?.firstHour ?? null;
};

export function tallyLines(series: readonly RaidTallySeries[], metric: TallyMetric, range: Span, membership: ReadonlyMap<string, readonly Span[]> | null): CountryRaceLine[] {
  return series.flatMap(item => {
    const spans = item.identities ?? item.segments;
    const starts = new Set(spans.map(segment => segment.firstHour));
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
const NOTE = '+ means at least this many. Success includes critical success and excludes limited success.';

export function tallyLineTooltip(series: ReadonlyMap<string, RaidTallySeries>, hour: number, tags: readonly string[],
  labelAt: LabelAt, country: (tag: string) => string, metric: TallyMetric): TooltipContent {
  const entries = [...new Set(tags)].flatMap(tag => { const run = runAt(series.get(tag)?.runs ?? [], Math.floor(hour)); return run ? [{ tag, run }] : []; });
  return { title: tallyLabel(metric), subtitle: formatDateTime(hour), rows: entries.length ? entries.map(({ tag, run }) => {
    const label = labelAt(hour)(tag);
    return { label: label.name, countryFlag: label, swatch: country(tag), value: metric === 'both' ? tallyBoth(run) : tallyCount(run[metric]) };
  }) : [{ label: 'Not recorded at this time', tone: 'muted' }],
  note: entries.length ? NOTE : undefined };
}
