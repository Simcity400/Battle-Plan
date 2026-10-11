import type { HourSpan, LedgerCommandHold, LedgerSpend, LedgerSpendItem, PoliticsLedgerReply } from '../generated/dashboard';
import type { LogRow } from '../charts/log/logData';
import type { SunburstItem } from '../charts/sunburst/layout';
import { drawableIcon } from './artwork.ts';
import { contentSet, setToken, storableToken, UNSTORABLE } from './options.ts';
import { fixed5Text } from './ideaTooltip.ts';
import { validateIdeaTooltipData } from './ideaHistoryReply.ts';
import { compareTags } from './tagOrder.ts';
import { runAt } from '../charts/series.ts';
import type { TooltipContent } from '../charts/tooltip';
import { formatDateTime } from './calendar.ts';

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
}
export interface SpendGap { from: number; through: number; readHour: number; reason: string; tag: string }
export type SpendingRow = { kind: 'spend'; at: SpendAt } | { kind: 'gap'; gap: SpendGap };
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
    item = { ...item, unknown: [item.unknown, 'Native payment hours were not recorded; the selected membership or date range is not known for every contribution.'].filter(Boolean).join(' ') };
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
  }
  return result;
}

export interface CommandHeldAt { hour: number; total: bigint | null; countries: readonly { tag: string; hold: LedgerCommandHold | null }[]; reason?: string }
const heldRuns = new WeakMap<PoliticsLedgerReply, { firstHour: number; lastHour: number; hold: LedgerCommandHold }[]>();

/** Allocation state applies only in the hours proved by its recorded run. A
 * missing run is unknown, including before the first read and across gaps. */
export function commandHeldAt(replies: readonly PoliticsLedgerReply[], hour: number, selection?: SpendingSelection): CommandHeldAt {
  if (selection?.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour)) {
    return { hour, total: null, countries: [], reason: 'Group membership was not recorded at this hour; current holdings are not known.' };
  }
  let total: bigint | null = 0n;
  const countries = replies.filter(reply => !selection || selection.matches(reply.tag, hour)).map(reply => {
    let runs = heldRuns.get(reply);
    if (!runs) {
      runs = reply.commandHolds.map(hold => ({ firstHour: hold.readHour, lastHour: hold.throughHour, hold }));
      heldRuns.set(reply, runs);
    }
    const hold = runAt(runs, hour)?.hold ?? null;
    total = total === null || hold?.totalRaw == null ? null : total + BigInt(hold.totalRaw);
    return { tag: reply.tag, hold };
  });
  if (!countries.length && selection?.requireRecordedHolder) {
    return { hour, total: null, countries, reason: 'No selected native country holder was recorded at this hour; current holdings are not known.' };
  }
  return { hour, total, countries };
}

export function commandHeldTooltip(held: CommandHeldAt): TooltipContent {
  const rows: TooltipContent['rows'][number][] = [];
  if (held.reason) rows.push({ label: held.reason, tone: 'muted' });
  for (const { tag, hold } of held.countries) {
    if (held.countries.length > 1) rows.push({ label: tag, strong: true, divider: rows.length > 0 });
    if (!hold) { rows.push({ label: 'Holdings were not recorded at this hour.', tone: 'muted' }); continue; }
    rows.push({ label: 'Read', value: formatDateTime(hold.readHour) });
    rows.push({ label: 'Recorded through', value: formatDateTime(Math.min(held.hour, hold.throughHour)) });
    for (const holder of hold.holders) rows.push({ label: `${holder.kind}: ${holder.name ?? 'Not identified'}`, value: spendText(holder.amountRaw === null ? null : BigInt(holder.amountRaw)) });
    if (hold.reason) rows.push({ label: hold.reason, tone: 'muted' });
  }
  rows.push({ label: 'Total held', value: spendText(held.total), strong: true, divider: rows.length > 0 });
  return { title: 'Command power held', subtitle: `Current moment ${formatDateTime(held.hour)}`, rows,
    note: 'Held command power is reserved and returned on release. It does not add to spending.' };
}

/** Named gaps are rows too. Consecutive unavailable reads with the same reason
 * are one stretch; no empty batch proves that no power was spent. */
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
      const reason = cover.reason ?? 'Spending coverage is incomplete';
      if (last && last.reason === reason && from <= last.through + 1) {
        last.through = Math.min(window.hour, Math.max(last.through, through));
        last.readHour = Math.max(last.readHour, cover.readHour);
      } else {
        last = { from: Math.max(from, window.from ?? from), through: Math.min(through, window.hour), readHour: cover.readHour, reason, tag: reply.tag };
        gaps.push(last);
      }
    }
    if (!reply.spendingCoverage.some(cover => cover.readHour <= window.hour)) gaps.push({ from: window.from ?? window.hour,
      through: window.hour, readHour: window.hour, reason: 'Spending coverage was not recorded', tag: reply.tag });
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

