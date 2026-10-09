import type { FixedRun, HourSpan } from '../../generated/dashboard';
import type { StackRun } from '../../charts/stack';
import { clipToSpans, runAt } from '../../charts/series.ts';
import { powerCoverageAt, powerDailyReadHour } from '../../dashboard/powerBalance.ts';
import type { PowerBalanceIndex } from '../../dashboard/powerBalance';
import { spendAt } from '../../dashboard/politicsSpending.ts';

export type PowerBreakdown = 'income' | 'spending';
export interface PowerPart { id: string; category: string; label: string; raw: bigint | null; reason?: string }
export interface PowerParts { parts: PowerPart[]; total: bigint | null; reason?: string }
export interface PowerLayer { id: string; label: string; runs: StackRun[] }
const dayAt = (hour: number) => Math.floor(hour / 24);
const sum = (values: readonly (string | null)[]): bigint | null => values.some(value => value === null)
  ? null : values.reduce<bigint>((result, value) => result + BigInt(value!), 0n);

/** Balance runs already break at country replacement and failed observations.
 * Their stretches guard the ledger's native application, independent of amounts. */
export function powerCountrySegments(runs: readonly FixedRun[]): HourSpan[] {
  const result: HourSpan[] = [];
  for (const run of runs) {
    const previous = result.at(-1);
    if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour) previous.lastHour = run.lastHour;
    else result.push({ firstHour: run.firstHour, lastHour: run.lastHour });
  }
  return result;
}

/** Native daily operands stay apart from realized/clamped applications. The
 * unrecorded source modifiers remain Other / not identified, never guessed. */
export function powerPartsAt(index: PowerBalanceIndex, mode: PowerBreakdown, hour: number, visibleThrough: number, countries?: readonly HourSpan[]): PowerParts {
  const through = Math.min(hour, visibleThrough);
  const day = dayAt(hour);
  const country = countries ? runAt(countries, through) : null;
  if (countries && !country) return { parts: [], total: null, reason: 'Country balance/identity was not recorded at this hour.' };
  if (mode === 'income') {
    const observations = (index.days.get(day) ?? []).filter(row => {
      const read = powerDailyReadHour(row, index.currency);
      return read !== null && row.dayHour <= through && read <= through && (!country || row.dayHour >= country.firstHour);
    });
    if (new Set(observations.map(row => `${row.country}:${row.instance}:${row.epoch}`)).size > 1) {
      return { parts: [], total: null, reason: 'More than one native country identity supplied income on this date.' };
    }
    if (!observations.length) return { parts: [], total: null, reason: 'Daily income was not recorded at this hour.' };
    if (index.currency === 'Command') {
      const income = sum(observations.map(row => row.cpIncomeRaw));
      return { parts: [{ id: 'income', category: 'income', label: 'Other / not identified', raw: income }], total: income,
        reason: income === null ? 'The native daily command power operand was not recorded.' : undefined };
    }
    const gross = sum(observations.map(row => row.ppGrossRaw));
    const costs = sum(observations.map(row => row.ppCostRaw));
    const net = sum(observations.map(row => row.ppIncomeRaw));
    // P: politics.daily@140B990B0 subtracts signed fixed5 cost at 140B993AC.
    // The captured native net includes its i64 wrapping; BigInt never replaces it.
    const wrapping = gross === null || costs === null || net === null ? null : net - (gross - costs);
    return { parts: [
      { id: 'income', category: 'income', label: 'Other / not identified', raw: gross },
      { id: 'costs', category: 'costs', label: 'Daily costs', raw: costs === null ? null : -costs },
      { id: 'wrapping', category: 'wrapping', label: 'Native fixed-point wrapping', raw: wrapping },
    ], total: net, reason: net === null || gross === null || costs === null ? 'A native daily income operand was not recorded.' : undefined };
  }
  const parts: PowerPart[] = [];
  for (const spend of index.spends.get(day) ?? []) {
    const item = index.items.get(spend.itemId)!;
    const category = index.categories.get(spend.category)!;
    const at = spendAt(spend, item, category.name, through, category.identified, day * 24);
    if (at) parts.push({ id: spend.id, category: spend.category, label: item.action, raw: at.charge,
      reason: item.unknown ?? (at.covered ? undefined : 'Payment interval coverage is incomplete.') });
  }
  const cover = powerCoverageAt(index, through);
  const covered = (index.currency === 'Political' ? cover?.political : cover?.command) === true
    && cover?.sinceHour !== null && cover?.sinceHour !== undefined && cover.sinceHour <= day * 24;
  const sameCountry = !country || country.firstHour <= day * 24;
  const known = covered && sameCountry && parts.every(part => part.raw !== null);
  return { parts, total: known ? parts.reduce((result, part) => result + part.raw!, 0n) : null,
    reason: !sameCountry ? 'Country identity or balance coverage changed during this day; recorded costs remain separate.'
      : !covered ? cover?.reason ?? 'Spending coverage does not establish the whole day through this hour.'
      : !known ? 'An evaluated payment was not recorded.' : undefined };
}

