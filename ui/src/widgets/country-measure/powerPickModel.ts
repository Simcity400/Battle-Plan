import type { CountryStabilityReply, PoliticsLedgerReply } from '../../generated/dashboard';

/** These projections are compared only when a product reply changes. Text and
 * artwork refreshes do not change the owner of the native numeric geometry. */
export function powerMaximaModel(reply: CountryStabilityReply) {
  const series = reply.series.map(row => ({ tag: row.tag, maxCommandPower: row.maxCommandPower, carried: row.carried }));
  return { decimals: reply.decimals, series, key: JSON.stringify([reply.decimals, series]) };
}

const ledgerModels = new WeakMap<PoliticsLedgerReply, string>();
export function powerLedgerModelKey(reply: PoliticsLedgerReply): string {
  const held = ledgerModels.get(reply);
  if (held !== undefined) return held;
  const key = JSON.stringify([
    reply.tag, reply.decimals, reply.daily ?? [],
    reply.spending.map(row => [row.id, row.countryId, row.hour, row.readHour, row.currency, row.category, row.chargeRaw,
      row.realizedDeltaRaw, row.covered, row.occurrences, row.changes]),
    reply.recurring ?? [],
    reply.spendCategories.map(row => row.id),
    reply.spendingCoverage.map(row => [row.readHour, row.sinceHour, row.throughHour, row.political, row.command]),
    (reply.balanceChanges ?? []).map(row => [row.countryId, row.hour, row.readHour, row.currency, row.amountRaw, row.realizedDeltaRaw, row.covered]),
    reply.commandHolds.map(row => [row.readHour, row.throughHour, row.totalRaw,
      row.holders.map(holder => [holder.id, holder.kind, holder.amountRaw])]),
  ]);
  ledgerModels.set(reply, key);
  return key;
}
