import type { HourSpan, PowerSpendingReply, SpendCategory, SpendCoverage, SpendIdeaTooltips, SpendItem } from '../generated/dashboard';
import { validateIdeaTooltipData } from './ideaHistoryReply.ts';
import type { BoundTrack, WidgetSpec } from './model';
import { resolveWindow } from './model.ts';
import type { SunburstItem } from '../charts/sunburst/layout';
import { dayLabel } from '../charts/draw.ts';
import { drawableIcon } from './artwork.ts';
import { gameDate, monthAt, monthIndex, monthNames } from './calendar.ts';
import { MAX_NAME_HOURS, nameKey } from './countryFacts.ts';
import type { NameAt } from './countryFacts';
import { contentSet, setToken, storableToken, UNSTORABLE } from './options.ts';
import { NO_SPENDING_CATEGORIES } from './powerSpendingOptions.ts';

// The currency a `power-spending` reply is asked for.
export type PowerCurrency = 'Political' | 'Command';

const RAW = /^-?\d{1,39}$/;
const raw = (value: unknown): value is string => typeof value === 'string' && RAW.test(value);
const optionalRaw = (value: unknown) => value === null || raw(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const count = (value: unknown) => integer(value) && value >= 0;
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const list = (value: unknown, each: (entry: unknown) => boolean) => Array.isArray(value) && value.every(each);

/** A `power-spending` reply, refused whole when any part is malformed; drawable art is the shared kit's check. */
export function validatePowerSpending(data: unknown): PowerSpendingReply {
  const reply = data as PowerSpendingReply;
  const items = object(reply) && Array.isArray(reply.items) ? reply.items.length : 0;
  const index = (value: unknown) => integer(value) && value >= 0 && value < items;
  const ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && (reply.currency === 'Political' || reply.currency === 'Command') && integer(reply.from) && integer(reply.to) && reply.from <= reply.to
    && reply.decimals === 5 && list(reply.warnings, entry => typeof entry === 'string')
    && list(reply.items, item => object(item) && typeof item.id === 'string' && !!item.id && typeof item.groupId === 'string'
      && typeof item.cause === 'string' && typeof item.category === 'string' && typeof item.recurring === 'boolean'
      && integer(item.firstHour) && integer(item.firstRead) && text(item.name) && typeof item.action === 'string' && text(item.icon)
      && (item.character === null || integer(item.character)) && text(item.targetTag) && text(item.targetName) && text(item.unknown)
      && text(item.tooltipToken))
    && list(reply.categories, category => object(category) && typeof category.id === 'string' && typeof category.name === 'string'
      && typeof category.identified === 'boolean')
    && list(reply.payments, entry => object(entry) && index(entry.item) && (entry.hour === null || integer(entry.hour)) && integer(entry.read)
      && (entry.sequence === null || (typeof entry.sequence === 'string' && /^\d{1,20}$/.test(entry.sequence)))
      && (entry.ordinal === null || count(entry.ordinal)) && optionalRaw(entry.knownRaw) && count(entry.payments) && count(entry.unknown)
      && typeof entry.uncovered === 'boolean')
    && list(reply.days, entry => object(entry) && index(entry.item) && (entry.day === null || integer(entry.day)) && integer(entry.read)
      && count(entry.days) && optionalRaw(entry.chargeRaw) && count(entry.payments) && count(entry.unknown) && count(entry.uncovered)
      && optionalRaw(entry.realizedRaw) && count(entry.credits))
    && list(reply.gains, entry => object(entry) && integer(entry.read) && optionalRaw(entry.gainedRaw) && optionalRaw(entry.cappedRaw)
      && count(entry.days) && count(entry.unknownDays) && count(entry.unmatched))
    && list(reply.coverage, entry => object(entry) && integer(entry.firstHour) && integer(entry.lastHour) && entry.firstHour <= entry.lastHour
      && typeof entry.covered === 'boolean' && text(entry.reason))
    && list(reply.icons, icon => object(icon) && typeof icon.key === 'string' && typeof icon.image === 'string');
  if (!ok) throw new Error('The power spending reply is malformed.');
  if (reply.ideaTooltips !== null) validateIdeaTooltipData(reply.ideaTooltips);
  const tooltips = new Set(reply.ideaTooltips?.ideas.map(info => info.token));
  if (reply.items.some(item => item.tooltipToken !== null && !tooltips.has(item.tooltipToken))) throw new Error('The power spending reply has an unlisted idea tooltip.');
  return reply;
}

/** One listed payment of a one-off item: the payments of its read hour. */
export interface PaymentEntry {
  tag: string; item: SpendItem; category: SpendCategory;
  /** The latest payment's own hour, else its read. */
  hour: number; read: number; sequence: bigint; ordinal: number;
  /** What the payments with a recorded charge charged; null when that sum was not recorded. */
  known: bigint | null; payments: number; unknown: number; uncovered: boolean;
}
/** One listed day count of a recurring item. */
export interface DayEntry {
  tag: string; item: SpendItem; category: SpendCategory;
  /** The day's midnight, else its read. */
  hour: number; read: number; days: number; charge: bigint | null; payments: number; unknown: number; uncovered: number;
  realized: bigint | null; credits: number;
}
export interface GainEntry { tag: string; read: number; gained: bigint | null; capped: bigint | null; days: number; unknownDays: number; unmatched: number }
export interface CoverageRun extends SpendCoverage { tag: string }

/** Every chunk of one country's ledger in one currency, joined: each list in read order. */
export interface PowerLedger {
  tag: string;
  currency: PowerCurrency;
  payments: readonly PaymentEntry[];
  days: readonly DayEntry[];
  gains: readonly GainEntry[];
  coverage: readonly CoverageRun[];
  icons: ReadonlyMap<string, string>;
  categories: ReadonlyMap<string, SpendCategory>;
  items: ReadonlyMap<string, SpendItem>;
  /** Every chunk's idea tooltips, each idea once; null when none names one. */
  ideaTooltips: SpendIdeaTooltips | null;
  warnings: readonly string[];
}

const big = (value: string | null) => value === null ? null : BigInt(value);
/** Category ids of costs the game charges every day, folded into one row a month per category; another recurring
 * item's days (a relation's upkeep) fold into one row a month per item. */
export const RECURRING_CATEGORIES: ReadonlySet<string> = new Set(['dailyCosts', 'organisationUpkeep', 'upkeep', 'dailyDeficit']);
const UNIDENTIFIED: SpendCategory = { id: 'notIdentified', name: 'Not identified', identified: false };

/** One country's chunks as one ledger. A chunk lists only what was read in its span, so nothing is counted twice. */
export function ledgerOf(tag: string, currency: PowerCurrency, chunks: readonly PowerSpendingReply[]): PowerLedger {
  const items = new Map<string, SpendItem>();
  const categories = new Map<string, SpendCategory>();
  const icons = new Map<string, string>();
  const payments: PaymentEntry[] = [], days: DayEntry[] = [], gains: GainEntry[] = [], coverage: CoverageRun[] = [];
  const warnings = new Set<string>();
  for (const chunk of [...chunks].sort((a, b) => a.from - b.from)) {
    for (const category of chunk.categories) categories.set(category.id, category);
    for (const icon of chunk.icons) { const drawn = drawableIcon(icon.image); if (drawn) icons.set(icon.key, drawn); }
    const own = chunk.items.map(item => { const held = items.get(item.id) ?? item; items.set(item.id, held); return held; });
    const categoryOf = (item: SpendItem) => categories.get(item.category) ?? UNIDENTIFIED;
    for (const entry of chunk.payments) {
      const item = own[entry.item];
      // A daily cost other than the daily gain's (a relation's upkeep) is counted one day at a time: a day of it.
      if (item.recurring) {
        days.push({ tag, item, category: categoryOf(item), hour: entry.hour ?? entry.read, read: entry.read, days: 1, charge: big(entry.knownRaw),
          payments: entry.payments, unknown: entry.unknown, uncovered: entry.uncovered ? 1 : 0, realized: null, credits: 0 });
        continue;
      }
      payments.push({ tag, item, category: categoryOf(item), hour: entry.hour ?? entry.read, read: entry.read,
        sequence: BigInt(entry.sequence ?? '0'), ordinal: entry.ordinal ?? 0, known: big(entry.knownRaw), payments: entry.payments,
        unknown: entry.unknown, uncovered: entry.uncovered });
    }
    for (const entry of chunk.days) {
      const item = own[entry.item];
      days.push({ tag, item, category: categoryOf(item), hour: entry.day ?? entry.read, read: entry.read, days: entry.days,
        charge: big(entry.chargeRaw), payments: entry.payments, unknown: entry.unknown, uncovered: entry.uncovered,
        realized: big(entry.realizedRaw), credits: entry.credits });
    }
    for (const entry of chunk.gains) gains.push({ tag, read: entry.read, gained: big(entry.gainedRaw), capped: big(entry.cappedRaw),
      days: entry.days, unknownDays: entry.unknownDays, unmatched: entry.unmatched });
    for (const run of chunk.coverage) coverage.push({ ...run, tag });
    for (const warning of chunk.warnings) warnings.add(warning);
  }
  const ideaTooltips = mergeTips(chunks.map(chunk => chunk.ideaTooltips)) ?? null;
  const byRead = <T extends { read: number }>(rows: T[]) => rows.sort((a, b) => a.read - b.read);
  return { tag, currency, payments: byRead(payments), days: byRead(days), gains: byRead(gains),
    coverage: joinCoverage(coverage), icons, categories, items, ideaTooltips, warnings: [...warnings] };
}

/** Coverage runs in order, a chunk edge inside an unchanged run joined. */
function joinCoverage(runs: CoverageRun[]): CoverageRun[] {
  const out: CoverageRun[] = [];
  for (const run of [...runs].sort((a, b) => a.firstHour - b.firstHour)) {
    const last = out.at(-1);
    if (last && last.lastHour + 1 === run.firstHour && last.covered === run.covered && last.reason === run.reason) last.lastHour = run.lastHour;
    else out.push({ ...run });
  }
  return out;
}

/** How a group of countries limits what is shown: a payment of a tag counts at an hour the group holds it. */
export interface SpendingSelection {
  matches: (tag: string, hour: number) => boolean;
  membership: ReadonlyMap<string, readonly HourSpan[]> | null;
  unobserved: readonly HourSpan[];
}
/** What is shown: read through `hour`, paid from `from` on (a range; null for the whole branch). */
export interface SpendingWindow { from: number | null; hour: number }

const inWindow = (entry: { tag: string; hour: number; read: number }, window: SpendingWindow, selection?: SpendingSelection) =>
  entry.read <= window.hour && (window.from === null || entry.hour >= window.from) && (!selection || selection.matches(entry.tag, entry.hour));

/** The number of entries of `rows` (in read order) read through `hour`. */
export function readThrough(rows: readonly { read: number }[], hour: number): number {
  let low = 0, high = rows.length;
  while (low < high) { const middle = (low + high) >> 1; if (rows[middle].read <= hour) low = middle + 1; else high = middle; }
  return low;
}

/** The number of `hours` (ascending) at or before `hour`. */
export function countThrough(hours: readonly number[], hour: number): number {
  let low = 0, high = hours.length;
  while (low < high) { const middle = (low + high) >> 1; if (hours[middle] <= hour) low = middle + 1; else high = middle; }
  return low;
}

export const paymentsIn = (ledgers: readonly PowerLedger[], window: SpendingWindow, selection?: SpendingSelection) =>
  ledgers.flatMap(ledger => ledger.payments.slice(0, readThrough(ledger.payments, window.hour)).filter(entry => inWindow(entry, window, selection)));
export const daysIn = (ledgers: readonly PowerLedger[], window: SpendingWindow, selection?: SpendingSelection) =>
  ledgers.flatMap(ledger => ledger.days.slice(0, readThrough(ledger.days, window.hour)).filter(entry => inWindow(entry, window, selection)));

/** A category's shown name; one with no recorded cause is plain “Other spending”. */
export const categoryLabel = (name: string, identified: boolean) => identified ? name : 'Other spending';
const UNRESOLVED = 'Not identified';
/** The recorded action, or the kind of payment when its name or target wasn't recorded. */
export function spendLabel(item: SpendItem, category: string): string {
  return item.action.includes(UNRESOLVED) ? item.name ?? category : item.action;
}

export interface SpendingSummary { spent: bigint; unknownCosts: number; uncovered: boolean }
/** The exact sum of recorded costs; a cost that wasn't recorded is counted, not guessed. */
export function spendingSummary(payments: readonly PaymentEntry[], days: readonly DayEntry[]): SpendingSummary {
  let spent = 0n, unknownCosts = 0, uncovered = false;
  for (const entry of payments) { spent += entry.known ?? 0n; unknownCosts += entry.known === null ? entry.payments : entry.unknown; uncovered ||= entry.uncovered; }
  for (const entry of days) { spent += entry.charge ?? 0n; unknownCosts += entry.charge === null ? entry.payments : entry.unknown; uncovered ||= entry.uncovered > 0; }
  return { spent, unknownCosts, uncovered };
}

const plural = (amount: number, one: string) => `${amount.toLocaleString('en-US')} ${one}${amount === 1 ? '' : 's'}`;
const duration = (hours: number) => hours < 48 ? plural(hours, 'hour') : plural(Math.floor(hours / 24), 'day');

/** Notes for costs counted without their amount, in plain words. */
export function spendingCostNotes(summary: SpendingSummary): string[] {
  return [
    ...(summary.unknownCosts ? [`The cost of ${plural(summary.unknownCosts, 'payment')} wasn’t recorded.`] : []),
    ...(summary.uncovered ? ['Some payments may be missing.'] : []),
  ];
}

export interface SpendGap { from: number; through: number; reason: string; tag: string }
/** Unrecorded stretches of the window, joined when they touch: no empty hour proves nothing was spent, so each is named. */
export function spendingGaps(ledgers: readonly PowerLedger[], window: SpendingWindow, selection?: SpendingSelection): SpendGap[] {
  const gaps: SpendGap[] = [];
  for (const ledger of ledgers) {
    let last: SpendGap | undefined;
    for (const run of ledger.coverage) {
      const from = Math.max(run.firstHour, window.from ?? run.firstHour), through = Math.min(run.lastHour, window.hour);
      if (from > through) continue;
      if (run.covered) { last = undefined; continue; }
      if (last && from <= last.through + 1) last.through = Math.max(last.through, through);
      else gaps.push(last = { from, through, reason: run.reason ?? 'Spending not recorded', tag: ledger.tag });
    }
  }
  if (!selection?.membership) return gaps;
  const selected = gaps.flatMap(gap => (selection.membership!.get(gap.tag) ?? []).flatMap(span => {
    const from = Math.max(gap.from, span.firstHour), through = Math.min(gap.through, span.lastHour);
    return from <= through ? [{ ...gap, from, through }] : [];
  }));
  for (const span of selection.unobserved) {
    const from = Math.max(span.firstHour, window.from ?? span.firstHour), through = Math.min(span.lastHour, window.hour);
    if (from <= through) selected.push({ from, through, tag: '', reason: 'Group membership was not recorded; spending for this group is not known.' });
  }
  return selected;
}

/** One plain sentence naming unrecorded time: how long and on which dates; equal stretches of several countries count once. */
export function spendingGapNote(gaps: readonly SpendGap[]): string | null {
  const stretches = [...new Map(gaps.map(gap => [`${gap.from}:${gap.through}`, gap])).values()].sort((a, b) => a.from - b.from);
  if (!stretches.length) return null;
  const hours = stretches.reduce((sum, gap) => sum + gap.through - gap.from + 1, 0);
  const dates = [...new Set(stretches.map(gap => dayLabel(gap.from)))];
  const shown = dates.length > 3 ? `${dates.slice(0, 3).join(', ')} and ${plural(dates.length - 3, 'other date')}` : dates.join(', ');
  return `Spending wasn’t recorded for ${duration(hours)} (${shown}); anything spent then isn’t counted.`;
}

export interface PowerGain { gained: bigint; lost: bigint; complete: boolean }
/** What the days counted in the window gained before daily costs, and what the cap kept off; null when no day was counted. */
export function powerGain(ledgers: readonly PowerLedger[], window: SpendingWindow, selection?: Pick<SpendingSelection, 'matches'>): PowerGain | null {
  let gained = 0n, lost = 0n, complete = true, any = false;
  for (const ledger of ledgers) for (const entry of ledger.gains.slice(0, readThrough(ledger.gains, window.hour))) {
    if ((window.from !== null && entry.read < window.from) || (selection && !selection.matches(ledger.tag, entry.read))) continue;
    any = true;
    if (entry.gained === null || entry.unknownDays) complete = false;
    gained += entry.gained ?? 0n;
    lost += entry.capped ?? 0n;
  }
  return any ? { gained, lost, complete } : null;
}

/** `powerGain` from `from` on at any hour, from running sums made once: a frame of playback reads one binary search,
 * not every day counted. */
export function powerGains(ledgers: readonly PowerLedger[], from: number | null, selection?: Pick<SpendingSelection, 'matches'>): (hour: number) => PowerGain | null {
  const entries = ledgers.flatMap(ledger => ledger.gains.filter(entry => (from === null || entry.read >= from) && (!selection || selection.matches(ledger.tag, entry.read))))
    .sort((a, b) => a.read - b.read);
  const reads: number[] = [], gained: bigint[] = [], lost: bigint[] = [], incomplete: number[] = [];
  let sumGained = 0n, sumLost = 0n, unknown = 0;
  for (const entry of entries) {
    sumGained += entry.gained ?? 0n; sumLost += entry.capped ?? 0n;
    if (entry.gained === null || entry.unknownDays) unknown++;
    reads.push(entry.read); gained.push(sumGained); lost.push(sumLost); incomplete.push(unknown);
  }
  return hour => {
    const at = countThrough(reads, hour) - 1;
    return at < 0 ? null : { gained: gained[at], lost: lost[at], complete: incomplete[at] === 0 };
  };
}

/** `spendingCostNotes` of what `groups` hold through `hour`, from their running sums: the same entries as the window's
 * payments and days (`spendingGroups` cuts them by `from` and the selection as `paymentsIn` does). */
export function groupCostNotes(groups: ReadonlyMap<string, SpendingGroup>, hour: number): string[] {
  return spendingCostNotes(groupSummary(groups, hour));
}

/** `spendingSummary` of the window's payments and days through `hour`, from the groups' running sums. */
export function groupSummary(groups: ReadonlyMap<string, SpendingGroup>, hour: number): SpendingSummary {
  let spent = 0n, unknownCosts = 0, uncovered = false;
  for (const group of groups.values()) {
    const at = groupAt(group, hour);
    if (at) { spent += at.charge; unknownCosts += at.unknown; uncovered ||= at.uncovered; }
  }
  return { spent, unknownCosts, uncovered };
}

/** Whether two hours' items draw the same slices: a frame whose hour moved but whose spending did not keeps the
 * items it had, and the chart draws nothing new. */
export function sameSpendingItems(a: readonly SunburstItem[], b: readonly SunburstItem[]): boolean {
  return a.length === b.length && a.every((item, at) => {
    const other = b[at];
    return item.exact === other.exact && item.present === other.present && item.count === other.count && item.final === other.final
      && item.note === other.note && item.path.length === other.path.length && item.path.every((step, depth) => step.id === other.path[depth].id && step.label === other.path[depth].label);
  });
}

/** The first hour any of `groups` was paid: before it a breakdown shows nothing spent. */
export function firstPaid(groups: ReadonlyMap<string, SpendingGroup>): number | null {
  let first: number | null = null;
  for (const group of groups.values()) if (group.reads.length && (first === null || group.reads[0] < first)) first = group.reads[0];
  return first;
}

export interface SpendingTableRow {
  key: string;
  tag: string;
  /** The payment's own hour; a folded month shows its latest day. */
  hour: number;
  /** The game month a folded recurring row covers. */
  month: number | null;
  label: string;
  icon: string | null;
  category: string;
  categoryName: string;
  identified: boolean;
  /** Days a folded row covers at the moment its rows were made; 0 for a payment. */
  days: number;
  /** Exact recorded cost; null when none of its costs were recorded. */
  amount: bigint | null;
  partial: boolean;
  /** Native order: hour, then action sequence, then ordinal. */
  order: bigint;
  payment: PaymentEntry | null;
  /** A folded month's day counts, every one of the loaded ledger's (also those after the moment shown). */
  monthDays: readonly DayEntry[];
}

const ORDER_HOUR = 1n << 80n;
const paymentOrder = (entry: PaymentEntry) => BigInt(entry.hour) * ORDER_HOUR + (entry.sequence << 16n) + BigInt(entry.ordinal);
export const monthOf = (hour: number) => { const date = gameDate(hour); return date ? monthIndex(date) : null; };
export const monthLabel = (month: number) => { const { year, month: number } = monthAt(month); return `${monthNames[number - 1].slice(0, 3)} ${year}`; };

/** A folded month's figures through `hour`. */
export function monthFigures(row: Pick<SpendingTableRow, 'monthDays'>, hour: number, from: number | null = null) {
  let amount = 0n, days = 0, missing = 0, known = 0, partial = false, latest: DayEntry | null = null;
  for (const entry of row.monthDays) {
    if (entry.read > hour) break;
    if (from !== null && entry.hour < from) continue;
    days += entry.days;
    if (entry.charge === null) missing += entry.payments; else { amount += entry.charge; known += 1; missing += entry.unknown; }
    partial ||= entry.uncovered > 0 || entry.unknown > 0 || entry.charge === null;
    latest = entry;
  }
  return { amount: known ? amount : missing ? null : 0n, days, partial, latest };
}

/** One row per one-off payment and, per country, one row per recurring category and game month. Category filters never
 * change an amount. */
export function spendingTableRows(ledgers: readonly PowerLedger[], window: SpendingWindow, categories: string, selection?: SpendingSelection): SpendingTableRow[] {
  const chosen = new Set(contentSet(categories).filter(value => value !== NO_SPENDING_CATEGORIES));
  const rows: SpendingTableRow[] = [];
  for (const entry of paymentsIn(ledgers, window, selection)) {
    if (chosen.size && !chosen.has(setToken(entry.category.id))) continue;
    const categoryName = categoryLabel(entry.category.name, entry.category.identified);
    rows.push({ key: JSON.stringify([entry.tag, entry.item.id, entry.read]), tag: entry.tag, hour: entry.hour, month: null,
      label: spendLabel(entry.item, categoryName), icon: entry.item.icon, category: entry.category.id, categoryName,
      identified: entry.category.identified, days: 0, amount: entry.known, partial: entry.unknown > 0 || entry.uncovered,
      order: paymentOrder(entry), payment: entry, monthDays: [] });
  }
  const months = new Map<string, DayEntry[]>();
  for (const ledger of ledgers) for (const entry of ledger.days) {
    if ((selection && !selection.matches(entry.tag, entry.hour)) || (window.from !== null && entry.hour < window.from)) continue;
    if (chosen.size && !chosen.has(setToken(entry.category.id))) continue;
    const month = monthOf(entry.hour);
    if (month === null) continue;
    const key = JSON.stringify(['month', entry.tag, entry.category.id, month, RECURRING_CATEGORIES.has(entry.category.id) ? null : entry.item.groupId]);
    const held = months.get(key);
    if (held) held.push(entry); else months.set(key, [entry]);
  }
  for (const [key, entries] of months) {
    if (entries[0].read > window.hour) continue;
    const [, tag, category, month, group] = JSON.parse(key) as [string, string, string, number, string | null];
    const first = entries[0];
    const now = monthFigures({ monthDays: entries }, window.hour, window.from);
    const latest = now.latest ?? first;
    const categoryName = categoryLabel(first.category.name, first.category.identified);
    rows.push({ key, tag, hour: latest.hour, month, label: group === null ? categoryName : spendLabel(latest.item, categoryName), icon: group === null ? null : latest.item.icon, category, categoryName, identified: first.category.identified,
      days: now.days, amount: now.amount, partial: now.partial, order: BigInt(latest.hour) * ORDER_HOUR, payment: null, monthDays: entries });
  }
  return rows;
}

/** A folded month's amount by item through `hour`, largest first. */
export function monthItems(row: SpendingTableRow, hour: number, from: number | null = null): [string, bigint][] {
  const byItem = new Map<string, bigint>();
  for (const entry of row.monthDays) {
    if (entry.read > hour) break;
    if (from !== null && entry.hour < from) continue;
    const name = entry.item.name ?? spendLabel(entry.item, row.categoryName);
    byItem.set(name, (byItem.get(name) ?? 0n) + (entry.charge ?? 0n));
  }
  return [...byItem].sort((a, b) => a[1] === b[1] ? a[0].localeCompare(b[0]) : a[1] > b[1] ? -1 : 1);
}

/** The names and flags mounted rows show: each country at its row's date when several are compared, and each payment's target
 * at its own hour; with `ahead`, those of the rows a block draws ahead too. */
export function spendingCountryRequests(drawn: readonly SpendingTableRow[], compare: boolean, ahead: readonly SpendingTableRow[] = []): { wanted: NameAt[]; maxHours: number } {
  const names = new Map<string, NameAt>();
  for (const row of drawn.length ? ahead.length ? [...drawn, ...ahead] : drawn : ahead) {
    if (compare) names.set(nameKey(row.hour, row.tag), { hour: row.hour, tag: row.tag });
    const target = row.payment?.item.targetTag;
    if (target) names.set(nameKey(row.hour, target), { hour: row.hour, tag: target });
  }
  const wanted = [...names.values()];
  return { wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
}

/** The rows a block draws from `from` through `through`: payments read in it, and months whose latest day is in it. */
export function spendingRowsAhead(rows: readonly SpendingTableRow[], from: number, through: number): SpendingTableRow[] {
  return rows.filter(row => { const at = row.payment?.read ?? row.hour; return at >= from && at <= through; });
}

/** Whether every name `next` asks is among those `held` asks. */
export function namesCovered(held: readonly NameAt[], next: readonly NameAt[]): boolean {
  if (next.length > held.length) return false;
  const keys = new Set(held.map(item => nameKey(item.hour, item.tag)));
  return next.every(item => keys.has(nameKey(item.hour, item.tag)));
}

/** Category chips: every category spent on, by its shown name. */
export function categoryChoices(ledgers: readonly PowerLedger[] | undefined) {
  const used = new Map<string, SpendCategory>();
  for (const ledger of ledgers ?? []) for (const entry of [...ledger.payments, ...ledger.days]) used.set(entry.category.id, entry.category);
  return [...used.values()].map(category => {
    const value = setToken(category.id);
    return { value, label: categoryLabel(category.name, category.identified), identified: category.identified,
      unavailable: storableToken(value) ? undefined : UNSTORABLE };
  });
}

/** Payments by the item they show as, cumulated in read order: what a breakdown reads at any hour without folding again. */
export interface SpendingGroup {
  id: string;
  /** The latest item shown in it, its category and its picture (a drawable image). */
  latest: SpendItem;
  image: string | null;
  category: SpendCategory;
  reads: number[];
  /** Per entry: the recorded charge, payments, those whose charge was not recorded, and whether one was read uncovered, all
   * cumulated. */
  charge: bigint[];
  payments: number[];
  unknown: number[];
  uncovered: number[];
  /** Per entry, its own hour: the latest shown. */
  hours: number[];
}

/** The groups of `ledgers`' entries paid from `from` on (every read; a reader cuts them at its hour). */
export function spendingGroups(ledgers: readonly PowerLedger[], from: number | null, selection?: SpendingSelection): Map<string, SpendingGroup> {
  const entries = ledgers.flatMap(ledger => [
    ...ledger.payments.map(entry => ({ entry, image: ledger.icons.get(entry.item.icon ?? '') ?? null, charge: entry.known, payments: entry.payments, unknown: entry.known === null ? entry.payments : entry.unknown, uncovered: entry.uncovered })),
    ...ledger.days.map(entry => ({ entry, image: ledger.icons.get(entry.item.icon ?? '') ?? null, charge: entry.charge, payments: entry.payments, unknown: entry.charge === null ? entry.payments : entry.unknown, uncovered: entry.uncovered > 0 })),
  ]).filter(({ entry }) => (from === null || entry.hour >= from) && (!selection || selection.matches(entry.tag, entry.hour)))
    .sort((a, b) => a.entry.read - b.entry.read);
  const groups = new Map<string, SpendingGroup>();
  for (const { entry, image, charge, payments, unknown, uncovered } of entries) {
    let group = groups.get(entry.item.groupId);
    if (!group) groups.set(entry.item.groupId, group = { id: entry.item.groupId, latest: entry.item, image, category: entry.category, reads: [], charge: [], payments: [], unknown: [], uncovered: [], hours: [] });
    const at = group.reads.length - 1;
    group.reads.push(entry.read);
    group.hours.push(entry.hour);
    group.charge.push((at < 0 ? 0n : group.charge[at]) + (charge ?? 0n));
    group.payments.push((at < 0 ? 0 : group.payments[at]) + payments);
    group.unknown.push((at < 0 ? 0 : group.unknown[at]) + unknown);
    group.uncovered.push((at < 0 ? 0 : group.uncovered[at]) + (uncovered ? 1 : 0));
    if (entry.hour >= (group.hours[at] ?? -Infinity)) { group.latest = entry.item; group.image = image; group.category = entry.category; }
  }
  return groups;
}

/** A group's figures through `hour`: none when it was not paid by then. */
export function groupAt(group: SpendingGroup, hour: number) {
  const at = countThrough(group.reads, hour) - 1;
  return at < 0 ? null : { charge: group.charge[at], payments: group.payments[at], unknown: group.unknown[at], uncovered: group.uncovered[at] > 0, latestHour: group.hours[at] };
}

/** Category, then the item it shows as, with exact fixed5 amounts at `hour` and their whole at `end` (Fill grows toward it).
 * A cost not recorded marks its slice partial; a payment with no recorded name sits in its category without an item slice. */
export function spendingItems(groups: ReadonlyMap<string, SpendingGroup>, hour: number, end: number): SunburstItem[] {
  return [...groups.values()].flatMap(group => {
    const final = groupAt(group, end);
    if (!final) return [];
    const now = groupAt(group, hour);
    const partial = !!now && (now.unknown > 0 || now.uncovered);
    const category = { id: group.category.id, label: categoryLabel(group.category.name, group.category.identified) };
    const path = group.latest.name === null ? [category] : [category, { id: group.id, label: group.latest.name, icon: group.image }];
    return [{ path, value: Number(now?.charge ?? 0n), exact: now?.charge ?? 0n, present: !!now, final: Number(final.charge),
      count: now?.payments ?? 0, unknown: partial ? { incomplete: 1 } : undefined, note: partial ? 'Some costs weren’t recorded.' : undefined }];
  });
}

/** What one day (`day`'s midnight) of `ledger` spent by category, read through `through`: one-off payments read that day and
 * recurring days of that day. Recurring costs are counted as their day ends, so an open day has none yet. */
export function daySpending(ledger: PowerLedger, day: number, through: number) {
  const byCategory = new Map<string, { category: SpendCategory; charge: bigint; unknown: number }>();
  const add = (category: SpendCategory, charge: bigint | null, unknown: number) => {
    const held = byCategory.get(category.id) ?? { category, charge: 0n, unknown: 0 };
    held.charge += charge ?? 0n; held.unknown += charge === null ? Math.max(1, unknown) : unknown;
    byCategory.set(category.id, held);
  };
  for (const entry of ledger.payments) if (entry.read >= day && entry.read < day + 24 && entry.read <= through) add(entry.category, entry.known, entry.unknown);
  for (const entry of ledger.days) if (entry.hour === day && entry.read <= through) add(entry.category, entry.charge, entry.unknown);
  return byCategory;
}

/** A ledger is asked in spans of this many hours, aligned to multiples of it: each reply reads a fixed stretch of the
 * recording, the same whatever the widget shows, so a view that moves asks nothing new. */
export const SPENDING_CHUNK_HOURS = 8192;

/** The aligned spans covering `first..=last`. */
export function spendingChunks(first: number, last: number): [number, number][] {
  const out: [number, number][] = [];
  for (let start = Math.floor(first / SPENDING_CHUNK_HOURS) * SPENDING_CHUNK_HOURS; start <= last; start += SPENDING_CHUNK_HOURS) {
    out.push([Math.max(first, start), Math.min(last, start + SPENDING_CHUNK_HOURS - 1)]);
  }
  return out;
}

/** The hours a widget's ledgers are read over: a range's, else the branch through a pin's end or the branch's end. */
export function spendingSpan(widget: Pick<WidgetSpec, 'time' | 'kind' | 'trackIds'>, track: BoundTrack, tracks: BoundTrack[]): HourSpan | null {
  const window = widget.time.mode === 'range' || widget.time.mode === 'pin' ? resolveWindow(widget as WidgetSpec, track, tracks, 0) : null;
  const first = widget.time.mode === 'range' ? Math.max(track.branch.firstHour, window?.firstHour ?? track.branch.firstHour) : track.branch.firstHour;
  const last = widget.time.mode === 'pin' || widget.time.mode === 'range' ? Math.min(track.branch.lastHour, window?.lastHour ?? track.branch.lastHour) : track.branch.lastHour;
  return first <= last ? { firstHour: first, lastHour: last } : null;
}

/** The hours at which what a spending widget lists changes: a payment read, a recurring month's first day, a coverage
 * run starting. Its structure at a head is how many of these lie at or before it. */
export function structureHours(ledgers: readonly PowerLedger[], selection?: SpendingSelection): number[] {
  const { payments, other } = structureParts(ledgers, selection);
  return [...payments, ...other].sort((a, b) => a - b);
}

/** `structureHours` in two, each ascending: the payments' reads (a row arriving, which a table can hold ahead and
 * reveal), and the rest (a month's row starting or turning partial, a coverage run), which change rows already shown. */
export function structureParts(ledgers: readonly PowerLedger[], selection?: SpendingSelection): { payments: number[]; other: number[] } {
  const payments: number[] = [], other: number[] = [];
  for (const ledger of ledgers) {
    for (const entry of ledger.payments) if (!selection || selection.matches(entry.tag, entry.hour)) payments.push(entry.read);
    const months = new Set<string>();
    for (const entry of ledger.days) {
      const key = `${entry.category.id}:${monthOf(entry.hour)}:${RECURRING_CATEGORIES.has(entry.category.id) ? '' : entry.item.groupId}`;
      if (entry.unknown || entry.charge === null || entry.uncovered || !months.has(key)) other.push(entry.read);
      months.add(key);
    }
    for (const run of ledger.coverage) other.push(run.firstHour);
  }
  return { payments: payments.sort((a, b) => a - b), other: other.sort((a, b) => a - b) };
}

/** How many of `hours` (ascending) lie at or before `hour`. */
export const structureAt = (hours: readonly number[], hour: number) => countThrough(hours, hour);

/** Idea tooltips joined, each idea once; undefined when none is given. */
function mergeTips(given: readonly (SpendIdeaTooltips | null)[]): SpendIdeaTooltips | undefined {
  const tips = given.filter((tip): tip is SpendIdeaTooltips => tip !== null);
  if (!tips.length) return undefined;
  const unique = <T,>(values: readonly T[], key: (value: T) => string) => [...new Map(values.map(value => [key(value), value])).values()];
  return { ideas: unique(tips.flatMap(tip => tip.ideas), info => info.token), artwork: unique(tips.flatMap(tip => tip.artwork), art => art.token),
    modifiers: unique(tips.flatMap(tip => tip.modifiers), format => format.key), texts: unique(tips.flatMap(tip => tip.texts), item => item.key) };
}

/** Every ledger's idea tooltips, each idea once: what `useIdeaTooltips` reads. */
export const mergedIdeaTooltips = (ledgers: readonly PowerLedger[] | undefined) => mergeTips((ledgers ?? []).map(ledger => ledger.ideaTooltips));
