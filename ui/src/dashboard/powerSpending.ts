import type { HourSpan, PowerSpendingReply, SpendBalanceRun, SpendCategory, SpendCoverage, SpendIdeaTooltips, SpendItem } from '../generated/dashboard';
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

export type PowerCurrency = 'Political' | 'Command';

const RAW = /^-?\d{1,39}$/;
const raw = (value: unknown): value is string => typeof value === 'string' && RAW.test(value);
const optionalRaw = (value: unknown) => value === null || raw(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const count = (value: unknown) => integer(value) && value >= 0;
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const list = (value: unknown, each: (entry: unknown) => boolean) => Array.isArray(value) && value.every(each);

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
    && list(reply.gains, entry => object(entry) && (entry.hour === null || integer(entry.hour)) && integer(entry.read) && optionalRaw(entry.gainedRaw) && optionalRaw(entry.cappedRaw)
      && count(entry.days) && count(entry.unknownDays) && count(entry.unmatched))
    && list(reply.coverage, entry => object(entry) && integer(entry.firstHour) && integer(entry.lastHour) && entry.firstHour <= entry.lastHour
      && typeof entry.covered === 'boolean' && text(entry.reason))
    && list(reply.gainCoverage, entry => object(entry) && integer(entry.firstHour) && integer(entry.lastHour) && entry.firstHour <= entry.lastHour
      && typeof entry.covered === 'boolean' && text(entry.reason))
    && list(reply.balances, entry => object(entry) && integer(entry.firstHour) && integer(entry.lastHour) && entry.firstHour <= entry.lastHour
      && optionalRaw(entry.valueRaw) && text(entry.reason))
    && list(reply.icons, icon => object(icon) && typeof icon.key === 'string' && typeof icon.image === 'string');
  if (!ok) throw new Error('The power spending reply is malformed.');
  if (reply.ideaTooltips !== null) validateIdeaTooltipData(reply.ideaTooltips);
  const tooltips = new Set(reply.ideaTooltips?.ideas.map(info => info.token));
  if (reply.items.some(item => item.tooltipToken !== null && !tooltips.has(item.tooltipToken))) throw new Error('The power spending reply has an unlisted idea tooltip.');
  return reply;
}

export interface PaymentEntry {
  tag: string; item: SpendItem; category: SpendCategory;
  hour: number; read: number; sequence: bigint; ordinal: number;
  known: bigint | null; payments: number; unknown: number; uncovered: boolean;
}
export interface DayEntry {
  tag: string; item: SpendItem; category: SpendCategory;
  hour: number; read: number; days: number; charge: bigint | null; payments: number; unknown: number; uncovered: number;
  realized: bigint | null; credits: number;
}
export interface GainEntry { tag: string; hour: number; read: number; gained: bigint | null; capped: bigint | null; days: number; unknownDays: number; unmatched: number }
export interface CoverageRun extends SpendCoverage { tag: string }
export interface BalanceRun { tag: string; firstHour: number; lastHour: number; value: bigint | null; reason: string | null }

export interface PowerLedger {
  tag: string;
  currency: PowerCurrency;
  payments: readonly PaymentEntry[];
  days: readonly DayEntry[];
  gains: readonly GainEntry[];
  coverage: readonly CoverageRun[];
  gainCoverage: readonly CoverageRun[];
  balances: readonly BalanceRun[];
  icons: ReadonlyMap<string, string>;
  categories: ReadonlyMap<string, SpendCategory>;
  items: ReadonlyMap<string, SpendItem>;
  ideaTooltips: SpendIdeaTooltips | null;
  warnings: readonly string[];
}

const big = (value: string | null) => value === null ? null : BigInt(value);
export const RECURRING_CATEGORIES: ReadonlySet<string> = new Set(['dailyCosts', 'organisationUpkeep', 'upkeep', 'dailyDeficit']);
const UNIDENTIFIED: SpendCategory = { id: 'notIdentified', name: 'Not identified', identified: false };