/** The stack is prepared once from read boundaries. A date with missing income
 * or spending coverage stays a gap; no current values fill another game day. */
export function powerPartHistory(index: PowerBalanceIndex, mode: PowerBreakdown, within: readonly HourSpan[] | null, countries?: readonly HourSpan[]): PowerLayer[] {
  const categories = new Set([...index.spends.values()].flatMap(spends => spends.map(spend => spend.category)));
  const layers: PowerLayer[] = mode === 'income' ? [
    { id: 'income', label: 'Other / not identified', runs: [] },
    ...(index.currency === 'Political' ? [{ id: 'costs', label: 'Daily costs', runs: [] },
      { id: 'wrapping', label: 'Native fixed-point wrapping', runs: [] }] : []),
  ] : [...index.categories].filter(([id]) => id !== 'dailyDeficit' && categories.has(id))
    .map(([id, category]) => ({ id, label: category.name, runs: [] }));
  const days = new Map<number, Set<number>>();
  const add = (hour: number) => {
    const day = dayAt(hour);
    const boundaries = days.get(day) ?? new Set([day * 24, (day + 1) * 24]);
    boundaries.add(hour); days.set(day, boundaries);
  };
  for (const observations of index.days.values()) for (const observation of observations) {
    const read = powerDailyReadHour(observation, index.currency);
    if (read !== null) { add(observation.dayHour); add(read); }
  }
  for (const spends of index.spends.values()) for (const spend of spends) {
    add(spend.hour);
    for (const change of spend.changes) add(change.readHour);
  }
  // Only observed dates are materialised; long recording gaps do not cause a
  // loop over their missing hours or days.
  for (const coverage of index.coverage) for (const hour of [coverage.firstHour, coverage.lastHour + 1, coverage.value.readHour]) {
    days.get(dayAt(hour))?.add(hour);
  }
  for (const country of countries ?? []) for (const hour of [country.firstHour, country.lastHour + 1]) days.get(dayAt(hour))?.add(hour);
  let previousCountry: HourSpan | null = null;
  for (const boundaries of [...days].sort(([a], [b]) => a - b).map(([, boundaries]) => boundaries)) {
    const hours = [...boundaries].sort((a, b) => a - b);
    for (let at = 0; at + 1 < hours.length; at++) {
      const firstHour = hours[at];
      const lastHour = hours[at + 1] - 1;
      const snapshot = powerPartsAt(index, mode, firstHour, firstHour, countries);
      const country = countries ? runAt(countries, firstHour) : null;
      const sameCountry = country === previousCountry;
      previousCountry = country;
      if (snapshot.total === null || snapshot.parts.some(part => part.raw === null)) continue;
      const totals = new Map<string, bigint>();
      for (const part of snapshot.parts) totals.set(part.category, (totals.get(part.category) ?? 0n) + part.raw!);
      for (const layer of layers) {
        const value = Number(totals.get(layer.id) ?? 0n) / 100000;
        const previous = layer.runs.at(-1);
        if (previous && sameCountry && previous.lastHour + 1 === firstHour && previous.value === value) previous.lastHour = lastHour;
        else layer.runs.push({ firstHour, lastHour, value, startsSegment: !sameCountry || !previous || previous.lastHour + 1 !== firstHour });
      }
    }
  }
  return within ? layers.map(layer => ({ ...layer, runs: clipToSpans(layer.runs, within) })) : layers;
}
