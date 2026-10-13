import type { BalanceDrop, LedgerBalanceChange, LedgerDaily, LedgerSpend, LedgerSpendCoverage, LedgerSpendItem, PoliticsLedgerReply } from '../generated/dashboard';
import type { TooltipRow } from '../charts/tooltip';
import { runAt } from '../charts/series.ts';
import { categoryLabel, RECURRING_CATEGORIES, spendAt, spendLabel, validatePoliticsSpending } from './politicsSpending.ts';
import type { PowerCurrency, SpendingSelection, SpendingWindow } from './politicsSpending';
import { formatPowerChange, formatRate, formatSpent } from './powerDecimals.ts';

const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,39}$/.test(value)
  && BigInt(value) >= -(1n << 127n) && BigInt(value) < (1n << 127n);
const optionalRaw = (value: unknown) => value === null || raw(value);

/** The balance view adds signed daily operands and applications to the shared
 * spending contract; names and prices still come from that same native scan. */
export function validatePowerBalance(data: unknown): PoliticsLedgerReply {
  const reply = validatePoliticsSpending(data);
  if ((reply.daily !== undefined && (!Array.isArray(reply.daily) || !reply.daily.every(day => object(day)
    && integer(day.country) && integer(day.instance) && typeof day.epoch === 'string'
    && integer(day.dayHour) && integer(day.readHour) && day.readHour >= day.dayHour
    && [day.ppReadHour, day.cpReadHour].every(hour => hour == null || (integer(hour) && hour >= day.dayHour && hour <= day.readHour))
    && [day.ppGrossRaw, day.ppCostRaw, day.ppIncomeRaw, day.ppRealizedRaw, day.cpIncomeRaw, day.cpRealizedRaw].every(optionalRaw)
    && [day.ppSequence, day.cpSequence].every(value => value === null || typeof value === 'string'))))
    || (reply.balanceChanges !== undefined && (!Array.isArray(reply.balanceChanges) || !reply.balanceChanges.every(change => object(change)
      && integer(change.hour) && integer(change.readHour) && change.readHour >= change.hour
      && (change.currency === 'Political' || change.currency === 'Command') && optionalRaw(change.amountRaw)
      && raw(change.realizedDeltaRaw) && typeof change.action === 'string' && typeof change.covered === 'boolean'
      && (change.reason === null || typeof change.reason === 'string'))))) {
    throw new Error('The power balance details are malformed.');
  }
  return reply;
}

export interface PowerBalanceIndex {
  reply: PoliticsLedgerReply;
  currency: PowerCurrency;
  days: ReadonlyMap<number, readonly LedgerDaily[]>;
  changes: ReadonlyMap<number, readonly LedgerBalanceChange[]>;
  spends: ReadonlyMap<number, readonly LedgerSpend[]>;
  items: ReadonlyMap<string, LedgerSpendItem>;
  categories: ReadonlyMap<string, { name: string; identified: boolean }>;
  coverage: readonly { firstHour: number; lastHour: number; value: LedgerSpendCoverage }[];
}
const dayAt = (hour: number) => Math.floor(hour / 24);
const indexes = new WeakMap<PoliticsLedgerReply, Map<PowerCurrency, PowerBalanceIndex>>();
function byDay<T>(rows: readonly T[], hour: (row: T) => number): Map<number, T[]> {
  const result = new Map<number, T[]>();
  for (const row of rows) {
    const day = dayAt(hour(row));
    const list = result.get(day) ?? [];
    list.push(row); result.set(day, list);
  }
  return result;
}

export function powerBalanceIndex(reply: PoliticsLedgerReply, currency: PowerCurrency): PowerBalanceIndex {
  let held = indexes.get(reply);
  if (!held) { held = new Map(); indexes.set(reply, held); }
  let index = held.get(currency);
  if (!index) {
    index = { reply, currency, days: byDay(reply.daily ?? [], day => day.dayHour),
      changes: byDay((reply.balanceChanges ?? []).filter(change => change.currency === currency), change => change.hour),
      spends: byDay(reply.spending.filter(spend => spend.currency === currency && spend.category !== 'dailyDeficit'), spend => spend.hour),
      items: new Map(reply.spendItems.map(item => [item.id, item])),
      categories: new Map(reply.spendCategories.map(category => [category.id, category])),
      coverage: reply.spendingCoverage.map(value => ({ firstHour: value.sinceHour ?? value.readHour,
        lastHour: value.throughHour ?? value.readHour, value })).sort((a, b) => a.firstHour - b.firstHour) };
    held.set(currency, index);
  }
  return index;
}

