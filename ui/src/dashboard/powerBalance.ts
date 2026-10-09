import type { BalanceDrop, LedgerBalanceChange, LedgerDaily, LedgerSpend, LedgerSpendCoverage, LedgerSpendItem, PoliticsLedgerReply } from '../generated/dashboard';
import type { TooltipRow } from '../charts/tooltip';
import { runAt } from '../charts/series.ts';
import { fixed5Text } from './ideaTooltip.ts';
import { formatDateTime } from './calendar.ts';
import { commandHeldAt, commandHeldTooltip, spendAt, spendText, validatePoliticsSpending } from './politicsSpending.ts';
import type { PowerCurrency } from './politicsSpending';

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

/** Native spend ticks include a payment masked by that hour's income or clamp.
 * Old or incomplete capture keeps the explicitly labelled net-drop fallback. */
export function powerSpendTicks(index: PowerBalanceIndex | undefined, drops: readonly BalanceDrop[], through: number): { hour: number }[] {
  const hours = new Set<number>();
  if (index) for (const spends of index.spends.values()) for (const spend of spends) {
    if (spend.hour <= through && spend.changes.some(change => change.readHour <= through
      && (change.chargeRaw === null || BigInt(change.chargeRaw) > 0n))) hours.add(spend.hour);
  }
  for (const drop of drops) if (drop.hour <= through && (!index || !coveredAt(index, drop.hour))) hours.add(drop.hour);
  return [...hours].sort((a, b) => a - b).map(hour => ({ hour }));
}

const signed = (value: string | null) => value === null ? 'Not recorded'
  : `${BigInt(value) > 0n ? '+' : ''}${fixed5Text(BigInt(value), -1).replace('-', '−')}`;

/** Details are local to the hovered game day and never use a read after the
 * shown moment. A day without a rate observation is unknown, never zero. */
export function powerBalanceRows(index: PowerBalanceIndex, hour: number, visibleThrough: number): TooltipRow[] {
  const through = Math.min(hour, visibleThrough);
  const day = dayAt(hour);
  const daily = (index.days.get(day) ?? []).filter(value => {
    const read = powerDailyReadHour(value, index.currency);
    return read !== null && value.dayHour <= through && read <= through;
  });
  const rows: TooltipRow[] = [];
  if (!daily.length) rows.push({ label: 'Daily income', value: 'Not recorded', tone: 'muted' });
  for (const value of daily) {
    const income = index.currency === 'Political' ? value.ppGrossRaw : value.cpIncomeRaw;
    rows.push({ label: index.currency === 'Command' && income !== null && BigInt(income) < 0n ? 'Daily deficit' : 'Daily income', value: signed(income) });
    if (index.currency === 'Political') {
      rows.push({ label: 'Daily costs', value: signed(value.ppCostRaw), tone: 'muted' });
      rows.push({ label: 'Net daily income', value: signed(value.ppIncomeRaw), tone: 'muted' });
    }
    rows.push({ label: 'Income applied / read', value: `${formatDateTime(value.dayHour)} / ${formatDateTime(powerDailyReadHour(value, index.currency)!)}`, tone: 'muted' });
  }
  const changes = (index.changes.get(day) ?? []).filter(value => value.hour <= through && value.readHour <= through);
  for (const change of changes) {
    rows.push({ label: `${change.action} · ${formatDateTime(change.hour)}`, value: signed(change.realizedDeltaRaw),
      tone: BigInt(change.realizedDeltaRaw) < 0n ? 'negative' : BigInt(change.realizedDeltaRaw) > 0n ? 'positive' : 'muted' });
    if (change.amountRaw !== null && BigInt(change.amountRaw) !== BigInt(change.realizedDeltaRaw)) {
      rows.push({ label: 'Evaluated adjustment', value: signed(change.amountRaw), indent: true });
    }
    if (change.readHour !== change.hour) rows.push({ label: 'Change read', value: formatDateTime(change.readHour), tone: 'muted', indent: true });
    if (!change.covered || change.reason) rows.push({ label: change.reason ?? 'Change coverage is incomplete', tone: 'muted', indent: true });
  }
  const spends = (index.spends.get(day) ?? []).flatMap(spend => {
    const item = index.items.get(spend.itemId)!;
    const category = index.categories.get(spend.category)!;
    const at = spendAt(spend, item, category.name, through, category.identified);
    return at ? [at] : [];
  });
  if (spends.length) rows.push({ label: 'Spending that day', strong: true, divider: true });
  for (const at of spends) {
    rows.push({ label: at.item.action, value: spendText(at.charge), tone: at.covered ? undefined : 'muted' });
    if (at.item.unknown) rows.push({ label: at.item.unknown, tone: 'muted', indent: true });
  }
  if (!coveredAt(index, through)) rows.push({ label: powerCoverageAt(index, through)?.reason ?? 'Spending coverage was not recorded at this hour.', tone: 'muted' });
  if (index.currency === 'Command') rows.push(...commandHeldTooltip(commandHeldAt([index.reply], through)).rows);
  return rows;
}
