import type { HourSpan, LedgerSpend, LedgerSpendItem, PoliticsLedgerReply } from '../generated/dashboard';
import type { SunburstItem } from '../charts/sunburst/layout';
import { drawableIcon } from './artwork.ts';
import { NO_SPENDING_CATEGORIES } from './politicsSpendingOptions.ts';
import { contentSet, setToken, storableToken, UNSTORABLE } from './options.ts';
import { validateIdeaTooltipData } from './ideaHistoryReply.ts';
import { dayLabel } from '../charts/draw.ts';
import { gameDate, monthIndex, monthAt, monthNames } from './calendar.ts';
import { MAX_NAME_HOURS, nameKey } from './countryFacts.ts';
import type { NameAt } from './countryFacts';

export type PowerCurrency = 'Political' | 'Command';
export interface SpendAt {
  spend: LedgerSpend;
  item: LedgerSpendItem;
  category: string;
  identified: boolean;
  charge: bigint | null;
  realized: bigint | null;
  recordedCharge: bigint;
  knownCharges: number;
  missingCharges: number;
  occurrences: number;
  readHour: number;
  covered: boolean;
  sequence: bigint;
  aggregateCount?: number;
  days?: number;
  firstHour?: number;
  lastHour?: number;
}
export interface SpendGap { from: number; through: number; readHour: number; reason: string; tag: string }
export interface SpendingWindow { from: number | null; hour: number; end: number }
export interface SpendingSelection {
  matches: (tag: string, hour: number) => boolean;
  membership: ReadonlyMap<string, readonly HourSpan[]> | null;
  unobserved: readonly HourSpan[];
  /** Manual current holdings need a recorded holder; a native gap is not zero. */
  requireRecordedHolder?: boolean;
}
const RAW = /^-?\d{1,39}$/;
const raw = (value: unknown): value is string => typeof value === 'string' && RAW.test(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const object = (value: unknown): value is Record<string, unknown> => value != null && typeof value === 'object';

/** All three spending views validate and use one ledger contract. A missing
 * operand stays missing; drawable art is the shared kit's bounded PNG check. */
export function validatePoliticsSpending(data: unknown): PoliticsLedgerReply {
  const reply = data as PoliticsLedgerReply;
  let ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && integer(reply.decimals) && reply.decimals === 5
    && Array.isArray(reply.warnings) && reply.warnings.every(value => typeof value === 'string')
    && Array.isArray(reply.spending) && Array.isArray(reply.spendItems) && Array.isArray(reply.spendCategories)
    && Array.isArray(reply.spendingCoverage) && Array.isArray(reply.commandHolds);
  if (ok) {
    const items = new Set<string>();
    for (const item of reply.spendItems) {
      if (!object(item) || typeof item.id !== 'string' || !item.id || typeof item.groupId !== 'string' || !item.groupId || items.has(item.id)
        || !text(item.name) || typeof item.action !== 'string' || !text(item.tooltipToken) || !('icon' in item)
        || !text(item.targetTag) || !text(item.targetName) || !text(item.unknown)
        || !(item.character === null || integer(item.character))) { ok = false; break; }
      items.add(item.id);
      item.icon = drawableIcon(item.icon);
    }
    const categories = new Set<string>();
    for (const category of reply.spendCategories) {
      if (!object(category) || typeof category.id !== 'string' || !category.id || categories.has(category.id)
        || typeof category.name !== 'string' || typeof category.identified !== 'boolean') { ok = false; break; }
      categories.add(category.id);
    }
    const ids = new Set<string>();
    for (const spend of reply.spending) {
      if (!object(spend) || typeof spend.id !== 'string' || !spend.id || ids.has(spend.id)
        || !integer(spend.hour) || !integer(spend.readHour) || spend.readHour < spend.hour
        || typeof spend.sequence !== 'string' || !/^\d{1,20}$/.test(spend.sequence) || !integer(spend.ordinal) || spend.ordinal < 0
        || typeof spend.countryTag !== 'string' || !text(spend.countryName)
        || (spend.currency !== 'Political' && spend.currency !== 'Command')
        || !categories.has(spend.category) || !items.has(spend.itemId)
        || !(spend.chargeRaw === null || (raw(spend.chargeRaw) && BigInt(spend.chargeRaw) >= 0n))
        || !(spend.realizedDeltaRaw === null || raw(spend.realizedDeltaRaw)) || typeof spend.covered !== 'boolean'
        || !integer(spend.occurrences) || spend.occurrences < 1 || !Array.isArray(spend.changes) || !spend.changes.length
        || !spend.changes.every((change, index) => object(change) && integer(change.readHour) && change.readHour >= spend.readHour
          && (change.hour == null || (integer(change.hour) && change.hour <= change.readHour))
          && (index === 0 || spend.changes[index - 1].readHour <= change.readHour)
          && (change.chargeRaw === null || (raw(change.chargeRaw) && BigInt(change.chargeRaw) >= 0n))
          && (change.realizedDeltaRaw === null || raw(change.realizedDeltaRaw)) && integer(change.occurrences) && change.occurrences > 0 && typeof change.covered === 'boolean')) { ok = false; break; }
      ids.add(spend.id);
    }
    for (const cover of reply.spendingCoverage) {
      if (!object(cover) || !integer(cover.readHour) || !(cover.sinceHour === null || integer(cover.sinceHour))
        || !(cover.throughHour === null || integer(cover.throughHour)) || typeof cover.political !== 'boolean'
        || typeof cover.command !== 'boolean' || !text(cover.reason)
        || (cover.sinceHour !== null && cover.throughHour !== null && cover.sinceHour > cover.throughHour)) { ok = false; break; }
    }
    let through = -Infinity;
    for (const hold of reply.commandHolds) {
      const holders = new Set<string>();
      if (!object(hold) || !integer(hold.readHour) || !integer(hold.throughHour) || hold.readHour > hold.throughHour || hold.readHour <= through
        || !(hold.totalRaw === null || raw(hold.totalRaw)) || !text(hold.reason) || !Array.isArray(hold.holders)
        || !hold.holders.every(holder => {
          if (!object(holder) || typeof holder.id !== 'string' || !holder.id || holders.has(holder.id)
            || typeof holder.kind !== 'string' || !holder.kind || !text(holder.name)
            || !(holder.amountRaw === null || raw(holder.amountRaw))) return false;
          holders.add(holder.id); return true;
        })) { ok = false; break; }
      through = hold.throughHour;
    }
  }
  if (!ok) throw new Error('The politics spending reply is malformed.');
  if (reply.ideaTooltips !== null) validateIdeaTooltipData(reply.ideaTooltips);
  const tooltipInfos = new Set(reply.ideaTooltips?.ideas.map(info => info.token));
  const tooltipArt = new Set(reply.ideaTooltips?.artwork.map(art => art.token));
  if (reply.spendItems.some(item => item.tooltipToken !== null && (!tooltipInfos.has(item.tooltipToken) || !tooltipArt.has(item.tooltipToken)))) {
    throw new Error('The politics spending reply has an unlisted idea tooltip.');
  }
  if (reply.recurring !== undefined) {
    if (!Array.isArray(reply.recurring) || !reply.recurring.every(row => object(row) && integer(row.firstHour) && integer(row.lastHour)
      && row.firstHour <= row.lastHour && integer(row.days) && row.days >= 0 && integer(row.count) && row.count > 0
      && raw(row.knownRaw) && BigInt(row.knownRaw) >= 0n && integer(row.knownCount) && row.knownCount >= 0 && row.knownCount <= row.spend.occurrences)) {
      throw new Error('The prepared spending totals are malformed.');
    }
    validatePoliticsSpending({ ...reply, recurring: undefined, spending: reply.recurring.map(row => row.spend) });
  }
  if (reply.gains !== undefined && (!Array.isArray(reply.gains) || !reply.gains.every(row => object(row)
    && (row.currency === 'Political' || row.currency === 'Command') && raw(row.gainedRaw) && raw(row.lostRaw) && BigInt(row.lostRaw) >= 0n
    && typeof row.complete === 'boolean'))) throw new Error('The prepared power gains are malformed.');
  if (reply.recurringDays !== undefined && (!Array.isArray(reply.recurringDays) || !reply.recurringDays.every(row => object(row)
    && (row.currency === 'Political' || row.currency === 'Command') && typeof row.category === 'string'
    && (row.month === null || integer(row.month)) && integer(row.days) && row.days >= 0))) throw new Error('The prepared paid days are malformed.');
  return reply;
}

/** Daily upkeep has one row and read-dated increments. Nothing from a later
 * batch appears early, even when its native action hour is earlier. */
export function spendAt(spend: LedgerSpend, item: LedgerSpendItem, category: string, hour: number, identified: boolean,
  from: number | null = null, selection?: SpendingSelection): SpendAt | null {
  if (spend.hour > hour || spend.readHour > hour) return null;
  if (selection && !selection.membership && !selection.matches(spend.countryTag, spend.hour)) return null;
  let charge: bigint | null = 0n;
  let realized: bigint | null = 0n;
  let recordedCharge = 0n;
  let knownCharges = 0;
  let missingCharges = 0;
  let occurrences = 0;
  let readHour = spend.readHour;
  let covered = true;
  let firstHour: number | null = null;
  let selectionUnknown = false;
  for (const change of spend.changes) {
    if (change.readHour > hour) break;
    const nativeHour = change.hour ?? (spend.changes.length === 1 ? spend.hour : null);
    if (nativeHour !== null && ((from !== null && nativeHour < from) || (selection && !selection.matches(spend.countryTag, nativeHour)))) continue;
    if (nativeHour === null && (from !== null || selection?.membership)) {
      selectionUnknown = true; occurrences += change.occurrences; readHour = change.readHour; continue;
    }
    firstHour = Math.min(firstHour ?? Infinity, nativeHour ?? spend.hour);
    charge = charge === null || change.chargeRaw === null ? null : charge + BigInt(change.chargeRaw);
    realized = realized === null || change.realizedDeltaRaw === null ? null : realized + BigInt(change.realizedDeltaRaw);
    if (change.chargeRaw === null) missingCharges += change.occurrences;
    else { recordedCharge += BigInt(change.chargeRaw); knownCharges += change.occurrences; }
    occurrences += change.occurrences;
    readHour = change.readHour;
    covered &&= change.covered;
  }
  if (selectionUnknown) {
    charge = null; realized = null; covered = false;
    item = { ...item, unknown: [item.unknown, 'Some payment dates are unavailable.'].filter(Boolean).join(' ') };
  }
  const dated = firstHour !== null && firstHour !== spend.hour ? { ...spend, hour: firstHour, countryName: null } : spend;
  return occurrences ? { spend: dated, item, category, identified, charge, realized, recordedCharge, knownCharges, missingCharges, occurrences, readHour, covered,
    sequence: BigInt(spend.sequence) } : null;
}

export function spendingAt(replies: readonly PoliticsLedgerReply[], currency: PowerCurrency, hour: number, from: number | null = null, selection?: SpendingSelection): SpendAt[] {
  const result: SpendAt[] = [];
  for (const reply of replies) {
    const items = new Map(reply.spendItems.map(item => [item.id, item]));
    const categories = new Map(reply.spendCategories.map(category => [category.id, category]));
    const artwork = new Map(reply.ideaTooltips?.artwork.map(art => [art.token, art.icon]));
    for (const spend of reply.spending) {
      if (spend.currency !== currency) continue;
      const item = items.get(spend.itemId);
      if (!item) continue; // Validated replies always have their item.
      const category = categories.get(spend.category);
      const shownItem = item.tooltipToken ? { ...item, icon: artwork.get(item.tooltipToken) ?? item.icon } : item;
      const at = spendAt(spend, shownItem, category?.name ?? 'Not identified', hour, category?.identified ?? false, from, selection);
      if (at) result.push(at);
    }
    for (const row of reply.recurring ?? []) {
      const spend = row.spend;
      if (spend.currency !== currency || spend.hour > hour || spend.readHour > hour) continue;
      const item = items.get(spend.itemId)!;
      const category = categories.get(spend.category)!;
      const shownItem = item.tooltipToken ? { ...item, icon: artwork.get(item.tooltipToken) ?? item.icon } : item;
      result.push({ spend, item: shownItem, category: category.name, identified: category.identified,
        charge: spend.chargeRaw === null ? null : BigInt(spend.chargeRaw), realized: spend.realizedDeltaRaw === null ? null : BigInt(spend.realizedDeltaRaw),
        recordedCharge: BigInt(row.knownRaw), knownCharges: row.knownCount, missingCharges: spend.occurrences - row.knownCount,
        occurrences: spend.occurrences, readHour: spend.readHour, covered: spend.covered, sequence: BigInt(spend.sequence),
        aggregateCount: row.count, days: row.days, firstHour: row.firstHour, lastHour: row.lastHour });
    }
  }
  return result;
}

/** Product category ids of costs the game charges every day: a daily cost
 * modifier, organisation upkeep, ability upkeep and a command power deficit.
 * Tables fold them into one row a month; balance markers leave them out. */
export const RECURRING_CATEGORIES: ReadonlySet<string> = new Set(['dailyCosts', 'organisationUpkeep', 'upkeep', 'dailyDeficit']);

/** The product's wording for a name or target it could not resolve. Presentation
 * never shows it: the payment is named by its kind instead. */
const UNRESOLVED = 'Not identified';

/** A category's shown name; one with no recorded cause is plain “Other spending”. */
export const categoryLabel = (name: string, identified: boolean) => identified ? name : 'Other spending';

/** The recorded action, or the kind of payment when its name or target wasn't recorded. */
export function spendLabel(item: LedgerSpendItem, category: string): string {
  if (!item.action.includes(UNRESOLVED)) return item.action;
  return item.name ?? category;
}

/** Payments whose cost wasn't recorded. */
export const unknownCosts = (at: SpendAt) => at.charge === null ? at.occurrences - at.knownCharges : 0;

/** Unrecorded stretches, joined when they touch whatever the reason. No empty
 * batch proves that no power was spent, so each is named, never counted as zero. */
export function spendingGaps(replies: readonly PoliticsLedgerReply[], currency: PowerCurrency, window: Pick<SpendingWindow, 'from' | 'hour'>, selection?: SpendingSelection): SpendGap[] {
  const gaps: SpendGap[] = [];
  for (const reply of replies) {
    let last: SpendGap | undefined;
    for (const cover of reply.spendingCoverage) {
      if (cover.readHour > window.hour) continue;
      const from = cover.sinceHour ?? cover.readHour;
      const through = cover.throughHour ?? cover.readHour;
      if (through < (window.from ?? -Infinity) || from > window.hour) continue;
      if (currency === 'Political' ? cover.political : cover.command) { last = undefined; continue; }
      if (last && from <= last.through + 1) {
        last.through = Math.min(window.hour, Math.max(last.through, through));
        last.readHour = Math.max(last.readHour, cover.readHour);
      } else {
        last = { from: Math.max(from, window.from ?? from), through: Math.min(through, window.hour), readHour: cover.readHour,
          reason: cover.reason ?? 'Spending not recorded', tag: reply.tag };
        gaps.push(last);
      }
    }
    if (!reply.spendingCoverage.some(cover => cover.readHour <= window.hour)) gaps.push({ from: window.from ?? window.hour,
      through: window.hour, readHour: window.hour, reason: 'Spending not recorded', tag: reply.tag });
  }
  if (!selection?.membership) return gaps;
  const selected = gaps.flatMap(gap => (selection.membership!.get(gap.tag) ?? []).flatMap(span => {
    const from = Math.max(gap.from, span.firstHour), through = Math.min(gap.through, span.lastHour);
    return from <= through ? [{ ...gap, from, through }] : [];
  }));
  for (const span of selection.unobserved) {
    const from = Math.max(span.firstHour, window.from ?? span.firstHour), through = Math.min(span.lastHour, window.hour);
    if (from <= through) selected.push({ from, through, readHour: from, tag: '', reason: 'Group membership was not recorded; spending for this group is not known.' });
  }
  return selected;
}

const plural = (count: number, one: string) => `${count.toLocaleString('en-US')} ${one}${count === 1 ? '' : 's'}`;
const duration = (hours: number) => hours < 48 ? plural(hours, 'hour') : plural(Math.floor(hours / 24), 'day');

/** One plain sentence naming unrecorded time: how long and on which dates. Equal
 * stretches of several countries count once. */
export function spendingGapNote(gaps: readonly SpendGap[]): string | null {
  const stretches = [...new Map(gaps.map(gap => [`${gap.from}:${gap.through}`, gap])).values()].sort((a, b) => a.from - b.from);
  if (!stretches.length) return null;
  const hours = stretches.reduce((sum, gap) => sum + gap.through - gap.from + 1, 0);
  const days = [...new Set(stretches.map(gap => dayLabel(gap.from)))];
  const dates = days.length > 3 ? `${days.slice(0, 3).join(', ')} and ${plural(days.length - 3, 'other date')}` : days.join(', ');
  return `Spending wasn’t recorded for ${duration(hours)} (${dates}); anything spent then isn’t counted.`;
}

export interface SpendingSummary { spent: bigint; unknownCosts: number; uncovered: boolean }
/** The exact sum of recorded costs. A cost that wasn't recorded is counted, not guessed. */
export function spendingSummary(spends: readonly SpendAt[]): SpendingSummary {
  let spent = 0n, unknown = 0, uncovered = false;
  for (const at of spends) {
    spent += at.recordedCharge;
    unknown += unknownCosts(at);
    uncovered ||= !at.covered && at.charge !== null;
  }
  return { spent, unknownCosts: unknown, uncovered };
}

/** Notes for costs counted without their amount, in plain words. */
export function spendingCostNotes(summary: SpendingSummary): string[] {
  return [
    ...(summary.unknownCosts ? [`The cost of ${plural(summary.unknownCosts, 'payment')} wasn’t recorded.`] : []),
    ...(summary.uncovered ? ['Some payments may be missing.'] : []),
  ];
}

export interface SpendingTableRow {
  key: string;
  tag: string;
  /** The payment's native hour; a folded month shows its latest payment. */
  hour: number;
  /** The game month a folded recurring row covers. */
  month: number | null;
  label: string;
  icon: string | null;
  category: string;
  categoryName: string;
  identified: boolean;
  /** Days a folded row covers; 0 for a single payment. */
  days: number;
  /** Exact recorded cost; null when none of its costs were recorded. */
  amount: bigint | null;
  partial: boolean;
  /** Native order: hour, then action sequence, then ordinal, exactly. */
  order: bigint;
  parts: readonly SpendAt[];
}

const ORDER_HOUR = 1n << 80n;
const nativeOrder = (at: SpendAt) => BigInt(at.spend.hour) * ORDER_HOUR + (at.sequence << 16n) + BigInt(at.spend.ordinal);
const monthOf = (hour: number) => { const date = gameDate(hour); return date ? monthIndex(date) : null; };
export const monthLabel = (month: number) => { const { year, month: number } = monthAt(month); return `${monthNames[number - 1].slice(0, 3)} ${year}`; };

type Folding = SpendingTableRow & { dayIndexes: Set<number>; partsList: SpendAt[]; missing: number };
/** One row per payment, with each country's recurring costs folded into one row
 * per category and game month. Category filters never change an amount. */
export function spendingTableRows(spends: readonly SpendAt[], categories: string): SpendingTableRow[] {
  const chosen = new Set(contentSet(categories).filter(value => value !== NO_SPENDING_CATEGORIES));
  const rows: SpendingTableRow[] = [];
  const months = new Map<string, Folding>();
  for (const at of spends) {
    if (chosen.size && !chosen.has(setToken(at.spend.category))) continue;
    const categoryName = categoryLabel(at.category, at.identified);
    const missing = unknownCosts(at);
    const month = RECURRING_CATEGORIES.has(at.spend.category) ? monthOf(at.spend.hour) : null;
    if (month === null) {
      rows.push({ key: JSON.stringify([at.spend.countryTag, at.spend.id]), tag: at.spend.countryTag, hour: at.spend.hour, month: null,
        label: spendLabel(at.item, categoryName), icon: at.item.icon, category: at.spend.category, categoryName, identified: at.identified, days: 0,
        amount: missing && !at.knownCharges ? null : at.recordedCharge, partial: missing > 0 || !at.covered, order: nativeOrder(at), parts: [at] });
      continue;
    }
    const key = JSON.stringify(['month', at.spend.countryTag, at.spend.category, month]);
    let row = months.get(key);
    if (!row) {
      row = { key, tag: at.spend.countryTag, hour: at.spend.hour, month, label: categoryName, icon: null, category: at.spend.category, categoryName,
        identified: at.identified, days: 0, amount: 0n, partial: false, order: nativeOrder(at), parts: [], dayIndexes: new Set(), partsList: [], missing: 0 };
      months.set(key, row);
    }
    row.partsList.push(at);
    row.dayIndexes.add(Math.floor(at.spend.hour / 24));
    row.amount = row.amount! + at.recordedCharge;
    row.missing += missing;
    row.partial ||= missing > 0 || !at.covered;
    if (nativeOrder(at) > row.order) { row.order = nativeOrder(at); row.hour = at.spend.hour; }
  }
  for (const { dayIndexes, partsList, missing, ...row } of months.values()) {
    rows.push({ ...row, days: dayIndexes.size, parts: partsList, amount: missing && partsList.every(at => !at.knownCharges) ? null : row.amount });
  }
  return rows;
}

/** Prepared item rows already contain the month's subtotal. This joins them
 * into the table's category rows without enumerating paid days. */
export function preparedSpendingTableRows(replies: readonly PoliticsLedgerReply[], spends: readonly SpendAt[], categories: string): SpendingTableRow[] {
  const chosen = new Set(contentSet(categories).filter(value => value !== NO_SPENDING_CATEGORIES));
  const rows = spendingTableRows(spends.filter(at => at.aggregateCount === undefined), categories);
  const months = new Map<string, SpendingTableRow>();
  for (const at of spends.filter(at => at.aggregateCount !== undefined)) {
    if (chosen.size && !chosen.has(setToken(at.spend.category))) continue;
    const categoryName = categoryLabel(at.category, at.identified);
    const month = monthOf(at.spend.hour);
    if (month === null || !RECURRING_CATEGORIES.has(at.spend.category)) {
      rows.push({ key: JSON.stringify([at.spend.countryTag, at.spend.id]), tag: at.spend.countryTag, hour: at.spend.hour, month: null,
        label: spendLabel(at.item, categoryName), icon: at.item.icon, category: at.spend.category, categoryName, identified: at.identified,
        days: 0, amount: at.knownCharges ? at.recordedCharge : null, partial: unknownCosts(at) > 0 || !at.covered, order: nativeOrder(at), parts: [at] });
      continue;
    }
    const key = JSON.stringify(['month', at.spend.countryTag, at.spend.category, month]);
    const old = months.get(key);
    const days = replies.find(reply => reply.tag === at.spend.countryTag)?.recurringDays?.find(row => row.currency === at.spend.currency
      && row.category === at.spend.category && row.month === month)?.days ?? at.days ?? 0;
    if (!old) months.set(key, { key, tag: at.spend.countryTag, hour: at.spend.hour, month, label: categoryName, icon: null,
      category: at.spend.category, categoryName, identified: at.identified, days, amount: at.knownCharges ? at.recordedCharge : null,
      partial: unknownCosts(at) > 0 || !at.covered, order: nativeOrder(at), parts: [at] });
    else {
      old.amount = old.amount === null && !at.knownCharges ? null : (old.amount ?? 0n) + at.recordedCharge;
      old.partial ||= unknownCosts(at) > 0 || !at.covered;
      old.parts = [...old.parts, at];
      if (nativeOrder(at) > old.order) { old.order = nativeOrder(at); old.hour = at.spend.hour; }
    }
  }
  return [...rows, ...months.values()];
}

/** The names and flags mounted rows show: each country at its row's date when
 * several are compared, and each payment's target at its own action hour. Every
 * distinct date is asked for, so the newest rows keep their names past the
 * default hour limit. */
export function spendingCountryRequests(drawn: readonly SpendingTableRow[], compare: boolean): { wanted: NameAt[]; maxHours: number } {
  const names = new Map<string, NameAt>();
  for (const row of drawn) {
    if (compare) names.set(nameKey(row.hour, row.tag), { hour: row.hour, tag: row.tag });
    for (const at of row.month === null ? row.parts : []) {
      if (at.item.targetTag != null) names.set(nameKey(at.spend.hour, at.item.targetTag), { hour: at.spend.hour, tag: at.item.targetTag });
    }
  }
  const wanted = [...names.values()];
  return { wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
}

/** Grouped content tooltips retain the exact known subtotal even when a missing
 * operand makes the whole unknown. Effects are dated to the latest matched
 * native action, named explicitly by the tooltip. */
export function spendingGroups(spends: readonly SpendAt[]) {
  const groups = new Map<string, { latest: SpendAt; recordedCharge: bigint; knownCharges: number; missingCharges: number; count: number; rows: number }>();
  for (const at of spends) {
    const previous = groups.get(at.item.groupId);
    const later = !previous || previous.latest.spend.hour < at.spend.hour || (previous.latest.spend.hour === at.spend.hour
      && (previous.latest.sequence < at.sequence
        || (previous.latest.sequence === at.sequence && previous.latest.spend.ordinal < at.spend.ordinal)));
    groups.set(at.item.groupId, { latest: later ? at : previous!.latest,
      recordedCharge: (previous?.recordedCharge ?? 0n) + at.recordedCharge,
      knownCharges: (previous?.knownCharges ?? 0) + at.knownCharges,
      missingCharges: (previous?.missingCharges ?? 0) + at.missingCharges,
      count: (previous?.count ?? 0) + at.occurrences, rows: (previous?.rows ?? 0) + (at.aggregateCount ?? 1) });
  }
  return groups;
}

/** Category → recorded item, retaining exact fixed5 amounts. Everything recorded
 * draws: a cost that wasn't recorded marks its slice partial, and unrecorded time
 * is named beside the chart, so the whole is the recorded spending. A payment
 * with no recorded name sits in its category without an item slice. */
export function spendingItems(replies: readonly PoliticsLedgerReply[], currency: PowerCurrency, window: SpendingWindow, selection?: SpendingSelection, finalReplies = replies): SunburstItem[] {
  const now = spendingGroups(spendingAt(replies, currency, window.hour, window.from, selection));
  const final = spendingGroups(spendingAt(finalReplies, currency, window.end, window.from, selection));
  return [...final].map(([key, group]) => {
    const shown = now.get(key);
    const at = group.latest;
    const partial = !!shown && (shown.missingCharges > 0 || !shown.latest.covered);
    const category = { id: at.spend.category, label: categoryLabel(at.category, at.identified) };
    const path = at.item.name === null ? [category] : [category, { id: at.item.groupId, label: at.item.name, icon: at.item.icon }];
    return { path, value: Number(shown?.recordedCharge ?? 0n), exact: shown?.recordedCharge ?? 0n,
      present: !!shown, final: Number(group.recordedCharge), count: shown?.rows ?? 0,
      unknown: partial ? { incomplete: 1 } : undefined, note: partial ? 'Some costs weren’t recorded.' : undefined };
  });
}

/** Category chips: every category spent on in this currency, by its shown name. */
export function categoryChoices(replies: readonly PoliticsLedgerReply[] | undefined, currency: PowerCurrency) {
  const used = new Set((replies ?? []).flatMap(reply => [...reply.spending, ...(reply.recurring ?? []).map(row => row.spend)].filter(spend => spend.currency === currency).map(spend => spend.category)));
  const categories = new Map((replies ?? []).flatMap(reply => reply.spendCategories.map(category => [category.id, category] as const)));
  return [...categories.values()].filter(category => used.has(category.id)).map(category => {
    const value = setToken(category.id);
    return { value, label: categoryLabel(category.name, category.identified), identified: category.identified,
      unavailable: storableToken(value) ? undefined : UNSTORABLE };
  });
}
