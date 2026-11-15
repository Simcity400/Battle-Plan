// The market's occurrences as the market ledger and the market drill's deliveries read them: rows of `market-ledger`
// (pages of the declared log `market-events`, parties and equipment named at each row's hour).
import type { LogRow } from '../charts/log/logData';
import type { LogReplyRow } from '../charts/log/logSource';
import type { FacetFilter, LogCell } from '../generated/dashboard';
import { FUEL_QUANTITY, fixedText } from './market.ts';

export const MARKET_LOG = 'market-events';
export const MARKET_LEDGER = 'market-ledger';

/** The ledger's toggles, as the game's market records each occurrence. */
export const KINDS = [
  { kind: 11, key: 'contracts', label: 'New contracts' },
  { kind: 10, key: 'requests', label: 'Requests posted' },
  { kind: 20, key: 'deliveries', label: 'Deliveries' },
  { kind: 21, key: 'payments', label: 'Payments' },
  { kind: 12, key: 'removals', label: 'Contracts removed' },
] as const;
export type KindKey = typeof KINDS[number]['key'];

/** A drill's delivery changes: lend-lease delivered, lost, fuel and ended, purchase deliveries and payments, and the
 * deliveries whose amount the recording names unavailable. */
export const DELIVERY_KINDS = [3, 4, 5, 6, 20, 21, 34] as const;

/** One occurrence with its parties' tags and its equipment named at its hour. */
export interface LedgerItem {
  id: string;
  kind: number;
  mode: number | null;
  /** The contract's or agreement's handle, hexadecimal. */
  contract: string;
  seller: string | null;
  buyer: string | null;
  variant: string;
  equipment: string | null;
  equipmentMissing: string | null;
  /** Null where the recording names the amount unavailable (`reason`). */
  amountRaw: string | null;
  auxiliaryRaw: string | null;
  totalRaw: string | null;
  reason: string | null;
}

const number = (cell: LogCell | undefined) => cell?.kind === 'number' ? Number(cell.value) : null;
const text = (cell: LogCell | undefined) => cell?.kind === 'text' ? cell.value : null;
const raw = (cell: LogCell | undefined) => cell?.kind === 'number' ? cell.value : null;

export function ledgerEntry(row: LogReplyRow): LogRow<LedgerItem> {
  const cells = row.cells;
  return { key: row.key, hour: row.hour, item: {
    id: text(cells.id) ?? row.key, kind: number(cells.kind) ?? 0, mode: number(cells.mode), contract: text(cells.contract) ?? '0',
    seller: text(cells.seller), buyer: text(cells.buyer), variant: text(cells.variant) ?? '0', equipment: text(cells.equipment),
    equipmentMissing: text(cells.equipmentMissing), amountRaw: raw(cells.amountRaw), auxiliaryRaw: raw(cells.auxiliaryRaw),
    totalRaw: raw(cells.totalRaw), reason: text(cells.reason),
  } };
}

/** The facets a page asks: the kinds shown, and the chosen countries as either side; every country with `tags` null. */
export function ledgerFacets(kinds: readonly number[], tags: readonly string[] | null): FacetFilter[] {
  const facets: FacetFilter[] = [{ name: 'kind', values: [...kinds], tags: [] }];
  if (tags) facets.push({ name: 'country|receiver', values: [], tags: [...tags] });
  return facets;
}

/** The kinds a ledger's toggles leave on; the unavailable-amount occurrences follow deliveries. */
export function shownKinds(enabled: (key: KindKey) => boolean): number[] {
  const kinds: number[] = KINDS.filter(kind => enabled(kind.key)).map(kind => kind.kind);
  if (enabled('deliveries')) kinds.push(34);
  return kinds.sort((a, b) => a - b);
}

/** Whether a row is a lend-lease occurrence (an agreement's), not a market contract's. */
const lease = (item: LedgerItem) => item.kind <= 6 || item.kind === 34 && item.mode !== 20;

/** The row's title, with the counterpart named from the seller's side. */
export function ledgerTitle(item: LedgerItem, name: (tag: string | null) => string): string {
  switch (item.kind) {
    case 3: return `Lend-lease delivered to ${name(item.buyer)}`;
    case 4: return `Lend-lease lost on the way to ${name(item.buyer)}`;
    case 5: return `Fuel lent to ${name(item.buyer)}`;
    case 6: return `Lend-lease to ${name(item.buyer)} ended`;
    case 10: return `Request posted by ${name(item.buyer)}`;
    case 11: return `New contract with ${name(item.buyer)}`;
    case 12: return 'Contract removed';
    case 20: return `Delivery to ${name(item.buyer)}`;
    case 21: return `Payment collected from ${name(item.buyer)}`;
    default: return lease(item) ? 'Lend-lease delivery amount unavailable' : 'Delivery amount unavailable';
  }
}

/** The right-aligned figure: units, CIC or fuel, the refusal of an unavailable amount, or nothing. */
export function ledgerFigure(item: LedgerItem): { text: string; negative: boolean } | null {
  if (item.kind === 34 || (item.amountRaw === null && item.reason !== null)) return { text: 'Amount unavailable', negative: false };
  if (item.amountRaw === null) return null;
  const amount = BigInt(item.amountRaw);
  if (item.kind === 3 || item.kind === 4 || item.kind === 20) return { text: fixedText(amount), negative: amount < 0n };
  if (item.kind === 5) return { text: `${fixedText(amount, FUEL_QUANTITY)} fuel${item.auxiliaryRaw && item.auxiliaryRaw !== '0' ? ` · ${fixedText(BigInt(item.auxiliaryRaw), FUEL_QUANTITY)} lost` : ''}`, negative: amount < 0n };
  if (item.kind === 21) return { text: `${fixedText(amount)} CIC`, negative: amount < 0n };
  return null;
}

export const ledgerIcon = (kind: number) => kind === 21 ? 'chart-bars' : kind === 3 || kind === 4 || kind === 5 || kind === 20 || kind === 34 ? 'anchor'
  : kind === 6 || kind === 12 ? 'close' : kind === 10 ? 'search' : 'handshake';

/** The drill a row opens: the contract (or request) or agreement it is of, as Trade flows' market view keys it. */
export const ledgerSelection = (item: LedgerItem) => lease(item) ? `agreement:${item.contract}` : `contract:${item.kind === 10}:${item.contract}`;

/** Why an empty list is empty: still loading, or nothing matches. */
export const LEDGER_EMPTY = 'No market activity matches these settings.';
