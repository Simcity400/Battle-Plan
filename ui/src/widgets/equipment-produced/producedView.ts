/** What the Equipment produced widget says and counts: units of one equipment type, or
 * the factory IC put into equipment (one currency, so types add up), per day, per
 * month or since the recording began, written in words with the date shown. */
import { exactPartShare } from '../../charts/race/countryPartsDrillData.ts';
import { UNRESOLVED_MARK } from '../../dashboard/equipment.ts';
import type { ProducedCount } from '../../generated/dashboard';
import { formatRaw, groupValue, keyLabel, producedAt, typeLabel, valueAt } from './producedData.ts';
import type { Measure, ProducedModel } from './producedData';

export type Count = ProducedCount;

export const COUNTS: readonly { value: Count; label: string; tooltip: string }[] = [
  { value: 'units', label: 'Units', tooltip: 'Units delivered, one equipment type at a time' },
  { value: 'ic', label: 'IC', tooltip: 'Factory IC put into equipment lines: one currency, so every type adds up' },
];

/** Decimals of a count's values: units are whole and IC is the game's fixed5. A reply
 * says so itself; one from before it did holds units. */
export const countDecimals = (count: Count, reply?: { decimals?: number } | null) =>
  typeof reply?.decimals === 'number' ? reply.decimals : count === 'ic' ? 5 : 0;

/** The caption: what is counted, over which window, at the date shown (none for a
 * chart over time). `since` is the day the running totals count from. */
export function producedCaption(count: Count, measure: Measure, date: string | null, since: string | null): string {
  const what = count === 'ic' ? 'IC put into equipment' : 'Units delivered';
  if (date == null) {
    if (measure === 'day') return `${what} per day`;
    if (measure === 'month') return `${what} per month`;
    return since ? `${what} since ${since}` : `${what} since the recording began`;
  }
  return `${what} ${producedWindow(measure, date, since)}`;
}

/** The window shown, in words: on a day, in the month to it, or from when the
 * running totals count to it. */
export function producedWindow(measure: Measure, date: string, since: string | null): string {
  if (measure === 'day') return `on ${date}`;
  if (measure === 'month') return `in the month to ${date}`;
  return since ? `from ${since} to ${date}` : `to ${date}`;
}

/** Why the race is empty where nothing can be shown for the window. */
export function producedWindowUnavailable(measure: Measure): string {
  return measure === 'day' ? 'A full day of production data is unavailable.'
    : measure === 'month' ? 'A full month of production data is unavailable.' : 'No production data at this time.';
}

/** The type delivered most, in units, over the countries an all-types reply by type
 * holds (each country's latest value), ties in the reply's order: the widget's type
 * until one is chosen. Null when nothing was delivered. */
export function mostDelivered(model: ProducedModel): string | null {
  const sums = new Map<number, number>();
  for (const series of model.series.values()) {
    const last = series.samples.length - 1;
    if (last < 0) continue;
    for (const [key, column] of series.values) {
      const value = valueAt(column, last);
      if (!Number.isNaN(value)) sums.set(key, (sums.get(key) ?? 0) + value);
    }
  }
  let best: number | null = null;
  for (const [key, sum] of [...sums].sort((a, b) => a[0] - b[0])) {
    if (sum > 0 && (best == null || sum > sums.get(best)!)) best = key;
  }
  return best == null ? null : model.reply.keys[best]?.key ?? null;
}

/** One type a country made, from a reply by exact variant: its models (key indices
 * with their exact values, nonzero or unknown, most first) and their sum. */
export interface TypeValue { type: string; value: number | null; models: { key: number; value: number | null }[] }

/** Each type a country made at an hour, most first (unknown last). A type content
 * resolves is unknown while a variant of an unresolved type holds anything, since
 * it could belong to it (the product's own rule for its type groups); an unresolved
 * type keeps its exact sum. Null where the country was not recorded then. */
