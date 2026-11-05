import type { CountryFactoriesReply } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { clipToSpans, overlayRuns, runIndexAt } from '../../charts/series';
import { overlayCountryIdentities } from '../../dashboard/countryIdentity';
import { summaryShareRuns } from '../../charts/share';
import { countryPageJoin, registerPageJoin } from '../../dashboard/pageJoins';

type FactoriesPage = CountryFactoriesReply & { summaries?: { tag: string; summary: ChartSummary }[] };
registerPageJoin(['country-factories'], countryPageJoin<FactoriesPage>(mergeFactories));

export function mergeFactories(replies: readonly (CountryFactoriesReply | undefined)[]): CountryFactoriesReply | undefined {
  const pages = replies.filter((page): page is CountryFactoriesReply => !!page);
  if (!pages.length) return undefined;
  const series = new Map<string, CountryFactoriesReply['series'][number]>();
  for (const page of pages) for (const country of page.series) {
    const before = series.get(country.tag);
    const summary = (page as CountryFactoriesReply & { summaries?: {tag:string;summary:ChartSummary}[] }).summaries?.find(line=>line.tag===country.tag)?.summary;
    const within = summary ? [{firstHour:Math.floor(summary.from),lastHour:Math.floor(summary.to)}] : undefined;
    const runs = overlayRuns(before?.runs ?? [],country.runs,(a,b)=>a.values.length===b.values.length && a.values.every((value,index)=>value===b.values[index]),within);
    const reads = overlayRuns(before?.reads ?? [],country.reads ?? [],(a,b)=>a.hourly===b.hourly && (a.hourly || a.readHour===b.readHour),within).map(read=>read.hourly ? {...read,readHour:read.firstHour} : read);
    series.set(country.tag, { ...before, ...country, runs, reads,
      identities: overlayCountryIdentities([{tag:country.tag,identities:before?.identities ?? []}],
        [{tag:country.tag,identities:country.identities ?? []}])[0].identities,
      segments: clipToSpans([{firstHour:-2147483648,lastHour:2147483647}],[...(before?.segments ?? []),...country.segments]),
      membership: before?.membership === null || country.membership === null ? null
        : clipToSpans([{ firstHour: -2147483648, lastHour: 2147483647 }], [...(before?.membership ?? []), ...(country.membership ?? [])]) });
  }
  return { ...pages[0], series: [...series.values()], warnings: [...new Set(pages.flatMap(page => page.warnings))] };
}

export function factoriesGeometry(reply: (CountryFactoriesReply & { summaries?: { tag: string; summary: ChartSummary }[] }) | undefined): CountryFactoriesReply | undefined {
  if (!reply) return undefined;
  return { ...reply, series: reply.series.map(series => {
    const summary = reply.summaries?.find(line => line.tag === series.tag)?.summary;
    if (!summary || summary.runs !== undefined) return series;
    const runs = summaryShareRuns(series.runs.map(run => ({ ...run, counts: run.values })), [summary]);
    return { ...series, runs: runs.map(run => ({ ...series.runs[runIndexAt(series.runs, run.firstHour)],
      firstHour: run.firstHour, lastHour: run.lastHour, startsSegment: run.startsSegment === true, values: [...run.counts] })) };
  }) };
}
