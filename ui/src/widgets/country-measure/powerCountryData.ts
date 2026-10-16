import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import type { PoliticsLedgerReply } from '../../generated/dashboard';

/** Keep event dates and supplying reads while joining only the selected holder's aliases. */
export function powerCountryLedger(replies: readonly PoliticsLedgerReply[], spans: ReadonlyMap<string, readonly Span[]>, tag: string, countryId: number): PoliticsLedgerReply | undefined {
  const aliases = replies.filter(reply => spans.get(reply.tag)?.length);
  if (!aliases.length) return undefined;
  const selected = aliases.map(reply => {
    const known = spans.get(reply.tag) ?? [];
    const at = (hour: number) => runAt(known, hour) !== null;
    const id = (id: string) => JSON.stringify([reply.tag, id]);
    return { ...reply,
      daily: reply.daily?.filter(row => row.country === countryId && at(row.dayHour)),
      balanceChanges: reply.balanceChanges?.filter(row => row.countryId === countryId && at(row.hour)),
      spending: [...reply.spending, ...(reply.recurring ?? []).map(row => row.spend)].filter(row => row.countryId === countryId && at(row.hour)).map(row => ({ ...row, id: id(row.id), itemId: id(row.itemId) })),
      spendItems: reply.spendItems.map(row => ({ ...row, id: id(row.id) })),
      spendingCoverage: clipToSpans(reply.spendingCoverage.map(row => ({ ...row,
        firstHour: row.sinceHour ?? row.readHour, lastHour: row.throughHour ?? row.readHour })), known)
        .map(({ firstHour, lastHour, ...row }) => ({ ...row, sinceHour: firstHour, throughHour: lastHour })),
      commandHolds: reply.commandHolds.filter(row => at(row.readHour)).map(row => ({ ...row,
        throughHour: Math.min(row.throughHour, runAt(known, row.readHour)!.lastHour) })),
    };
  });
  const spendingCoverage: PoliticsLedgerReply['spendingCoverage'] = [];
  for (const row of selected.flatMap(reply => reply.spendingCoverage).sort((a, b) => (a.sinceHour ?? a.readHour) - (b.sinceHour ?? b.readHour))) {
    const previous = spendingCoverage.at(-1);
    if (previous && previous.throughHour !== null && row.sinceHour !== null && previous.throughHour + 1 === row.sinceHour
      && row.readHour <= row.sinceHour && previous.political === row.political && previous.command === row.command && previous.reason === row.reason) {
      previous.throughHour = row.throughHour;
    } else spendingCoverage.push({ ...row });
  }
  return { ...aliases[0], tag, mutations: [], governments: [], dailyCosts: [], coverage: [], ideaTooltips: null,
    daily: selected.flatMap(reply => reply.daily ?? []).sort((a, b) => a.dayHour - b.dayHour || a.readHour - b.readHour),
    balanceChanges: selected.flatMap(reply => reply.balanceChanges ?? []).sort((a, b) => a.hour - b.hour),
    spending: selected.flatMap(reply => reply.spending).sort((a, b) => a.hour - b.hour),
    spendItems: selected.flatMap(reply => reply.spendItems),
    spendCategories: [...new Map(selected.flatMap(reply => reply.spendCategories).map(row => [row.id, row])).values()],
    spendingCoverage, recurring: undefined,
    commandHolds: selected.flatMap(reply => reply.commandHolds).sort((a, b) => a.readHour - b.readHour),
    warnings: selected.flatMap(reply => reply.warnings) };
}
