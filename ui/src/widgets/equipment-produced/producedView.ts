import { exactPartShare } from '../../charts/race/countryPartsDrillData.ts';
import { countedSince, formatRaw, typeLabel, UNKNOWN_WORDS } from './producedData.ts';
import type { Count, Measure } from './producedData';
import type { ProducedAmount, ProducedAtReply, ProducedModelAt } from '../../generated/dashboard';

export const COUNTS: readonly { value: Count; label: string; tooltip: string }[] = [
  { value: 'units', label: 'Units', tooltip: 'Units delivered, one equipment type at a time' },
  { value: 'ic', label: 'IC', tooltip: 'Factory IC put into equipment lines: one currency, so every type adds up' },
];

export const countDecimals = (count: Count) => count === 'ic' ? 5 : 0;

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

export function producedCaptionAt(count: Count, measure: Measure, reads: readonly number[] | null, hour: number, day: (hour: number) => string): string {
  const since = reads ? countedSince(reads, hour) : null;
  return producedCaption(count, measure, day(hour), since == null ? null : day(since));
}

export function producedWindow(measure: Measure, date: string, since: string | null): string {
  if (measure === 'day') return `on ${date}`;
  if (measure === 'week') return `in the week to ${date}`;
  if (measure === 'month') return `in the month to ${date}`;
  return since ? `from ${since} to ${date}` : `to ${date}`;
}

export function producedWindowUnavailable(measure: Measure): string {
  return measure === 'day' ? 'A full day of production data is unavailable.'
    : measure === 'week' ? 'A full week of production data is unavailable.'
      : measure === 'month' ? 'A full month of production data is unavailable.' : 'No production data at this time.';
}

export const amountText = (amount: ProducedAmount, decimals: number) => amount.value == null ? 'Not known' : formatRaw(amount.value, decimals);
export const amountWhy = (amount: ProducedAmount) => amount.reason ? UNKNOWN_WORDS[amount.reason] : undefined;

export interface TypeRow { type: string; label: string; models: string; units: number | null; ic: number | null; share: string; recorded: boolean;
  why: string | undefined }

const sum = (amounts: readonly ProducedAmount[]) => amounts.reduce<number | null>((total, amount) => total == null || amount.value == null ? null : total + amount.value, 0);

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
