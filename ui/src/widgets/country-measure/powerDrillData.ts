import type { FixedRun, HourSpan } from '../../generated/dashboard';
import type { StackRun } from '../../charts/stack';
import { clipToSpans, runAt } from '../../charts/series.ts';
import { dailyFigures, lostAtCap, powerCoverageAt, powerDailyReadHour, reserveMoves } from '../../dashboard/powerBalance.ts';
import type { PowerBalanceIndex } from '../../dashboard/powerBalance';
import { categoryLabel, RECURRING_CATEGORIES, spendAt, spendLabel } from '../../dashboard/politicsSpending.ts';
import { formatPowerChange, formatRate } from '../../dashboard/powerDecimals.ts';

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
 * unrecorded source modifiers remain Other, never guessed. */
export function powerPartsAt(index: PowerBalanceIndex, mode: PowerBreakdown, hour: number, visibleThrough: number, countries?: readonly HourSpan[]): PowerParts {
  const through = Math.min(hour, visibleThrough);
  const day = dayAt(hour);
  const country = countries ? runAt(countries, through) : null;
  if (countries && !country) return { parts: [], total: null, reason: 'Country unavailable at this time.' };
  if (mode === 'income') {
    const observations = (index.days.get(day) ?? []).filter(row => {
      const read = powerDailyReadHour(row, index.currency);
      return read !== null && row.dayHour <= through && read <= through && (!country || row.dayHour >= country.firstHour);
    });
    if (new Set(observations.map(row => `${row.country}:${row.instance}:${row.epoch}`)).size > 1) {
      return { parts: [], total: null, reason: 'Income unavailable for this date.' };
    }
    if (!observations.length) return { parts: [], total: null, reason: 'Income not recorded.' };
    if (index.currency === 'Command') {
      const income = sum(observations.map(row => row.cpIncomeRaw));
      return { parts: [{ id: 'income', category: 'income', label: 'Daily gain', raw: income }], total: income,
        reason: income === null ? 'Income not recorded.' : undefined };
    }
    const gross = sum(observations.map(row => row.ppGrossRaw));
    const costs = sum(observations.map(row => row.ppCostRaw));
    const net = sum(observations.map(row => row.ppIncomeRaw));
    // Daily political power subtracts the signed fixed5 cost.
    // The captured native net includes its i64 wrapping; BigInt never replaces it.
    const wrapping = gross === null || costs === null || net === null ? null : net - (gross - costs);
    return { parts: [
      { id: 'income', category: 'income', label: 'Daily gain', raw: gross },
      { id: 'costs', category: 'costs', label: 'Daily costs', raw: costs === null ? null : -costs },
      { id: 'wrapping', category: 'wrapping', label: 'Adjustment', raw: wrapping },
    ], total: net, reason: net === null ? 'Income not recorded.' : gross === null || costs === null ? 'Income breakdown incomplete.' : undefined };
  }
  // One-off payments by name; costs charged every day as one part per category.
  const parts: PowerPart[] = [];
  const recurring = new Map<string, PowerPart>();
  for (const spend of index.spends.get(day) ?? []) {
    const item = index.items.get(spend.itemId)!;
    const category = index.categories.get(spend.category)!;
    const at = spendAt(spend, item, category.name, through, category.identified, day * 24);
    if (!at) continue;
    const name = categoryLabel(category.name, category.identified);
    const reason = !at.covered || at.charge === null ? 'Spending total is incomplete.' : undefined;
    if (!RECURRING_CATEGORIES.has(spend.category)) {
      parts.push({ id: spend.id, category: spend.category, label: spendLabel(item, name), raw: at.charge, reason });
      continue;
    }
    const part = recurring.get(spend.category);
    if (!part) {
      const folded: PowerPart = { id: `recurring:${spend.category}`, category: spend.category, label: name, raw: at.charge, reason };
      recurring.set(spend.category, folded); parts.push(folded);
    } else {
      part.raw = part.raw === null || at.charge === null ? null : part.raw + at.charge;
      part.reason ??= reason;
    }
  }
  const cover = powerCoverageAt(index, through);
  const covered = (index.currency === 'Political' ? cover?.political : cover?.command) === true
    && cover?.sinceHour !== null && cover?.sinceHour !== undefined && cover.sinceHour <= day * 24;
  const sameCountry = !country || country.firstHour <= day * 24;
  const known = covered && sameCountry && parts.every(part => part.raw !== null);
  return { parts, total: known ? parts.reduce((result, part) => result + part.raw!, 0n) : null,
    reason: !known || parts.some(part => part.reason) ? 'Spending total is incomplete.' : undefined };
}

/** The stack is prepared once from read boundaries. A date with missing income
 * or spending coverage stays a gap; no current values fill another game day. */
export function powerPartHistory(index: PowerBalanceIndex, mode: PowerBreakdown, within: readonly HourSpan[] | null, countries?: readonly HourSpan[]): PowerLayer[] {
  const categories = new Set([...index.spends.values()].flatMap(spends => spends.map(spend => spend.category)));
  const layers: PowerLayer[] = mode === 'income' ? [
    { id: 'income', label: 'Daily gain', runs: [] },
    ...(index.currency === 'Political' ? [{ id: 'costs', label: 'Daily costs', runs: [] },
      { id: 'wrapping', label: 'Adjustment', runs: [] }] : []),
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

/** What the parts don't show: gain the cap took and, for command power, what
 * moved into or out of reserve that day. Never a read after the shown moment. */
export function powerActionDetails(index: PowerBalanceIndex, hour: number, visibleThrough: number, digits = 0): { label: string; value: string }[] {
  const through = Math.min(hour, visibleThrough);
  const day = dayAt(hour);
  const details: { label: string; value: string }[] = [];
  for (const row of index.days.get(day) ?? []) {
    const read = powerDailyReadHour(row, index.currency);
    if (read === null || row.dayHour > through || read > through) continue;
    const lost = lostAtCap(dailyFigures(row, index.currency));
    if (lost) details.push({ label: 'Lost at the cap', value: formatRate(-lost, digits) });
  }
  if (index.currency === 'Command') {
    const { reserved, released } = reserveMoves(index.reply, day * 24, through);
    if (reserved) details.push({ label: 'Reserved', value: formatPowerChange(-reserved, digits) });
    if (released) details.push({ label: 'Released from reserve', value: formatPowerChange(released, digits) });
  }
  return details;
}