export function powerCoverageAt(index: PowerBalanceIndex, hour: number): LedgerSpendCoverage | null {
  const cover = runAt(index.coverage, hour)?.value;
  return cover && cover.readHour <= hour ? cover : null;
}
/** PP and CP daily tuples are published independently; an older payload has
 * only its conservative whole-row supplying date. */
export function powerDailyReadHour(day: LedgerDaily, currency: PowerCurrency): number | null {
  const supplied = currency === 'Political'
    ? [day.ppGrossRaw, day.ppCostRaw, day.ppIncomeRaw, day.ppRealizedRaw, day.ppSequence]
    : [day.cpIncomeRaw, day.cpRealizedRaw, day.cpSequence];
  return supplied.every(value => value === null) ? null
    : (currency === 'Political' ? day.ppReadHour : day.cpReadHour) ?? day.readHour;
}
/** Keyboard hover can reach details that changed without moving the balance. */
export function powerBalanceKeyHours(index: PowerBalanceIndex): number[] {
  return [...new Set([
    ...(index.reply.daily ?? []).flatMap(day => {
      const read = powerDailyReadHour(day, index.currency);
      return read === null ? [] : [read];
    }),
    ...(index.reply.balanceChanges ?? []).filter(change => change.currency === index.currency).map(change => change.readHour),
    ...index.reply.spending.filter(spend => spend.currency === index.currency).flatMap(spend => spend.changes.map(change => change.readHour)),
  ])].sort((a, b) => a - b);
}
const coveredAt = (index: PowerBalanceIndex, hour: number) => {
  const cover = powerCoverageAt(index, hour);
  return index.currency === 'Political' ? cover?.political === true : cover?.command === true;
};

/** Native spend ticks mark one-off payments, including one masked by that hour's
 * income or clamp; costs charged every day would mark every day, so they don't.
 * Old or incomplete capture keeps the explicitly labelled net-drop fallback. */
export function powerSpendTicks(index: PowerBalanceIndex | undefined, drops: readonly BalanceDrop[], through: number): { hour: number }[] {
  const hours = new Set<number>();
  if (index) for (const spends of index.spends.values()) for (const spend of spends) {
    if (RECURRING_CATEGORIES.has(spend.category)) continue;
    if (spend.hour <= through && spend.changes.some(change => change.readHour <= through
      && (change.chargeRaw === null || BigInt(change.chargeRaw) > 0n))) hours.add(spend.hour);
  }
  for (const drop of drops) if (drop.hour <= through && (!index || !coveredAt(index, drop.hour))) hours.add(drop.hour);
  return [...hours].sort((a, b) => a - b).map(hour => ({ hour }));
}

const big = (value: string | null | undefined) => value == null ? null : BigInt(value);

/** A day's native figures: gain before daily costs, the costs (political power
 * only), the net change and the change actually applied. */
export interface DailyFigures { gain: bigint | null; costs: bigint | null; net: bigint | null; applied: bigint | null }
export function dailyFigures(day: LedgerDaily, currency: PowerCurrency): DailyFigures {
  return currency === 'Political'
    ? { gain: big(day.ppGrossRaw), costs: big(day.ppCostRaw), net: big(day.ppIncomeRaw), applied: big(day.ppRealizedRaw) }
    : { gain: big(day.cpIncomeRaw), costs: null, net: big(day.cpIncomeRaw), applied: big(day.cpRealizedRaw) };
}

/** The gain the cap took: a positive net gain the game applied only in part. */
export function lostAtCap(figures: DailyFigures): bigint {
  const { net, applied } = figures;
  return net !== null && applied !== null && net > 0n && applied < net ? net - (applied > 0n ? applied : 0n) : 0n;
}

export interface PowerGain { gained: bigint; lost: bigint; complete: boolean }
/** Daily gain over the shown period, as the game adds it each day: before daily
 * costs (which are spending), and never a command power deficit (also spending).
 * `lost` is gain the cap took. A day whose gain wasn't read leaves it incomplete;
 * null when the reply has no daily figures in the period. */
export function powerGain(replies: readonly PoliticsLedgerReply[], currency: PowerCurrency, window: Pick<SpendingWindow, 'from' | 'hour'>,
  selection?: Pick<SpendingSelection, 'matches'>): PowerGain | null {
  let gained = 0n, lost = 0n, complete = true, any = false;
  for (const reply of replies) for (const day of reply.daily ?? []) {
    if (day.dayHour > window.hour || (window.from !== null && day.dayHour < window.from)) continue;
    if (selection && !selection.matches(reply.tag, day.dayHour)) continue;
    const read = powerDailyReadHour(day, currency);
    // A day read without this currency's figures has a gain that wasn't read.
    if (read === null) { if (day.readHour <= window.hour) complete = false; continue; }
    if (read > window.hour) continue;
    any = true;
    const figures = dailyFigures(day, currency);
    if (figures.gain === null) complete = false;
    else if (figures.gain > 0n || currency === 'Political') gained += figures.gain;
    lost += lostAtCap(figures);
  }
  return any ? { gained, lost, complete } : null;
}

