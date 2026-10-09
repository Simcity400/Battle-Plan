import type { TradeRun } from '../../generated/dashboard';

/** All native operands contribute, including deals outside the DOM window. */
export function dealAccounting(deals: readonly TradeRun[]) {
  return deals.reduce((sum, deal) => {
    const raw = BigInt(deal.deliveredRaw), magnitude = raw < 0n ? -raw : raw;
    return { total: sum.total + raw, widest: magnitude > sum.widest ? magnitude : sum.widest };
  }, { total: 0n, widest: 0n });
}

/** Only rendered rows need country names/flags; scrolling can reach every deal. */
export function dealWindow(deals: readonly TradeRun[], range: { start: number; end: number }, hour: number) {
  const shown = deals.slice(range.start, range.end);
  return { shown, wanted: shown.map(deal => ({ tag: deal.partner, hour })) };
}