export type SpendOrder = 'newest' | 'oldest' | 'cost';
export function spendingRows(spends: readonly SpendAt[], gaps: readonly SpendGap[], categories: string, order: SpendOrder, selection: 'include' | 'exclude' = 'exclude'): LogRow<SpendingRow>[] {
  const chosen = new Set(contentSet(categories));
  const rows: LogRow<SpendingRow>[] = spends.filter(at => selection === 'include' ? chosen.has(setToken(at.spend.category)) : !chosen.has(setToken(at.spend.category)))
    .map(at => ({ key: JSON.stringify([at.spend.countryTag, at.spend.id]), hour: at.spend.hour, item: { kind: 'spend', at } }));
  for (const gap of gaps) rows.push({ key: JSON.stringify(['gap', gap.tag, gap.from]), hour: gap.from, item: { kind: 'gap', gap } });
  return rows.sort((a, b) => {
    if (order === 'cost') {
      const av = a.item.kind === 'spend' ? a.item.at.charge : null;
      const bv = b.item.kind === 'spend' ? b.item.at.charge : null;
      if (av !== bv) return av === null ? 1 : bv === null ? -1 : av > bv ? -1 : 1;
    }
    const hour = order === 'oldest' ? a.hour - b.hour : b.hour - a.hour;
    if (hour) return hour;
    if (a.item.kind === 'spend' && b.item.kind === 'spend') {
      const av = a.item.at.sequence;
      const bv = b.item.at.sequence;
      if (av !== bv) return (av < bv ? -1 : 1) * (order === 'oldest' ? 1 : -1);
      const ordinal = a.item.at.spend.ordinal - b.item.at.spend.ordinal;
      if (ordinal) return order === 'oldest' ? ordinal : -ordinal;
    }
    return compareTags(a.key, b.key);
  });
}

export function spendingTotals(spends: readonly SpendAt[], gaps: readonly SpendGap[]) {
  const categories = new Map<string, { name: string; value: bigint | null }>();
  let total: bigint | null = gaps.length ? null : 0n;
  for (const at of spends) {
    const known = at.covered ? at.charge : null;
    total = total === null || known === null ? null : total + known;
    const old = categories.get(setToken(at.spend.category));
    categories.set(setToken(at.spend.category), { name: at.category, value: gaps.length || known === null || old?.value === null ? null : (old?.value ?? 0n) + known });
  }
  return { total, categories, categoryTotal: (id: string): bigint | null => {
    const entry = categories.get(id);
    return entry ? entry.value : gaps.length ? null : 0n;
  } };
}

/** Grouped content tooltips retain the exact known subtotal even when a missing
 * operand makes the whole unknown. Effects are dated to the latest matched
 * native action, named explicitly by the tooltip. */
export function spendingGroups(spends: readonly SpendAt[]) {
  const groups = new Map<string, { latest: SpendAt; recordedCharge: bigint; knownCharges: number; missingCharges: number }>();
  for (const at of spends) {
    const previous = groups.get(at.item.groupId);
    const later = !previous || previous.latest.spend.hour < at.spend.hour || (previous.latest.spend.hour === at.spend.hour
      && (previous.latest.sequence < at.sequence
        || (previous.latest.sequence === at.sequence && previous.latest.spend.ordinal < at.spend.ordinal)));
    groups.set(at.item.groupId, { latest: later ? at : previous!.latest,
      recordedCharge: (previous?.recordedCharge ?? 0n) + at.recordedCharge,
      knownCharges: (previous?.knownCharges ?? 0) + at.knownCharges,
      missingCharges: (previous?.missingCharges ?? 0) + at.missingCharges });
  }
  return groups;
}

/** Category → recorded item, retaining exact fixed5 amounts. A coverage gap
 * makes the whole unknown and cannot supply Fill's final denominator. */
export function spendingItems(replies: readonly PoliticsLedgerReply[], currency: PowerCurrency, window: SpendingWindow, selection?: SpendingSelection): SunburstItem[] {
  const shown = new Map(spendingAt(replies, currency, window.hour, window.from, selection).map(at => [JSON.stringify([at.spend.countryTag, at.spend.id]), at]));
  const final = spendingAt(replies, currency, window.end, window.from, selection);
  const items = final.map<SunburstItem>(at => {
    const key = JSON.stringify([at.spend.countryTag, at.spend.id]);
    const now = shown.get(key);
    const unknown = Boolean(now && (now.charge === null || !now.covered));
    return { path: [{ id: at.spend.category, label: at.category }, { id: at.item.groupId, label: at.item.name ?? 'Not identified', icon: at.item.icon }],
      value: now ? Number(now.recordedCharge) : 0, exact: now ? now.recordedCharge : 0n, present: Boolean(now),
      unknownValue: unknown, final: Number(at.recordedCharge), finalUnknown: at.charge === null || !at.covered,
      unknown: unknown ? { incomplete: 1 } : undefined, note: unknown ? at.item.unknown ?? 'The evaluated charge or coverage is not known.' : undefined,
      counted: now ? [key] : [] };
  });
  const gaps = spendingGaps(replies, currency, window, selection);
  const finalGaps = spendingGaps(replies, currency, { from: window.from, hour: window.end }, selection);
  if (gaps.length || finalGaps.length) items.push({ path: [{ id: 'coverage-gap', label: 'Hours not recorded' }],
    value: null, exact: null, present: gaps.length > 0, unknownValue: gaps.length > 0, final: null, finalUnknown: finalGaps.length > 0,
    unknown: gaps.length ? { coverage: gaps.length } : {}, note: 'Spending in gaps is not known; recorded spends are kept.' });
  return items;
}

export const spendText = (value: bigint | null) => value === null ? 'Not known' : fixed5Text(value, -1);
export function categoryChoices(replies: readonly PoliticsLedgerReply[] | undefined, currency: PowerCurrency) {
  const used = new Set((replies ?? []).flatMap(reply => reply.spending.filter(spend => spend.currency === currency).map(spend => spend.category)));
  const categories = new Map((replies ?? []).flatMap(reply => reply.spendCategories.map(category => [category.id, category] as const)));
  return [...categories.values()].filter(category => used.has(category.id)).map(category => {
    const value = setToken(category.id);
    return { value, label: category.name, identified: category.identified, unavailable: storableToken(value) ? undefined : UNSTORABLE };
  });
}
