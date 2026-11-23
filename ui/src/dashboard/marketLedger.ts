import { countryFromKey } from './countryKey.ts';
import type { LogRow } from '../charts/log/logData';
import type { LogReplyRow } from '../charts/log/logSource';
import type { FacetFilter, LogCell } from '../generated/dashboard';
import { FUEL_QUANTITY, fixedText } from './market.ts';

export const MARKET_LOG = 'market-events';
export const MARKET_LEDGER = 'market-ledger';
export const MARKET_LEDGER_TAIL = 'market-ledger-tail';

export const KINDS = [
  { kind: 11, key: 'contracts', label: 'New contracts' },
  { kind: 10, key: 'requests', label: 'Requests posted' },
  { kind: 20, key: 'deliveries', label: 'Deliveries' },
  { kind: 21, key: 'payments', label: 'Payments' },
  { kind: 12, key: 'removals', label: 'Contracts removed' },
] as const;
export type KindKey = typeof KINDS[number]['key'];

export const DELIVERY_KINDS = [3, 4, 5, 6, 20, 21, 34] as const;

export interface LedgerItem {
  id: string;
  kind: number | null;
  mode: number | null;
  contract: string | null;
  seller: string | null;
  buyer: string | null;
  variant: string | null;
  equipment: string | null;
  equipmentMissing: string | null;
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
    id: text(cells.id) ?? row.key, kind: number(cells.kind), mode: number(cells.mode), contract: text(cells.contract),
    seller: text(cells.seller), buyer: text(cells.buyer), variant: text(cells.variant), equipment: text(cells.equipment),
    equipmentMissing: text(cells.equipmentMissing), amountRaw: raw(cells.amountRaw), auxiliaryRaw: raw(cells.auxiliaryRaw),
    totalRaw: raw(cells.totalRaw), reason: text(cells.reason),
  } };
}

export function ledgerFacets(kinds: readonly number[], tags: readonly string[] | null): FacetFilter[] {
  const facets: FacetFilter[] = [{ name: 'kind', values: [...kinds], countries: [] }];
  if (tags) facets.push({ name: 'country|receiver', values: [], countries: tags.map(countryFromKey) });
  return facets;
}

export function shownKinds(enabled: (key: KindKey) => boolean): number[] {
  const kinds: number[] = KINDS.filter(kind => enabled(kind.key)).map(kind => kind.kind);
  if (enabled('deliveries')) kinds.push(34);
  return kinds.sort((a, b) => a - b);
}

const lease = (item: LedgerItem) => item.kind !== null && (item.kind <= 6 || (item.kind === 34 && item.mode !== 20));

export function ledgerTitle(item: LedgerItem, name: (tag: string | null) => string): string {
  switch (item.kind) {
    case null: return 'Event kind not recorded';
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

export function ledgerFigure(item: LedgerItem): { text: string; negative: boolean } | null {
  if (item.kind === null) return null;
  if (item.kind === 34 || (item.amountRaw === null && item.reason !== null)) return { text: 'Amount unavailable', negative: false };
  if (item.amountRaw === null) return null;
  const amount = BigInt(item.amountRaw);
  if (item.kind === 3 || item.kind === 4 || item.kind === 20) return { text: fixedText(amount), negative: amount < 0n };
  if (item.kind === 5) return { text: `${fixedText(amount, FUEL_QUANTITY)} fuel${item.auxiliaryRaw && item.auxiliaryRaw !== '0' ? ` · ${fixedText(BigInt(item.auxiliaryRaw), FUEL_QUANTITY)} lost` : ''}`, negative: amount < 0n };
  if (item.kind === 21) return { text: `${fixedText(amount)} CIC`, negative: amount < 0n };
  return null;
}

export const ledgerIcon = (kind: number | null) => kind === null ? 'history' : kind === 21 ? 'chart-bars' : kind === 3 || kind === 4 || kind === 5 || kind === 20 || kind === 34 ? 'anchor'
  : kind === 6 || kind === 12 ? 'close' : kind === 10 ? 'search' : 'handshake';

export const ledgerSelection = (item: LedgerItem) => item.contract === null ? null
  : lease(item) ? `agreement:${item.contract}` : `contract:${item.kind === 10}:${item.contract}`;

export const LEDGER_EMPTY = 'No market activity matches these settings.';
