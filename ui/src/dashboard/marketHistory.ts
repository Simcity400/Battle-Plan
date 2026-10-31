// Shared reading of the prepared market history (`market-history`): validation and
// the dated lookups the market and lend-lease widgets share.
import type { AgreementHistory, ContractHistory, CountryIdentityHistory, HolderTag, HourSpan, MarketHistoryReply, MarketPoolItem, MarketView, OfferHistory } from '../generated/dashboard';
import type { PreparedPrefixFields } from './preparedPrefix';
import { object } from './validation.ts';
import { isEconomyRuns, isHolderMap, isVariantNames, lastRunBy, preparedAt, runAt, tagAt, validateEconomyReply, variantNameAt } from './economyHistory.ts';
import type { EconomyRun } from '../generated/dashboard';

export const MARKET_HISTORY = 'market-history';
/** Equipment, CIC and fuel shares are fixed5; lend-lease fuel quantities /32768. */
export const FIXED5 = 100000n;
export const FUEL_QUANTITY = 32768n;

const items = (value: unknown) => Array.isArray(value) && value.every(item => object(item) && Number.isSafeInteger(item.pool)
  && Number.isSafeInteger(item.ordinal) && typeof item.variant === 'string' && /^[0-9a-f]+$/.test(item.variant) && isEconomyRuns(item.amounts));

export function validateMarketHistory(view: MarketView) {
  return (value: unknown): MarketHistoryReply => validateEconomyReply<MarketHistoryReply>(value, reply => reply.view === view
    && isEconomyRuns(reply.known) && isHolderMap(reply.holders) && isVariantNames(reply.variants)
    && Array.isArray(reply.contracts) && reply.contracts.every(row => object(row) && typeof row.seller === 'string' && typeof row.id === 'string'
      && isEconomyRuns(row.buyer) && isEconomyRuns(row.speed) && items(row.items) && items(row.levels)
      && Array.isArray(row.purchase) && row.purchase.every(runs => isEconomyRuns(runs)))
    && Array.isArray(reply.offers) && reply.offers.every(row => object(row) && typeof row.seller === 'string' && items(row.inventory) && items(row.levels))
    && Array.isArray(reply.agreements) && reply.agreements.every(row => object(row) && typeof row.giver === 'string' && typeof row.id === 'string'
      && ['receiver', 'revision', 'since', 'last'].every(key => isEconomyRuns(row[key])) && items(row.items)
      && Array.isArray(row.fuel) && row.fuel.every(runs => isEconomyRuns(runs))), 'market history');
}

/** A holder's tag at `hour`. */
export const holderTagAt = (reply: Pick<MarketHistoryReply, 'holders'>, holder: string | null, hour: number): string | null =>
  holder === null ? null : tagAt(reply.holders[holder], hour);

/** A variant's in-game name at `hour`; null where content cannot name it. */
export const variantAt = (reply: Pick<MarketHistoryReply, 'variants'>, variant: string, hour: number): string | null =>
  variantNameAt(reply.variants[variant], hour);

/** Whether the view's tables were recorded complete at `hour`. */
export const knownAt = (reply: MarketHistoryReply, hour: number) => runAt(reply.known, hour) !== null;

/** A header's count of the rows shown at `hour`: none unless the view's roster is known whole
 * and prepared then, and a lower bound ("3+") while rows it cannot attribute may also match. */
export const rosterCount = (reply: MarketHistoryReply, hour: number, count: number, unattributed: number): string | null =>
  knownAt(reply, hour) && preparedAt(reply, hour) ? `${count.toLocaleString()}${unattributed ? '+' : ''}` : null;

/** The pool entries holding an amount at `hour`, in native order. */
export function poolAt(entries: readonly MarketPoolItem[], pool: number, hour: number): { variant: string; amount: bigint }[] {
  return entries.filter(item => item.pool === pool).flatMap(item => {
    const run = runAt(item.amounts, hour);
    return run ? [{ variant: item.variant, amount: BigInt(run[2]) }] : [];
  });
}

/** The hour a lifetime of runs is read at: `hour` while it holds, else its last
 * read before `hour` (an ended episode), else null (not yet begun). */
export function readHourOf(runs: readonly EconomyRun[], hour: number): { at: number; ended: boolean } | null {
  if (runAt(runs, hour)) return { at: hour, ended: false };
  const last = lastRunBy(runs, hour);
  return last ? { at: last[1], ended: true } : null;
}

export type { AgreementHistory, ContractHistory, OfferHistory };

/** A picked party: its native holder and the tags it had when picked. */
export interface Party { id: string; tags: readonly HolderTag[] }

/** The parties of a pick, from their holders (null where unidentified). */
export const partiesOf = (holders: Readonly<Record<string, readonly HolderTag[] | undefined>>, ids: readonly (string | null)[]): Party[] =>
  ids.flatMap(id => id === null ? [] : [{ id, tags: holders[id] ?? [] }]);

/** The tags a drill asks for at `hour`: each picked party's tag then, through its native
 * holder: `exact` where it knows it, else `current`, else the tags kept at the pick; null
 * when none was recorded. */
export function partyTagsAt(parties: readonly Party[], current: Readonly<Record<string, readonly HolderTag[] | undefined>> | undefined, hour: number,
  exact?: (id: string, hour: number) => string | null): string[] | null {
  const tags = parties.flatMap(party => exact?.(party.id, hour) ?? tagAt(current?.[party.id] ?? party.tags, hour) ?? []);
  return tags.length ? tags : null;
}

/** A holder's (canonical slot's) tag at `hour` in the country identity history, where that
 * history is exact: one of its runs covers the hour, and preparation has left the hour
 * neither pending nor approximate. Null otherwise. */
export function identityTagAt(identities: readonly CountryIdentityHistory[], prefix: PreparedPrefixFields | null, id: string, hour: number): string | null {
  const within = (spans: readonly HourSpan[] | undefined) => !!spans?.some(span => span.firstHour <= hour && hour <= span.lastHour);
  if (prefix && (within(prefix.pending) || within(prefix.approximate) || !prefix.coverage.complete && hour > prefix.coverage.lastHour)) return null;
  for (const country of identities) for (const run of country.identities)
    if (String(run.slot) === id && run.firstHour <= hour && hour <= run.lastHour) return country.tag;
  return null;
}
