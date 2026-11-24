import { countryFromKey } from './countryKey.ts';
import type { LogRow } from '../charts/log/logData';
import type { BoundFacetFilter, LogReplyRow } from '../charts/log/logSource';
import type { LogCell } from '../generated/dashboard';
import { FUEL_QUANTITY, fixedText } from './market.ts';

export const MARKET_LOG = 'market-events';
export const MARKET_LEDGER = 'market-ledger';

export const KINDS = [
  { kind: 11, key: 'contracts', label: 'New contracts' },
  { kind: 10, key: 'requests', label: 'Requests posted' },
  { kind: 20, key: 'deliveries', label: 'Deliveries' },
  { kind: 21, key: 'payments', label: 'Payments' },
  { kind: 12, key: 'removals', label: 'Contracts removed' },
] as const;
export type KindKey = typeof KINDS[number]['key'];

export const DELIVERY_KINDS = [3, 4, 5, 6, 20, 21, 34] as const;

export type PartyState = 'tag' | 'notRecorded' | 'unavailable';
export type AmountState = 'number' | 'notRecorded' | 'missing';

export interface LedgerItem {
  id: string;
  kind: number | null;
  mode: number | null;
  contract: string | null;
  seller: string | null;
  sellerState: PartyState;
  buyer: string | null;
  buyerState: PartyState;
  variant: string | null;
  equipment: string | null;
  equipmentMissing: string | null;
  amountRaw: string | null;
  amountState: AmountState;
  auxiliaryRaw: string | null;
  auxiliaryState: AmountState;
  totalRaw: string | null;
  reason: string | null;
}

const number = (cell: LogCell | undefined) => cell?.kind === 'number' ? Number(cell.value) : null;
const text = (cell: LogCell | undefined) => cell?.kind === 'text' ? cell.value : null;
const raw = (cell: LogCell | undefined) => cell?.kind === 'number' ? cell.value : null;
const partyState = (cell: LogCell | undefined): PartyState =>
  cell?.kind === 'text' ? 'tag' : cell?.kind === 'notRecorded' ? 'notRecorded' : 'unavailable';
const amountState = (cell: LogCell | undefined): AmountState =>
  cell?.kind === 'number' ? 'number' : cell?.kind === 'notRecorded' ? 'notRecorded' : 'missing';

export function ledgerPartyName(tag: string | null, state: PartyState, resolve: (tag: string) => string): string {
  if (tag !== null) return resolve(tag);
  return state === 'notRecorded' ? 'Country not recorded' : 'Country name unavailable';
}

export function ledgerEntry(row: LogReplyRow): LogRow<LedgerItem> {
  const cells = row.cells;
  return { key: row.key, hour: row.hour, item: {
    id: text(cells.id) ?? row.key, kind: number(cells.kind), mode: number(cells.mode), contract: text(cells.contract),
    seller: text(cells.seller), sellerState: partyState(cells.seller),
    buyer: text(cells.buyer), buyerState: partyState(cells.buyer),
    variant: text(cells.variant), equipment: text(cells.equipment),
    equipmentMissing: text(cells.equipmentMissing),
    amountRaw: raw(cells.amountRaw), amountState: amountState(cells.amountRaw),
    auxiliaryRaw: raw(cells.auxiliaryRaw), auxiliaryState: amountState(cells.auxiliaryRaw),
    totalRaw: raw(cells.totalRaw), reason: text(cells.reason),
  } };
}

export function ledgerFacets(kinds: readonly number[], tags: readonly string[] | null): BoundFacetFilter[] {
  const facets: BoundFacetFilter[] = [{ name: 'kind', values: [...kinds], countries: [] }];
  if (tags) facets.push({ name: 'country|receiver', values: [], countries: tags.map(countryFromKey) });
  return facets;
}

export function shownKinds(enabled: (key: KindKey) => boolean): number[] {
  const kinds: number[] = KINDS.filter(kind => enabled(kind.key)).map(kind => kind.kind);
  if (enabled('deliveries')) kinds.push(34);
  return kinds.sort((a, b) => a - b);
}

const lease = (item: LedgerItem) => item.kind !== null && (item.kind <= 6 || (item.kind === 34 && item.mode !== null && item.mode !== 20));

export function ledgerTitle(item: LedgerItem, name: (tag: string | null, state?: PartyState) => string): string {
  const buyer = () => name(item.buyer, item.buyerState);
  switch (item.kind) {
    case null: return 'Market event kind not recorded';
    case 3: return `Lend-lease delivered to ${buyer()}`;
    case 4: return `Lend-lease lost on the way to ${buyer()}`;
    case 5: return `Fuel lent to ${buyer()}`;
    case 6: return `Lend-lease to ${buyer()} ended`;
    case 10: return `Request posted by ${buyer()}`;
    case 11: return `New contract with ${buyer()}`;
    case 12: return 'Contract removed';
    case 20: return `Delivery to ${buyer()}`;
    case 21: return `Payment collected from ${buyer()}`;
    default: return lease(item) ? 'Lend-lease delivery amount unavailable' : 'Delivery amount unavailable';
  }
}

export function ledgerFigure(item: LedgerItem): { text: string; negative: boolean } | null {
  if (item.kind === 34 || (item.amountRaw === null && item.reason !== null)) return { text: 'Amount unavailable', negative: false };
  if (item.kind === null) return null;
  if (item.amountRaw === null) {
    if (item.kind === 3 || item.kind === 4 || item.kind === 5 || item.kind === 20 || item.kind === 21) {
      return { text: item.amountState === 'notRecorded' ? 'Amount not recorded' : 'Amount unavailable', negative: false };
    }
    return null;
  }
  const amount = BigInt(item.amountRaw);
  if (item.kind === 3 || item.kind === 4 || item.kind === 20) return { text: fixedText(amount), negative: amount < 0n };
  if (item.kind === 5) {
    const lost = item.auxiliaryRaw === null
      ? item.auxiliaryState === 'notRecorded' ? ' · lost not recorded' : ''
      : item.auxiliaryRaw !== '0' ? ` · ${fixedText(BigInt(item.auxiliaryRaw), FUEL_QUANTITY)} lost` : '';
    return { text: `${fixedText(amount, FUEL_QUANTITY)} fuel${lost}`, negative: amount < 0n };
  }
  if (item.kind === 21) return { text: `${fixedText(amount)} CIC`, negative: amount < 0n };
  return null;
}

export const ledgerIcon = (kind: number | null) => kind === 21 ? 'chart-bars' : kind === 3 || kind === 4 || kind === 5 || kind === 20 || kind === 34 ? 'anchor'
  : kind === 6 || kind === 12 ? 'close' : kind === 10 ? 'search' : 'handshake';

export const ledgerSelection = (item: LedgerItem): string | null =>
  item.kind === null || item.contract === null || (item.kind === 34 && item.mode === null) ? null
    : lease(item) ? `agreement:${item.contract}` : `contract:${item.kind === 10}:${item.contract}`;