/** Command power moved into and out of reserve between `from` and `through`,
 * from changes in the game's recorded reserve counter between consecutive reads.
 * The counter's level is not an amount held (it can sit below zero), so only its
 * changes are ever shown. */
export function reserveMoves(reply: PoliticsLedgerReply, from: number, through: number): { reserved: bigint; released: bigint } {
  let reserved = 0n, released = 0n;
  const holds = reply.commandHolds;
  for (let at = 1; at < holds.length; at++) {
    const before = holds[at - 1], after = holds[at];
    if (after.readHour < from || after.readHour > through) continue;
    if (before.throughHour + 1 !== after.readHour || before.totalRaw === null || after.totalRaw === null) continue;
    const change = BigInt(after.totalRaw) - BigInt(before.totalRaw);
    if (change > 0n) reserved += change; else released -= change;
  }
  return { reserved, released };
}

/** A day's one-off payments revealed by `through`; recurring costs are part of the daily change. */
export function oneOffSpendsOn(index: PowerBalanceIndex, day: number, through: number) {
  return (index.spends.get(day) ?? []).flatMap(spend => {
    if (RECURRING_CATEGORIES.has(spend.category)) return [];
    const item = index.items.get(spend.itemId)!;
    const category = index.categories.get(spend.category)!;
    const at = spendAt(spend, item, category.name, through, category.identified);
    return at ? [at] : [];
  });
}

/** The details under a balance: the day's change and what made it, each one-off
 * payment and, for command power, what moved into or out of reserve. Each is local
 * to the hovered game day, never uses a read after the shown moment, and a day
 * without a rate observation is not recorded, never zero. */
export function powerBalanceRows(index: PowerBalanceIndex, hour: number, visibleThrough: number, digits: number): TooltipRow[] {
  const through = Math.min(hour, visibleThrough);
  const day = dayAt(hour);
  const daily = (index.days.get(day) ?? []).filter(value => {
    const read = powerDailyReadHour(value, index.currency);
    return read !== null && value.dayHour <= through && read <= through;
  });
  const political = index.currency === 'Political';
  const rate = (value: bigint | null) => value === null ? 'Not recorded' : formatRate(value, digits);
  const rows: TooltipRow[] = [];
  if (!daily.length) rows.push({ label: political ? 'Daily change' : 'Daily gain', value: 'Not recorded', tone: 'muted' });
  for (const value of daily) {
    const figures = dailyFigures(value, index.currency);
    if (political) {
      rows.push({ label: 'Daily change', value: rate(figures.net), tone: figures.net === null ? 'muted' : undefined });
      if (figures.gain !== null) rows.push({ label: 'Gain', value: rate(figures.gain), indent: true, tone: 'muted' });
      if (figures.costs !== null && figures.costs !== 0n) rows.push({ label: 'Daily costs', value: rate(-figures.costs), indent: true, tone: 'muted' });
    } else {
      rows.push({ label: figures.gain !== null && figures.gain < 0n ? 'Daily deficit' : 'Daily gain', value: rate(figures.gain),
        tone: figures.gain === null ? 'muted' : undefined });
    }
    const lost = lostAtCap(figures);
    if (lost) rows.push({ label: 'Lost at the cap', value: rate(-lost), indent: political, tone: 'muted' });
  }
  const spends = oneOffSpendsOn(index, day, through);
  for (const [position, at] of spends.entries()) {
    rows.push({ label: spendLabel(at.item, categoryLabel(at.category, at.identified)), divider: position === 0,
      value: at.charge === null && !at.knownCharges ? 'Not recorded' : formatSpent(at.recordedCharge, digits),
      tone: at.charge === null && !at.knownCharges ? 'muted' : undefined });
  }
  if (!political) {
    const { reserved, released } = reserveMoves(index.reply, day * 24, through);
    if (reserved) rows.push({ label: 'Reserved', value: formatPowerChange(-reserved, digits), divider: !spends.length });
    if (released) rows.push({ label: 'Released from reserve', value: formatPowerChange(released, digits), divider: !spends.length && !reserved });
  }
  if (!coveredAt(index, through)) rows.push({ label: 'Payments weren’t recorded at this time.', tone: 'muted' });
  return rows;
}
