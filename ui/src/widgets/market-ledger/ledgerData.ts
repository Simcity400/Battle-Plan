import type { CountryGroupReply, HolderTag, LedgerMember, MarketLedgerReply, MarketLedgerRow } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import { countryMatchFilter, groupMembers, groupRuns, groupUnobserved, membersKnown } from '../../dashboard/groups.ts';
import type { MembershipKnown } from '../../dashboard/groups';
import { withoutHours } from '../../charts/timeline/runState.ts';
import type { LogPage, LogRow } from '../../charts/log/logData';
import { object } from '../../dashboard/validation.ts';
import { partiesOf } from '../../dashboard/marketHistory.ts';
import type { Party } from '../../dashboard/marketHistory.ts';
import { fixedText, isEconomyRuns, isHolderMap, isVariantNames, preparedAt, tagAt, validateEconomyReply, variantNameAt } from '../../dashboard/economyHistory.ts';

export const MARKET_LEDGER = 'market-ledger';
export const PAGE_SIZE = 256;

/** The ledger's occurrence kinds, as the game's market records them. */
export const KINDS = [
  { kind: 11, key: 'contracts', label: 'New contracts' },
  { kind: 10, key: 'requests', label: 'Requests posted' },
  { kind: 20, key: 'deliveries', label: 'Deliveries' },
  { kind: 21, key: 'payments', label: 'Payments' },
  { kind: 12, key: 'removals', label: 'Contracts removed' },
] as const;
export type KindKey = typeof KINDS[number]['key'];

const text = (value: unknown) => value === null || typeof value === 'string';
export function validateLedger(value: unknown): MarketLedgerReply {
  return validateEconomyReply<MarketLedgerReply>(value, reply => (reply.total === null || typeof reply.total === 'string' && /^\d+$/.test(reply.total))
    && text(reply.next) && isEconomyRuns(reply.known) && isHolderMap(reply.holders) && isVariantNames(reply.variants)
    && Array.isArray(reply.rows) && reply.rows.every(row => object(row) && typeof row.id === 'string' && Number.isSafeInteger(row.hour)
      && Number.isSafeInteger(row.kind) && typeof row.contract === 'string' && text(row.seller) && text(row.buyer)
      && typeof row.variant === 'string' && text(row.amountRaw) && typeof row.totalRaw === 'string' && text(row.reason)), 'market ledger');
}

/** The countries a ledger asks for: chosen tags at every hour, or a group's
 * members over their own dated memberships; both empty for every country. */
export interface LedgerCountries {
  tags: string[];
  members: LedgerMember[];
  /** Hours the group's membership was not recorded: their occurrences are not listed. */
  unobserved: boolean;
}

/** The widget's country choice as the ledger asks for it. A group resolves to its
 * members' dated memberships (aliases included, unrecorded hours left out) and is
 * never asked as every country: null while the group history loads, a reason when
 * it names nobody. `known` says whether the members are known at the hours asked
 * (`MembershipKnown`): no reason holds, and no listed occurrence is all there is,
 * until the membership it is about has been read. */