export function ledgerOf(tag: string, currency: PowerCurrency, chunks: readonly PowerSpendingReply[]): PowerLedger {
  const items = new Map<string, SpendItem>();
  const categories = new Map<string, SpendCategory>();
  const icons = new Map<string, string>();
  const payments: PaymentEntry[] = [], days: DayEntry[] = [], gains: GainEntry[] = [], coverage: CoverageRun[] = [], gainCoverage: CoverageRun[] = [], balances: BalanceRun[] = [];
  const warnings = new Set<string>();
  for (const chunk of [...chunks].sort((a, b) => a.from - b.from)) {
    for (const category of chunk.categories) categories.set(category.id, category);
    for (const icon of chunk.icons) { const drawn = drawableIcon(icon.image); if (drawn) icons.set(icon.key, drawn); }
    const own = chunk.items.map(item => { const held = items.get(item.id) ?? item; items.set(item.id, held); return held; });
    const categoryOf = (item: SpendItem) => categories.get(item.category) ?? UNIDENTIFIED;
    for (const entry of chunk.payments) {
      const item = own[entry.item];
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
    for (const entry of chunk.gains) gains.push({ tag, hour: entry.hour ?? entry.read, read: entry.read, gained: big(entry.gainedRaw), capped: big(entry.cappedRaw),
      days: entry.days, unknownDays: entry.unknownDays, unmatched: entry.unmatched });
    for (const run of chunk.coverage) coverage.push({ ...run, tag });
    for (const run of chunk.gainCoverage) gainCoverage.push({ ...run, tag });
    for (const run of chunk.balances) balances.push(balanceEntry(tag, run));
    for (const warning of chunk.warnings) warnings.add(warning);
  }
  const ideaTooltips = mergeTips(chunks.map(chunk => chunk.ideaTooltips)) ?? null;
  const byRead = <T extends { read: number }>(rows: T[]) => rows.sort((a, b) => a.read - b.read);
  return { tag, currency, payments: byRead(payments), days: byRead(days), gains: byRead(gains),
    coverage: joinCoverage(coverage), gainCoverage: joinCoverage(gainCoverage), balances: joinBalances(balances),
    icons, categories, items, ideaTooltips, warnings: [...warnings] };
}

const balanceEntry = (tag: string, run: SpendBalanceRun): BalanceRun => ({
  tag, firstHour: run.firstHour, lastHour: run.lastHour, value: big(run.valueRaw), reason: run.reason,
});

function joinBalances(runs: BalanceRun[]): BalanceRun[] {
  const out: BalanceRun[] = [];
  for (const run of [...runs].sort((a, b) => a.firstHour - b.firstHour)) {
    let firstHour = run.firstHour;
    const last = out.at(-1);
    if (last && firstHour <= last.lastHour) firstHour = last.lastHour + 1;
    if (firstHour > run.lastHour) continue;
    if (last && last.lastHour + 1 === firstHour && last.value === run.value && last.reason === run.reason) last.lastHour = run.lastHour;
    else out.push({ ...run, firstHour });
  }
  return out;
}

function joinCoverage(runs: CoverageRun[]): CoverageRun[] {
  const out: CoverageRun[] = [];
  for (const run of [...runs].sort((a, b) => a.firstHour - b.firstHour)) {
    const last = out.at(-1);
    if (last && last.lastHour + 1 === run.firstHour && last.covered === run.covered && last.reason === run.reason) last.lastHour = run.lastHour;
    else out.push({ ...run });
  }
  return out;
}

export interface SpendingSelection {
  matches: (tag: string, hour: number) => boolean;
  membership: ReadonlyMap<string, readonly HourSpan[]> | null;
  unobserved: readonly HourSpan[];
}
export interface SpendingWindow { from: number | null; hour: number }

const inWindow = (entry: { tag: string; hour: number; read: number }, window: SpendingWindow, selection?: SpendingSelection) =>
  entry.read <= window.hour && (window.from === null || entry.hour >= window.from) && (!selection || selection.matches(entry.tag, entry.hour));

export function readThrough(rows: readonly { read: number }[], hour: number): number {
  let low = 0, high = rows.length;
  while (low < high) { const middle = (low + high) >> 1; if (rows[middle].read <= hour) low = middle + 1; else high = middle; }
  return low;
}

export function countThrough(hours: readonly number[], hour: number): number {
  let low = 0, high = hours.length;
  while (low < high) { const middle = (low + high) >> 1; if (hours[middle] <= hour) low = middle + 1; else high = middle; }
  return low;
}

export const paymentsIn = (ledgers: readonly PowerLedger[], window: SpendingWindow, selection?: SpendingSelection) =>
  ledgers.flatMap(ledger => ledger.payments.slice(0, readThrough(ledger.payments, window.hour)).filter(entry => inWindow(entry, window, selection)));
export const daysIn = (ledgers: readonly PowerLedger[], window: SpendingWindow, selection?: SpendingSelection) =>
  ledgers.flatMap(ledger => ledger.days.slice(0, readThrough(ledger.days, window.hour)).filter(entry => inWindow(entry, window, selection)));

export const categoryLabel = (name: string, identified: boolean) => identified ? name : 'Other spending';
const UNRESOLVED = 'Not identified';
export function spendLabel(item: SpendItem, category: string): string {
  return item.action.includes(UNRESOLVED) ? item.name ?? category : item.action;
}

export interface SpendingSummary { spent: bigint; unknownCosts: number; uncovered: boolean }
export function spendingSummary(payments: readonly PaymentEntry[], days: readonly DayEntry[]): SpendingSummary {
  let spent = 0n, unknownCosts = 0, uncovered = false;
  for (const entry of payments) { spent += entry.known ?? 0n; unknownCosts += entry.known === null ? entry.payments : entry.unknown; uncovered ||= entry.uncovered; }
  for (const entry of days) { spent += entry.charge ?? 0n; unknownCosts += entry.charge === null ? entry.payments : entry.unknown; uncovered ||= entry.uncovered > 0; }
  return { spent, unknownCosts, uncovered };
}

const plural = (amount: number, one: string) => `${amount.toLocaleString('en-US')} ${one}${amount === 1 ? '' : 's'}`;
const duration = (hours: number) => hours < 48 ? plural(hours, 'hour') : plural(Math.floor(hours / 24), 'day');

export function spendingCostNotes(summary: SpendingSummary): string[] {
  return [
    ...(summary.unknownCosts ? [`The cost of ${plural(summary.unknownCosts, 'payment')} wasn’t recorded.`] : []),
    ...(summary.uncovered ? ['Some payments may be missing.'] : []),
  ];
}

export interface SpendGap { from: number; through: number; reason: string; tag: string }
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

export function spendingGapNote(gaps: readonly SpendGap[]): string | null {
  const stretches = [...new Map(gaps.map(gap => [`${gap.from}:${gap.through}`, gap])).values()].sort((a, b) => a.from - b.from);
  if (!stretches.length) return null;
  const hours = stretches.reduce((sum, gap) => sum + gap.through - gap.from + 1, 0);
  const dates = [...new Set(stretches.map(gap => dayLabel(gap.from)))];
  const shown = dates.length > 3 ? `${dates.slice(0, 3).join(', ')} and ${plural(dates.length - 3, 'other date')}` : dates.join(', ');
  return `Spending wasn’t recorded for ${duration(hours)} (${shown}); anything spent then isn’t counted.`;
}

export const OTHER_CHANGES_HINT = 'Balance change the recording does not attribute to a gain or a spend';

export interface PowerGain { gained: bigint; lost: bigint; complete: boolean }
export function powerGain(ledgers: readonly PowerLedger[], window: SpendingWindow, selection?: Pick<SpendingSelection, 'matches'>): PowerGain | null {
  let gained = 0n, lost = 0n, complete = true, any = false;
  for (const ledger of ledgers) for (const entry of ledger.gains.slice(0, readThrough(ledger.gains, window.hour))) {
    if ((window.from !== null && entry.hour < window.from) || (selection && !selection.matches(ledger.tag, entry.hour))) continue;
    any = true;
    if (entry.gained === null || entry.capped === null || entry.unknownDays || entry.unmatched) complete = false;
    gained += entry.gained ?? 0n;
    lost += entry.capped ?? 0n;
  }
  return any ? { gained, lost, complete } : null;
}

export function powerGains(ledgers: readonly PowerLedger[], from: number | null, selection?: Pick<SpendingSelection, 'matches'>): (hour: number) => PowerGain | null {
  const entries = ledgers.flatMap(ledger => ledger.gains.filter(entry => (from === null || entry.hour >= from) && (!selection || selection.matches(ledger.tag, entry.hour))))
    .sort((a, b) => a.read - b.read);
  const reads: number[] = [], gained: bigint[] = [], lost: bigint[] = [], incomplete: number[] = [];
  let sumGained = 0n, sumLost = 0n, unknown = 0;
  for (const entry of entries) {
    sumGained += entry.gained ?? 0n; sumLost += entry.capped ?? 0n;
    if (entry.gained === null || entry.capped === null || entry.unknownDays || entry.unmatched) unknown++;
    reads.push(entry.read); gained.push(sumGained); lost.push(sumLost); incomplete.push(unknown);
  }
  return hour => {
    const at = countThrough(reads, hour) - 1;
    return at < 0 ? null : { gained: gained[at], lost: lost[at], complete: incomplete[at] === 0 };
  };
}

export type PowerReconciliation =
  | { kind: 'zero' }
  | { kind: 'value'; other: bigint }
  | { kind: 'unrecorded'; reason: string };

interface IndexedCoverage {
  runs: readonly CoverageRun[];
  badPrefix: number[];
  gapPrefix: number[];
}

function indexCoverage(runs: readonly CoverageRun[]): IndexedCoverage {
  const badPrefix: number[] = [], gapPrefix: number[] = [];
  let bad = 0, gaps = 0;
  for (let i = 0; i < runs.length; i++) {
    if (!runs[i].covered) bad++;
    if (i > 0 && runs[i - 1].lastHour + 1 !== runs[i].firstHour) gaps++;
    badPrefix.push(bad);
    gapPrefix.push(gaps);
  }
  return { runs, badPrefix, gapPrefix };
}

function findRunAt<T extends { firstHour: number; lastHour: number }>(runs: readonly T[], hour: number): number {
  let low = 0, high = runs.length - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (hour < runs[mid].firstHour) high = mid - 1;
    else if (hour > runs[mid].lastHour) low = mid + 1;
    else return mid;
  }
  return -1;
}

