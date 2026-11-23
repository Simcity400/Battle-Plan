
export interface TradeFlowParams { branchId: string; tags: string[] | null; from?: number | null; to?: number | null }

export type { MarketAt } from '../../dashboard/market';
export type { MarketGrant, MarketItem, MarketLendLeaseReply, MarketLevel, MarketOffer, MarketTerms, LeaseAmount, LeaseHour, LeaseItems, LeaseTerms } from '../../generated/dashboard';
