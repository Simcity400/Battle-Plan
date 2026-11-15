// The request as the widget builds it, and the market family's `market-lendlease` reply as the market view reads it
// (the contract's types, and the reply with its hour).

export interface TradeFlowParams { branchId: string; tags: string[] | null; from?: number | null; to?: number | null }

export type { MarketAt } from '../../dashboard/market';
export type { MarketGrant, MarketItem, MarketLendLeaseReply, MarketLevel, MarketOffer, MarketTerms, LeaseTerms } from '../../generated/dashboard';
