import { clipToSpans } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { drillHour } from '../../charts/drill/drillState.ts';
import type { DrillState } from '../../charts/drill/drillState';
import { signed32 } from '../../dashboard/validation.ts';
import type { ResourceBalanceReply, TradeRun } from '../../generated/dashboard';

import type { DatedCountrySelection as ResourceCountrySelection } from '../../dashboard/datedCountry';
export { datedCountryToken as resourceCountryToken, readDatedCountry as readResourceCountry, datedCountrySpans as resourceCountrySpans } from '../../dashboard/datedCountry.ts';
export type { DatedCountrySelection as ResourceCountrySelection } from '../../dashboard/datedCountry';

export function resourceExtent(selection: ResourceCountrySelection | null, firstHour: number, lastHour: number): [number, number] | null {
  if (!selection?.extent) return null;
  const first = firstHour + selection.extent[0], last = firstHour + selection.extent[1];
  return signed32(first) && signed32(last) && first >= firstHour && last <= lastHour ? [first, last] : null;
}

/** Keep the resource catalogue and exact pools; only identity coverage is cut. */
export function resourceCountryData(replies: readonly ResourceBalanceReply[], spans: ReadonlyMap<string, readonly Span[]>, tag: string): ResourceBalanceReply | undefined {
  const first = replies[0];
  if (!first) return undefined;
  const series = new Map<number, ResourceBalanceReply['series'][number]>();
  const trades: ResourceBalanceReply['trades'] = [], dealHours: ResourceBalanceReply['dealHours'] = [];
  for (const reply of replies) {
    const coverage = spans.get(reply.tag) ?? [];
    for (const item of reply.series) {
      const runs = clipToSpans(item.runs, coverage), previous = series.get(item.id);
      if (previous) previous.runs.push(...runs); else series.set(item.id, { ...item, runs });
    }
    trades.push(...clipResourceTrades(reply.trades, coverage));
    dealHours.push(...clipToSpans(reply.dealHours, coverage));
  }
  return { ...first, tag, series: [...series.values()].map(item => ({ ...item, runs: item.runs.sort((a, b) => a.firstHour - b.firstHour) })),
    trades, dealHours: dealHours.sort((a, b) => a.firstHour - b.firstHour), warnings: [...new Set(replies.flatMap(reply => reply.warnings))] };
}

/** Deals overlap each other; each finds only the membership spans it meets. */
export function clipResourceTrades(trades: readonly TradeRun[], spans: readonly Span[]): TradeRun[] {
  const clipped: TradeRun[] = [];
  for (const trade of trades) {
    let low = 0, high = spans.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (spans[middle].lastHour < trade.firstHour) low = middle + 1; else high = middle;
    }
    for (let index = low; index < spans.length && spans[index].firstHour <= trade.lastHour; index++) {
      const firstHour = Math.max(trade.firstHour, spans[index].firstHour), lastHour = Math.min(trade.lastHour, spans[index].lastHour);
      clipped.push({ ...trade, firstHour, lastHour, startsSegment: trade.startsSegment && firstHour === trade.firstHour });
    }
  }
  return clipped;
}

export const resourceMoment = (drill: DrillState, saved: boolean, shownHour: number, clockHour: number) =>
  saved && drill.breakdown === null ? Math.floor(shownHour) : drillHour(drill, clockHour);
