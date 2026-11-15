/** What the Equipment produced widget says and counts: units of chosen equipment types, or the factory IC put into
 * equipment (one currency, so types add up), per day, per month or since the recording began, in words with the
 * date shown; and a country's types at one hour for its breakdown's table. */
import { exactPartShare } from '../../charts/race/countryPartsDrillData.ts';
import { countedSince, formatRaw, typeLabel, UNKNOWN_WORDS } from './producedData.ts';
import type { Count, Measure } from './producedData';
import type { ProducedAmount, ProducedAtReply, ProducedModelAt } from '../../generated/dashboard';

export const COUNTS: readonly { value: Count; label: string; tooltip: string }[] = [
  { value: 'units', label: 'Units', tooltip: 'Units delivered, one equipment type at a time' },
  { value: 'ic', label: 'IC', tooltip: 'Factory IC put into equipment lines: one currency, so every type adds up' },
];

/** Decimals of a count's values: units are whole and IC is the game's fixed5. */
export const countDecimals = (count: Count) => count === 'ic' ? 5 : 0;

/** The caption: what is counted, over which window, at the date shown (none for a chart over time). */
export function producedCaption(count: Count, measure: Measure, date: string | null, since: string | null): string {
  const what = count === 'ic' ? 'IC put into equipment' : 'Units delivered';
  if (date == null) {
    if (measure === 'day') return `${what} per day`;
    if (measure === 'week') return `${what} per week`;
    if (measure === 'month') return `${what} per month`;
    return since ? `${what} since ${since}` : `${what} since the recording began`;
  }
  return `${what} ${producedWindow(measure, date, since)}`;
}

/** The race's caption at `hour`: its window ends that day, and a total counts from the read it began at then
 * (`countedSince`); `null` reads are not yet known. */
export function producedCaptionAt(count: Count, measure: Measure, reads: readonly number[] | null, hour: number, day: (hour: number) => string): string {
  const since = reads ? countedSince(reads, hour) : null;
  return producedCaption(count, measure, day(hour), since == null ? null : day(since));
}

/** The window shown, in words. */
export function producedWindow(measure: Measure, date: string, since: string | null): string {
  if (measure === 'day') return `on ${date}`;
  if (measure === 'week') return `in the week to ${date}`;
  if (measure === 'month') return `in the month to ${date}`;
  return since ? `from ${since} to ${date}` : `to ${date}`;
}

/** Why the race is empty where nothing can be shown for the window. */
export function producedWindowUnavailable(measure: Measure): string {
  return measure === 'day' ? 'A full day of production data is unavailable.'
    : measure === 'week' ? 'A full week of production data is unavailable.'
      : measure === 'month' ? 'A full month of production data is unavailable.' : 'No production data at this time.';
}

/** An amount in words: its value, or why it is not known. */
export const amountText = (amount: ProducedAmount, decimals: number) => amount.value == null ? 'Not known' : formatRaw(amount.value, decimals);
export const amountWhy = (amount: ProducedAmount) => amount.reason ? UNKNOWN_WORDS[amount.reason] : undefined;

/** One type a country made: its models as recorded then, units delivered and IC put in. */
export interface TypeRow { type: string; label: string; models: string; units: number | null; ic: number | null; share: string; recorded: boolean;
  why: string | undefined }

const sum = (amounts: readonly ProducedAmount[]) => amounts.reduce<number | null>((total, amount) => total == null || amount.value == null ? null : total + amount.value, 0);

/** The rows: every type with units delivered or IC put in at the hour, its models named as recorded then. A single
 * model is named alone; several carry their units. A type whose designs are all recorded is still unknown while a
 * design whose type was not recorded holds anything: that output could be of it. Null where the country's
 * production was not recorded then. */
export function typeRows(reply: ProducedAtReply): TypeRow[] | null {
  if (!reply.recorded && !reply.models.length) return null;
  const byType = new Map<string, ProducedModelAt[]>();
  for (const model of reply.models) byType.set(model.typeKey, [...(byType.get(model.typeKey) ?? []), model]);
  const unsure = (pick: (model: ProducedModelAt) => ProducedAmount) => reply.models.some(model => !model.typeRecorded && pick(model).value !== 0);
  const unitsUnsure = unsure(model => model.units), icUnsure = unsure(model => model.ic);
  const icTotal = reply.icTotal.value;
  return [...byType].map(([type, models]) => {
    const recorded = models.every(model => model.typeRecorded);
    const units = recorded && unitsUnsure ? null : sum(models.map(model => model.units));
    const ic = recorded && icUnsure ? null : sum(models.map(model => model.ic));
    const label = models[0].typeRecorded ? typeLabel({ key: type, name: models[0].typeName }) : 'Type not recorded';
    const named = (model: ProducedModelAt) => model.name ?? `Design ${model.variant}`;
    const text = models.length === 1 ? named(models[0]) : models.map(model => `${named(model)} ${amountText(model.units, 0)}`).join(' · ');
    const why = models.map(model => amountWhy(model.units) ?? amountWhy(model.ic)).find(Boolean)
      ?? (units == null || ic == null ? UNKNOWN_WORDS.typeNotRecorded : undefined);
    return { type, label, models: text, units, ic, recorded: reply.recorded, why, share: ic == null ? '—' : exactPartShare(BigInt(ic), icTotal) };
  }).sort((a, b) => (a.ic == null ? 1 : 0) - (b.ic == null ? 1 : 0) || (b.ic ?? 0) - (a.ic ?? 0) || a.label.localeCompare(b.label));
}

export const typeRowRevision = (row: TypeRow) => [row.label, row.models, row.units, row.ic, row.share, row.recorded, row.why];