function checkCoverage(indexed: IndexedCoverage, first: number, last: number, fallback: string): string | null {
  if (first > last) return null;
  const a = findRunAt(indexed.runs, first), b = findRunAt(indexed.runs, last);
  if (a < 0 || b < 0) return fallback;
  const badCount = indexed.badPrefix[b] - (a > 0 ? indexed.badPrefix[a - 1] : 0);
  if (badCount > 0) {
    for (let i = a; i <= b; i++) if (!indexed.runs[i].covered) return indexed.runs[i].reason ?? fallback;
  }
  if (indexed.gapPrefix[b] - indexed.gapPrefix[a] > 0) return fallback;
  return null;
}

interface IndexedBalances {
  runs: readonly BalanceRun[];
  missingPrefix: number[];
  gapPrefix: number[];
}

function indexBalances(runs: readonly BalanceRun[]): IndexedBalances {
  const missingPrefix: number[] = [], gapPrefix: number[] = [];
  let missing = 0, gaps = 0;
  for (let i = 0; i < runs.length; i++) {
    if (runs[i].value === null) missing++;
    if (i > 0 && runs[i - 1].lastHour + 1 !== runs[i].firstHour) gaps++;
    missingPrefix.push(missing);
    gapPrefix.push(gaps);
  }
  return { runs, missingPrefix, gapPrefix };
}

