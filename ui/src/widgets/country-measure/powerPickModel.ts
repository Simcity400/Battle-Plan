import type { CountryStabilityReply } from '../../generated/dashboard';

/** These projections are compared only when a product reply changes. Text and
 * artwork refreshes do not change the owner of the native numeric geometry. */
export function powerMaximaModel(reply: CountryStabilityReply) {
  const series = reply.series.map(row => ({ tag: row.tag, maxCommandPower: row.maxCommandPower, carried: row.carried }));
  return { decimals: reply.decimals, series, key: JSON.stringify([reply.decimals, series]) };
}