export function ledgerCountries(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  firstHour: number, lastHour: number): { countries: LedgerCountries | null; reason: string | null; known: MembershipKnown } {
  const always = () => true;
  if (selection.kind === 'all') return { countries: { tags: [], members: [], unobserved: false }, reason: null, known: always };
  if (selection.kind !== 'group') {
    const tags = [...selection.kind === 'tags' ? selection.tags : fallback].sort();
    return tags.length ? { countries: { tags, members: [], unobserved: false }, reason: null, known: always }
      : { countries: null, reason: 'Choose a country in the widget settings.', known: always };
  }
  if (selection.group.type === 'top') return { countries: null, reason: 'This list doesn’t rank countries. Choose countries or another group in the widget settings.', known: always };
  const known = membersKnown(selection, groups, lastHour);
  if (!groups) return { countries: null, reason: null, known };
  const runs = groupRuns(groups, selection.group);
  // A group the prepared prefix does not name yet may still be founded later.
  if (!runs) return { countries: null, reason: known(firstHour) ? 'This group is unavailable in this campaign.' : null, known };
  if (!Array.isArray(groups.countryIdentities)) return { countries: null, reason: 'Country identities are unavailable in this campaign.', known };
  const filter = countryMatchFilter(selection, fallback, groups, lastHour, undefined, firstHour);
  const sorted = [...filter.membership ?? []].sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  const unobserved = [...filter.unobserved].sort((a, b) => a.firstHour - b.firstHour);
  const recorded = unobserved.length ? withoutHours(sorted, unobserved) : sorted;
  const members = recorded.map(run => ({ tag: run.tag, firstHour: run.firstHour, lastHour: run.lastHour }));
  // No member yet while membership is still being prepared is loading, not proof of none.
  if (!members.length && !known(firstHour, lastHour)) return { countries: null, reason: null, known };
  if (!members.length) {
    // Proven empty only where the roster was recorded where it counts: every hour for
    // a dated or ever-member group, the branch's end for an end-member one.
    const roster = groupUnobserved(groups, selection.group);
    const counted = groupMembers(runs, selection.membership === 'end' ? 'end' : 'ever', lastHour, lastHour, Infinity, roster);
    const unknown = counted === null || (selection.membership !== 'end' && roster.length > 0) || sorted.length > 0;
    return { countries: null, reason: unknown ? 'This group’s membership was not recorded.'
      : selection.membership === 'end' ? 'This group has no members at the end of this campaign.' : 'This group has no members in this campaign.', known };
  }
  return { countries: { tags: [], members, unobserved: unobserved.length > 0 }, reason: null, known };
}

/** One occurrence with its parties' tags and equipment name at its hour. */
export interface LedgerItem {
  row: MarketLedgerRow;
  seller: string | null;
  buyer: string | null;
  /** Seller and buyer by native holder, so a drill follows their later tags. */
  parties: Party[];
  equipment: string | null;
}

export function ledgerRows(reply: MarketLedgerReply, page: number): LogRow<LedgerItem>[] {
  return reply.rows.map(row => ({ key: `${page}:${row.id}`, id: row.id, hour: row.hour, item: {
    row, seller: row.seller === null ? null : tagAt(reply.holders[row.seller], row.hour),
    buyer: row.buyer === null ? null : tagAt(reply.holders[row.buyer], row.hour), parties: partiesOf(reply.holders, [row.seller, row.buyer]),
    equipment: row.variant === '0' ? null : variantNameAt(reply.variants[row.variant], row.hour) } }));
}

/** The cursor of each page a ledger can ask for: page 0 has
 * none, and page k+1 opens with page k's `next`, so the list ends at the
 * first page not yet loaded or at the oldest page. `next` holds the cursors
 * of pages 2 onwards found so far. */
export function ledgerCursors(first: MarketLedgerReply, next: readonly string[]): (string | undefined)[] {
  if (first.next === null) return [undefined];
  return [undefined, first.next, ...next];
}

/** The log's page for each cursor: its loaded rows, or a window to wait for. A page
 * not loaded ends where the loaded page before it starts, so it never sorts above it. */
export function ledgerPages(first: MarketLedgerReply, length: number, page: (index: number) => MarketLedgerReply | undefined,
  errors: ReadonlyMap<number, string>, hour: number | null, branchStart: number): LogPage<LedgerItem>[] {
  const total = first.total == null ? null : Number(first.total);
  let bound = hour ?? branchStart;
  return Array.from({ length }, (_, index) => {
    const reply = index === 0 ? first : page(index);
    const rows = reply ? ledgerRows(reply, index) : undefined;
    const count = rows?.length ?? (total === null ? PAGE_SIZE : Math.max(1, Math.min(PAGE_SIZE, total - index * PAGE_SIZE)));
    const start = rows?.length ? rows.at(-1)!.hour : branchStart;
    const end = rows?.length ? rows[0].hour : bound;
    if (rows?.length) bound = start;
    return { key: String(index), start, end, count, rows, error: errors.get(index) };
  });
}