function balanceChange(indexed: IndexedBalances, first: number, last: number): { delta: bigint } | { reason: string } {
  const a = findRunAt(indexed.runs, first), b = findRunAt(indexed.runs, last);
  if (a < 0 || b < 0) return { reason: 'Balance was not recorded.' };
  const missingCount = indexed.missingPrefix[b] - (a > 0 ? indexed.missingPrefix[a - 1] : 0);
  if (missingCount > 0) {
    for (let i = a; i <= b; i++) if (indexed.runs[i].value === null) return { reason: indexed.runs[i].reason ?? 'Balance was not recorded.' };
  }
  if (indexed.gapPrefix[b] - indexed.gapPrefix[a] > 0) return { reason: 'Balance was not recorded for part of this period.' };
  return { delta: indexed.runs[b].value! - indexed.runs[a].value! };
}

function spansCover(spans: readonly HourSpan[], first: number, last: number): boolean {
  if (first > last) return true;
  let cursor = first;
  for (const span of spans) {
    if (span.lastHour < cursor) continue;
    if (span.firstHour > cursor) return false;
    cursor = span.lastHour + 1;
    if (cursor > last) return true;
  }
  return cursor > last;
}

export function powerReconciler(
  ledgers: readonly PowerLedger[],
  groups: ReadonlyMap<string, SpendingGroup>,
  from: number | null,
  branchFirstHour: number,
  selection?: SpendingSelection,
): (hour: number) => PowerReconciliation {
  const openingHour = from === null || from <= branchFirstHour ? branchFirstHour : from - 1;
  const interiorStart = openingHour + 1;
  const indexed = ledgers.map(ledger => ({
    tag: ledger.tag,
    coverage: indexCoverage(ledger.coverage),
    gainCoverage: indexCoverage(ledger.gainCoverage),
    balances: indexBalances(ledger.balances),
  }));
  const gainEntries = ledgers.flatMap(ledger => ledger.gains.filter(entry => (from === null || entry.hour >= from) && (!selection || selection.matches(ledger.tag, entry.hour))))
    .sort((a, b) => a.read - b.read);
  const reads: number[] = [], gained: bigint[] = [], lost: bigint[] = [], gainMiss: number[] = [], capMiss: number[] = [], unmatched: number[] = [];
  let sumGained = 0n, sumLost = 0n, gMiss = 0, cMiss = 0, unm = 0;
  for (const entry of gainEntries) {
    sumGained += entry.gained ?? 0n;
    sumLost += entry.capped ?? 0n;
    if (entry.gained === null || entry.unknownDays > 0) gMiss++;
    if (entry.capped === null) cMiss++;
    if (entry.unmatched > 0) unm++;
    reads.push(entry.read);
    gained.push(sumGained);
    lost.push(sumLost);
    gainMiss.push(gMiss);
    capMiss.push(cMiss);
    unmatched.push(unm);
  }
  return hour => {
    if (!indexed.length) return { kind: 'unrecorded', reason: 'No country selected.' };
    if (hour < openingHour) return { kind: 'unrecorded', reason: 'Outside recording span.' };
    if (selection?.membership) {
      for (const span of selection.unobserved) {
        if (span.firstHour <= hour && span.lastHour >= openingHour) {
          return { kind: 'unrecorded', reason: 'Group membership was not recorded for part of this period.' };
        }
      }
      const checkStart = interiorStart <= hour ? interiorStart : openingHour;
      for (const entry of indexed) {
        if (!spansCover(selection.membership.get(entry.tag) ?? [], checkStart, hour)) {
          return { kind: 'unrecorded', reason: 'Group membership changed during this period.' };
        }
      }
    }
    let balanceDelta = 0n;
    for (const entry of indexed) {
      const bal = balanceChange(entry.balances, openingHour, hour);
      if ('reason' in bal) return { kind: 'unrecorded', reason: bal.reason };
      balanceDelta += bal.delta;
      const covErr = checkCoverage(entry.coverage, interiorStart, hour, 'Spending coverage was not recorded.');
      if (covErr) return { kind: 'unrecorded', reason: covErr };
      const gainCovErr = checkCoverage(entry.gainCoverage, interiorStart, hour, 'Power changes were not recorded.');
      if (gainCovErr) return { kind: 'unrecorded', reason: gainCovErr };
    }
    const at = countThrough(reads, hour) - 1;
    let totalGained = 0n, totalLost = 0n;
    if (at >= 0) {
      if (gainMiss[at] > 0) return { kind: 'unrecorded', reason: 'Gain was not recorded for part of this period.' };
      if (capMiss[at] > 0) return { kind: 'unrecorded', reason: 'Amount lost at cap was not recorded for part of this period.' };
      if (unmatched[at] > 0) return { kind: 'unrecorded', reason: 'Daily costs could not be matched to daily gain.' };
      totalGained = gained[at];
      totalLost = lost[at];
    }
    const summary = groupSummary(groups, hour);
    if (summary.unknownCosts > 0) return { kind: 'unrecorded', reason: 'Some spending costs were not recorded.' };
    if (summary.uncovered) return { kind: 'unrecorded', reason: 'Some payments were read without full coverage.' };
    const other = balanceDelta - (totalGained - totalLost - summary.spent);
    return other === 0n ? { kind: 'zero' } : { kind: 'value', other };
  };
}

