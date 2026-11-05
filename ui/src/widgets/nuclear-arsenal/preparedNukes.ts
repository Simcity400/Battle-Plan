import type { CountryNukesReply } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { clipToSpans, overlayRuns, runIndexAt } from '../../charts/series';
import { summaryShareRuns } from '../../charts/share';
import { countryPageJoin, registerPageJoin } from '../../dashboard/pageJoins';

type NukesPage = CountryNukesReply & { summaries?: { tag: string; summary: ChartSummary }[] };
registerPageJoin(['country-nukes'], countryPageJoin<NukesPage>(mergeNukes));

export function mergeNukes(replies: readonly (CountryNukesReply | undefined)[]): CountryNukesReply | undefined {
  const pages = replies.filter((page): page is CountryNukesReply => !!page);
  if (!pages.length) return undefined;
  const series = new Map<string, CountryNukesReply['series'][number]>();
  for (const page of pages) for (const country of page.series) {
    const before = series.get(country.tag);
    const summary = (page as CountryNukesReply & {summaries?: {tag:string;summary:ChartSummary}[]}).summaries?.find(line=>line.tag===country.tag)?.summary;
    const runs = overlayRuns(before?.runs ?? [],country.runs,(a,b)=>a.total===b.total && a.counts.length===b.counts.length
      && a.counts.every((count,index)=>count.ordinal===b.counts[index].ordinal && count.value===b.counts[index].value),
      summary ? [{firstHour:Math.floor(summary.from),lastHour:Math.floor(summary.to)}] : undefined);
    const spans = [...(before?.membership ?? []), ...(country.membership ?? [])];
    series.set(country.tag, { ...before, ...country, runs,
      segments: clipToSpans([{firstHour:-2147483648,lastHour:2147483647}],[...(before?.segments ?? []),...country.segments]),
      membership: before?.membership === null || country.membership === null ? null
        : clipToSpans([{ firstHour: -2147483648, lastHour: 2147483647 }], spans) });
  }
  const dates = (key: 'readHours' | 'emptyReadHours') => [...new Set(pages.flatMap(page => page[key]))].sort((a, b) => a - b);
  return { ...pages[0], series: [...series.values()], readHours: dates('readHours'), emptyReadHours: dates('emptyReadHours'),
    warnings: [...new Set(pages.flatMap(page => page.warnings))] };
}

export function nukesGeometry(reply: (CountryNukesReply & { summaries?: { tag: string; summary: ChartSummary }[] }) | undefined): CountryNukesReply | undefined {
  if (!reply) return undefined;
  return { ...reply, series: reply.series.map(series => {
    const summary = reply.summaries?.find(line => line.tag === series.tag)?.summary;
    if (!summary || summary.runs !== undefined || series.runs.some(run => run.total === null || run.counts.length !== reply.types.length)) return series;
    const runs = summaryShareRuns(series.runs.map(run => ({ ...run, counts: run.counts.map(count => count.value) })), [summary]);
    return { ...series, runs: runs.map(run => ({ ...series.runs[runIndexAt(series.runs, run.firstHour)],
      firstHour: run.firstHour, lastHour: run.lastHour, startsSegment: run.startsSegment === true,
      counts: reply.types.map((type, index) => ({ ordinal: type.ordinal, value: run.counts[index] })) })) };
  }) };
}