/** `next` grown by the loaded pages' own cursors; the same array when no
 * loaded page adds one. */
export function extendCursors(first: MarketLedgerReply, next: readonly string[],
  page: (index: number) => MarketLedgerReply | undefined): readonly string[] {
  if (first.next === null) return next;
  let grown: string[] | null = null;
  for (let index = 1 + next.length; ; index++) {
    const reply = page(index);
    if (!reply || reply.next === null) break;
    (grown ??= [...next]).push(reply.next);
  }
  return grown ?? next;
}

/** Why an empty ledger is empty through `hour`: still being prepared, no
 * occurrences in hours the market was read at all of them, in only some, or
 * the market's occurrences never read in the branch through that hour. */
export type LedgerEmpty = 'pending' | 'none' | 'partial' | 'unrecorded';

export function ledgerEmpty(reply: MarketLedgerReply, firstHour: number, hour: number): LedgerEmpty {
  if (!preparedAt(reply, hour)) return 'pending';
  let covered = 0, next = firstHour;
  for (const [first, last] of reply.known) {
    const from = Math.max(first, next), to = Math.min(last, hour);
    if (from > to) continue;
    covered += to - from + 1;
    next = to + 1;
  }
  return covered === 0 ? 'unrecorded' : covered === hour - firstHour + 1 ? 'none' : 'partial';
}

export const LEDGER_EMPTY: Record<LedgerEmpty, string> = {
  pending: 'Market activity for this time is still being prepared.',
  none: 'No market activity matches these settings.',
  partial: 'No market activity matches these settings in the hours the market was recorded. Some hours weren’t recorded.',
  unrecorded: 'Market activity wasn’t recorded up to this time.',
};

/** The kinds a widget's toggles leave on; the unavailable-amount occurrences follow deliveries. */
export function shownKinds(enabled: (key: KindKey) => boolean): number[] {
  const kinds = KINDS.filter(kind => enabled(kind.key)).map(kind => kind.kind) as number[];
  if (enabled('deliveries')) kinds.push(34);
  return kinds.sort((a, b) => a - b);
}

const units = (raw: string) => fixedText(BigInt(raw), 100000n);

/** The row's title, with the counterpart named from the seller's side. */
export function ledgerTitle(item: LedgerItem, name: (tag: string | null) => string): string {
  switch (item.row.kind) {
    case 10: return `Request posted by ${name(item.buyer)}`;
    case 11: return `New contract with ${name(item.buyer)}`;
    case 12: return 'Contract removed';
    case 20: return `Delivery to ${name(item.buyer)}`;
    case 21: return `Payment collected from ${name(item.buyer)}`;
    default: return 'Delivery amount unavailable';
  }
}

/** The right-aligned figure: units, CIC, or nothing for a removal. */
export function ledgerFigure(item: LedgerItem): { text: string; negative: boolean } | null {
  const { row } = item;
  if (row.kind === 34 || row.amountRaw === null) return { text: 'Amount unavailable', negative: false };
  if (row.kind === 20) return { text: units(row.amountRaw), negative: BigInt(row.amountRaw) < 0n };
  if (row.kind === 21) return { text: `${units(row.amountRaw)} CIC`, negative: BigInt(row.amountRaw) < 0n };
  return null;
}

export const ledgerIcon = (kind: number) => kind === 21 ? 'chart-bars' : kind === 20 || kind === 34 ? 'anchor' : kind === 12 ? 'close' : kind === 10 ? 'search' : 'handshake';

/** Each holder's tags as the loaded pages last read them, so a picked party's later tags,
 * found once preparation reaches them, are followed. */
export function loadedHolders(pages: readonly LogPage<LedgerItem>[]): Record<string, readonly HolderTag[]> {
  const holders: Record<string, readonly HolderTag[]> = {};
  for (const page of pages) for (const row of page.rows ?? []) for (const party of row.item.parties) holders[party.id] ??= party.tags;
  return holders;
}