export function powerReconciliation(
  ledgers: readonly PowerLedger[],
  window: SpendingWindow,
  branchFirstHour: number,
  selection?: SpendingSelection,
): PowerReconciliation {
  const groups = spendingGroups(ledgers, window.from, selection);
  return powerReconciler(ledgers, groups, window.from, branchFirstHour, selection)(window.hour);
}

export function groupCostNotes(groups: ReadonlyMap<string, SpendingGroup>, hour: number): string[] {
  return spendingCostNotes(groupSummary(groups, hour));
}

export function groupSummary(groups: ReadonlyMap<string, SpendingGroup>, hour: number): SpendingSummary {
  let spent = 0n, unknownCosts = 0, uncovered = false;
  for (const group of groups.values()) {
    const at = groupAt(group, hour);
    if (at) { spent += at.charge; unknownCosts += at.unknown; uncovered ||= at.uncovered; }
  }
  return { spent, unknownCosts, uncovered };
}

export function sameSpendingItems(a: readonly SunburstItem[], b: readonly SunburstItem[]): boolean {
  return a.length === b.length && a.every((item, at) => {
    const other = b[at];
    return item.exact === other.exact && item.present === other.present && item.count === other.count && item.final === other.final
      && item.note === other.note && item.path.length === other.path.length && item.path.every((step, depth) => step.id === other.path[depth].id && step.label === other.path[depth].label);
  });
}