export function typeValues(model: ProducedModel, tag: string, hour: number): TypeValue[] | null {
  const at = producedAt(model, tag, hour);
  const series = model.series.get(tag);
  if (!at || !series) return null;
  const types = new Map<string, TypeValue>();
  let unsure = false;
  model.reply.keys.forEach((key, index) => {
    const raw = groupValue(series, index, at.index);
    const value = Number.isNaN(raw) ? null : raw;
    if (value === 0) return;
    const type = key.typeKey ?? key.key;
    if (type.startsWith(UNRESOLVED_MARK)) unsure = true;
    const entry = types.get(type) ?? { type, value: 0, models: [] };
    entry.models.push({ key: index, value });
    entry.value = entry.value == null || value == null ? null : entry.value + value;
    types.set(type, entry);
  });
  const order = (a: number | null, b: number | null) => (a == null ? 1 : 0) - (b == null ? 1 : 0) || (b ?? 0) - (a ?? 0);
  return [...types.values()].map(entry => ({ ...entry,
    value: unsure && !entry.type.startsWith(UNRESOLVED_MARK) ? null : entry.value,
    models: entry.models.sort((a, b) => order(a.value, b.value) || a.key - b.key) }))
    .sort((a, b) => order(a.value, b.value) || a.type.localeCompare(b.type));
}

/** One type a country made: its models as recorded then, units delivered and IC put in.
 * A count is null where it is not known, and `recorded` says which counts the
 * recording holds for the country at the hour: one it lacks is never zero. */
export interface TypeRow { type: string; label: string; models: string; units: number | null; ic: number | null; share: string;
  recorded: { units: boolean; ic: boolean } }

/** The rows: every type with units delivered or IC put in, its models named as
 * recorded at the hour. A single model is named alone; several carry their units.
 * A type absent from a count the recording holds is zero there; from a count it
 * lacks, not recorded. Null where units are not recorded for the country then and
 * IC shows nothing either: only recorded units say nothing was delivered. */
export function typeRows(units: ProducedModel | null, ic: ProducedModel | null, tag: string, hour: number, icTotal: number | null): TypeRow[] | null {
  const unitTypes = units ? typeValues(units, tag, hour) : null;
  const icTypes = ic ? typeValues(ic, tag, hour) : null;
  if (!unitTypes && !icTypes?.length) return null;
  const keys = [...new Set([...unitTypes ?? [], ...icTypes ?? []].map(entry => entry.type))];
  const typeName = (type: string) => typeLabel({ key: type, name: (units ?? ic)?.reply.types.find(item => item.key === type)?.name
    ?? ic?.reply.types.find(item => item.key === type)?.name ?? null });
  const recorded = { units: unitTypes !== null, ic: icTypes !== null };
  // A resolved type absent from a count is zero there only when no unresolved variant
  // holds anything in that count: such production could be of this type.
  const unsure = (types: readonly TypeValue[]) => types.some(entry => entry.type.startsWith(UNRESOLVED_MARK));
  const absent = (types: readonly TypeValue[] | null, type: string) => !types ? null : unsure(types) && !type.startsWith(UNRESOLVED_MARK) ? null : 0;
  return keys.map(type => {
    const unitEntry = unitTypes?.find(entry => entry.type === type);
    const icEntry = icTypes?.find(entry => entry.type === type);
    const models = new Map<string, { label: string; units: number | null }>();
    for (const model of unitEntry?.models ?? []) {
      models.set(units!.reply.keys[model.key].key, { label: keyLabel(units!, model.key, hour), units: model.value });
    }
    for (const model of icEntry?.models ?? []) {
      const key = ic!.reply.keys[model.key].key;
      if (!models.has(key)) models.set(key, { label: keyLabel(ic!, model.key, hour), units: unitTypes ? 0 : null });
    }
    const list = [...models.values()];
    const unitsValue = unitEntry ? unitEntry.value : absent(unitTypes, type);
    const icValue = icEntry ? icEntry.value : absent(icTypes, type);
    const text = !unitTypes || (list.length === 1 && list[0].units === unitsValue) ? list.map(model => model.label).join(' · ')
      : list.map(model => `${model.label} ${model.units == null ? 'not known' : formatRaw(model.units, 0)}`).join(' · ');
    return { type, label: typeName(type), models: text, units: unitsValue, ic: icValue, recorded,
      share: icValue == null ? '—' : exactPartShare(BigInt(icValue), icTotal) };
  });
}

export const typeRowRevision = (row: TypeRow) => [row.label, row.models, row.units, row.ic, row.share, row.recorded.units, row.recorded.ic];
