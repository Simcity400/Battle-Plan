// Shared reading of the prepared market history (`market-history`): validation and
// the dated lookups the market and lend-lease widgets share.
import type { AgreementHistory, ContractHistory, MarketHistoryReply, MarketPoolItem, MarketView, OfferHistory } from '../generated/dashboard';
import { object } from './validation.ts';
import { isEconomyRuns, isHolderMap, isVariantNames, lastRunBy, runAt, tagAt, validateEconomyReply, variantNameAt } from './economyHistory.ts';
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