export function firstPaid(groups: ReadonlyMap<string, SpendingGroup>): number | null {
  let first: number | null = null;
  for (const group of groups.values()) if (group.reads.length && (first === null || group.reads[0] < first)) first = group.reads[0];
  return first;
}

export interface SpendingTableRow {
  key: string;
  tag: string;
  hour: number;
  month: number | null;
  label: string;
  icon: string | null;
  category: string;
  categoryName: string;
  identified: boolean;
  days: number;
  amount: bigint | null;
  partial: boolean;
  order: bigint;
  payment: PaymentEntry | null;
  monthDays: readonly DayEntry[];
}

const ORDER_HOUR = 1n << 80n;
const paymentOrder = (entry: PaymentEntry) => BigInt(entry.hour) * ORDER_HOUR + (entry.sequence << 16n) + BigInt(entry.ordinal);
export const monthOf = (hour: number) => { const date = gameDate(hour); return date ? monthIndex(date) : null; };
export const monthLabel = (month: number) => { const { year, month: number } = monthAt(month); return `${monthNames[number - 1].slice(0, 3)} ${year}`; };

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

export function stableSpendingRows(held: ReadonlyMap<string, SpendingTableRow>, rows: readonly SpendingTableRow[]): SpendingTableRow[] {
  return rows.map(row => {
    const before = held.get(row.key);
    return before && sameSpendingRow(before, row) ? before : row;
  });
}

function sameSpendingRow(a: SpendingTableRow, b: SpendingTableRow): boolean {
  return a.tag === b.tag && a.hour === b.hour && a.month === b.month && a.label === b.label && a.icon === b.icon && a.category === b.category
    && a.categoryName === b.categoryName && a.identified === b.identified && a.days === b.days && a.amount === b.amount && a.partial === b.partial
    && a.order === b.order && a.payment === b.payment && a.monthDays.length === b.monthDays.length
    && a.monthDays.every((day, at) => day === b.monthDays[at]);
}

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

export function spendingRowsAhead(rows: readonly SpendingTableRow[], from: number, through: number): SpendingTableRow[] {
  return rows.filter(row => { const at = row.payment?.read ?? row.hour; return at >= from && at <= through; });
}

export function namesCovered(held: readonly NameAt[], next: readonly NameAt[]): boolean {
  if (next.length > held.length) return false;
  const keys = new Set(held.map(item => nameKey(item.hour, item.tag)));
  return next.every(item => keys.has(nameKey(item.hour, item.tag)));
}

export function categoryChoices(ledgers: readonly PowerLedger[] | undefined) {
  const used = new Map<string, SpendCategory>();
  for (const ledger of ledgers ?? []) for (const entry of [...ledger.payments, ...ledger.days]) used.set(entry.category.id, entry.category);
  return [...used.values()].map(category => {
    const value = setToken(category.id);
    return { value, label: categoryLabel(category.name, category.identified), identified: category.identified,
      unavailable: storableToken(value) ? undefined : UNSTORABLE };
  });
}

export interface SpendingGroup {
  id: string;
  latest: SpendItem;
  image: string | null;
  category: SpendCategory;
  reads: number[];
  charge: bigint[];
  payments: number[];
  unknown: number[];
  uncovered: number[];
  hours: number[];
}

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

export function groupAt(group: SpendingGroup, hour: number) {
  const at = countThrough(group.reads, hour) - 1;
  return at < 0 ? null : { charge: group.charge[at], payments: group.payments[at], unknown: group.unknown[at], uncovered: group.uncovered[at] > 0, latestHour: group.hours[at] };
}

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

export const SPENDING_CHUNK_HOURS = 8192;

export function spendingChunks(first: number, last: number): [number, number][] {
  const out: [number, number][] = [];
  for (let start = Math.floor(first / SPENDING_CHUNK_HOURS) * SPENDING_CHUNK_HOURS; start <= last; start += SPENDING_CHUNK_HOURS) {
    out.push([Math.max(first, start), Math.min(last, start + SPENDING_CHUNK_HOURS - 1)]);
  }
  return out;
}

export function spendingSpan(widget: Pick<WidgetSpec, 'time' | 'kind' | 'trackIds'>, track: BoundTrack, tracks: BoundTrack[]): HourSpan | null {
  const window = widget.time.mode === 'range' || widget.time.mode === 'pin' ? resolveWindow(widget as WidgetSpec, track, tracks, 0) : null;
  const first = widget.time.mode === 'range' ? Math.max(track.branch.firstHour, window?.firstHour ?? track.branch.firstHour) : track.branch.firstHour;
  const last = widget.time.mode === 'pin' || widget.time.mode === 'range' ? Math.min(track.branch.lastHour, window?.lastHour ?? track.branch.lastHour) : track.branch.lastHour;
  return first <= last ? { firstHour: first, lastHour: last } : null;
}

export function structureHours(ledgers: readonly PowerLedger[], selection?: SpendingSelection): number[] {
  const { payments, other } = structureParts(ledgers, selection);
  return [...payments, ...other].sort((a, b) => a - b);
}

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

export const structureAt = (hours: readonly number[], hour: number) => countThrough(hours, hour);

function mergeTips(given: readonly (SpendIdeaTooltips | null)[]): SpendIdeaTooltips | undefined {
  const tips = given.filter((tip): tip is SpendIdeaTooltips => tip !== null);
  if (!tips.length) return undefined;
  const unique = <T,>(values: readonly T[], key: (value: T) => string) => [...new Map(values.map(value => [key(value), value])).values()];
  return { ideas: unique(tips.flatMap(tip => tip.ideas), info => info.token), artwork: unique(tips.flatMap(tip => tip.artwork), art => art.token),
    modifiers: unique(tips.flatMap(tip => tip.modifiers), format => format.key), texts: unique(tips.flatMap(tip => tip.texts), item => item.key) };
}

export const mergedIdeaTooltips = (ledgers: readonly PowerLedger[] | undefined) => mergeTips((ledgers ?? []).map(ledger => ledger.ideaTooltips));
