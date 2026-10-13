import { formatDateTime } from '../../dashboard/calendar.ts';
import { UNRESOLVED_MARK } from '../../dashboard/equipment.ts';
import { readAt } from '../../dashboard/reads.ts';
import { shownRuns } from '../../charts/race/lines.ts';
import { UNATTRIBUTED_PART } from '../../charts/race/partsRaceDrill.ts';
import type { CountryPartsData } from '../../charts/race/countryPartsDrillData';
import type { Run, Span } from '../../charts/series';
import { countedSince, equipmentGroupValue, formatRaw, groupValue, keyLabel, MEASURES, plotted, producedAt, typeLabel, valueAt } from './producedData.ts';
import type { ProducedModel, ProducedSeriesModel } from './producedData';
import type { Count } from './producedView';
import type { TooltipRow } from '../../charts/tooltip';
import type { EquipmentProducedReply } from '../../generated/dashboard';

export const producedDrillNumbers = (reply: EquipmentProducedReply): EquipmentProducedReply => ({ ...reply, warnings: [],
  types: reply.types.map(type => ({ key: type.key, name: null, domain: type.domain })),
  keys: reply.keys.map(key => ({ key: key.key, typeKey: key.typeKey, domain: key.domain, name: null, names: [] })) });

/** Most types the IC chart colours; the rest share "Other types". */
export const CHART_TYPES = 6;

/** What the breakdown's chart stacks: one type's models (in units or IC), or with no
 * type (IC) every type, the largest `CHART_TYPES` in colour and the rest together. */
export interface ChartSpec { count: Count; decimals: number; type: string | null }

/** One stacked part: the reply's variant keys it adds up. */
interface ChartPart { id: string; keys: readonly number[]; label: (hour: number) => string }

const typeOf = (model: ProducedModel, index: number) => model.reply.keys[index].typeKey ?? model.reply.keys[index].key;

function chartParts(model: ProducedModel, spec: ChartSpec): ChartPart[] {
  const indices = model.reply.keys.map((_, index) => index);
  if (spec.type != null) {
    return indices.filter(index => typeOf(model, index) === spec.type)
      .map(index => ({ id: model.reply.keys[index].key, keys: [index], label: hour => keyLabel(model, index, hour) }));
  }
  const byType = new Map<string, number[]>();
  for (const index of indices) byType.set(typeOf(model, index), [...(byType.get(typeOf(model, index)) ?? []), index]);
  // A type's colour goes by the largest its models reached, read from the compact
  // columns themselves (never sample by sample).
  const peak = (keys: readonly number[]) => {
    let sum = 0;
    for (const key of keys) {
      let most = 0;
      for (const series of model.series.values()) for (const value of series.values.get(key)?.sums ?? []) most = Math.max(most, Math.abs(value));
      sum += most;
    }
    return sum;
  };
  const ranked = [...byType].map(([type, keys]) => ({ type, keys, peak: peak(keys) }))
    .sort((a, b) => b.peak - a.peak || a.type.localeCompare(b.type));
  // A type's content name: listed with the reply's types, or on its own key where the
  // reply is grouped by type.
  const name = (type: string) => typeLabel({ key: type, name: model.reply.types.find(item => item.key === type)?.name
    ?? model.reply.keys.find(key => key.key === type)?.name ?? null });
  const kept = ranked.slice(0, ranked.length > CHART_TYPES + 1 ? CHART_TYPES : ranked.length)
    .map(item => ({ id: item.type, keys: item.keys, label: () => name(item.type) }));
  const rest = ranked.slice(kept.length).flatMap(item => item.keys);
  return rest.length ? [...kept, { id: 'other types', keys: rest, label: () => 'Other types' }] : kept;
}

/** Units of different types are added only within one type; a type content resolves is
 * unknown while a variant of an unresolved type holds anything (it could belong to it),
 * as the product rules for its own type groups. */
function partValues(series: ProducedSeriesModel, parts: readonly ChartPart[], sample: number, unresolved: readonly number[],
  byType: boolean): number[] {
  const unsure = byType && unresolved.some(key => groupValue(series, key, sample) !== 0);
  return parts.map(part => {
    if (unsure && !part.id.startsWith(UNRESOLVED_MARK)) return Number.NaN;
    let sum = 0;
    for (const key of part.keys) sum += groupValue(series, key, sample);
    return sum;
  });
}

/** The breakdown's chart and key as the shared parts drill reads them. Values stay the
 * product's exact integers; the bands plot them in the count's own unit. */
export function producedDrillData(model: ProducedModel, spec: ChartSpec): CountryPartsData {
  const measure = MEASURES.find(item => item.value === model.reply.measure)?.label ?? 'Per week';
  const parts = chartParts(model, spec);
  const byType = spec.type == null;
  const unresolved = model.reply.keys.flatMap((_, index) => typeOf(model, index).startsWith(UNRESOLVED_MARK) ? [index] : []);
  // The whole a sample's parts make: the product's exact total of every type, or one
  // type's models added (unknown where one of them is).
  const totalOf = (series: ProducedSeriesModel, sample: number, values: readonly number[]) =>
    byType ? valueAt(series.total, sample) : values.reduce((sum, value) => sum + value, 0);
  const at = (tag: string, hour: number) => {
    const sample = producedAt(model, tag, hour);
    const series = model.series.get(tag);
    if (!sample || !series) return null;
    const raw = partValues(series, parts, sample.index, unresolved, byType);
    const total = totalOf(series, sample.index, raw);
    const since = model.reply.measure === 'total' ? countedSince(model.reply, hour) : null;
    return { total: Number.isNaN(total) ? null : total, values: raw.map(value => Number.isNaN(value) ? null : value),
      note: since == null ? '' : `Counted since ${formatDateTime(since)}.` };
  };
  const bands = (tag: string, range: Span): Run[][] => {
    const out: Run[][] = Array.from({ length: parts.length + 1 }, () => []);
    const series = model.series.get(tag);
    if (!series) return out;
    series.samples.forEach((sample, index) => {
      const values = partValues(series, parts, index, unresolved, byType);
      const total = totalOf(series, index, values);
      // A missing whole leaves the stack blank; what known parts leave of an exact
      // whole is its own band (geometry only).
      if (Number.isNaN(total)) return;
      const known = values.map(value => Number.isNaN(value) ? 0 : value);
      [...known, total - known.reduce((sum, value) => sum + value, 0)].forEach((raw, band) => {
        const value = plotted(raw, spec.decimals);
        const runs = out[band];
        const previous = runs.at(-1);
        if (previous && !sample.breakBefore && previous.lastHour + 1 === sample.firstHour && previous.value === value) previous.lastHour = sample.lastHour;
        else runs.push({ ...sample, value });
      });
    });
    return out.map(runs => shownRuns(runs, null, range));
  };
  return {
    histories: [...model.series.values()].map(series => ({ tag: series.tag, identities: series.identities })),
    warnings: model.reply.warnings,
    parts: hour => parts.map(part => ({ id: part.id, label: part.label(hour) })),
    at, bands,
    segmentAt: (tag, hour) => producedAt(model, tag, hour)?.stretch ?? null,
    readHour: (tag, hour) => producedAt(model, tag, hour) ? readAt(model.reply.reads, hour) : null,
    keyHours: model.reply.reads,
    tooltip: (tag, hour, name, colours) => {
      const value = at(tag, hour);
      const series = model.series.get(tag);
      const sample = producedAt(model, tag, hour);
      if (!value || !series || !sample) return null;
      const amount = (raw: number | null) => raw == null ? 'Not known' : formatRaw(raw, spec.decimals);
      const rows: TooltipRow[] = parts.flatMap((part, index) => value.values[index] === 0 ? [] : [{ label: part.label(hour),
        value: amount(value.values[index]), swatch: colours.get(part.id), ...(value.values[index] == null ? { tone: 'muted' as const } : {}) }]);
      // Where Rocket coverage leaves a model's combined amount unknown, its exact
      // equipment-line subtotal is named apart, never added in.
      for (const [index, part] of parts.entries()) {
        if (value.values[index] != null || part.keys.length !== 1) continue;
        const subtotal = equipmentGroupValue(series, part.keys[0], sample.index);
        if (!Number.isNaN(subtotal)) rows.push({ label: `${part.label(hour)} — Equipment lines`, value: amount(subtotal), swatch: colours.get(part.id) });
      }
      const residual = value.total == null ? null : value.total - value.values.reduce<number>((sum, part) => sum + (part ?? 0), 0);
      if (residual) rows.push({ label: UNATTRIBUTED_PART.label, value: amount(residual), swatch: colours.get(UNATTRIBUTED_PART.id) });
      rows.push({ label: byType ? 'All equipment' : 'Total', value: amount(value.total), strong: true, divider: true });
      return [{ title: name, subtitle: `${formatDateTime(hour)} · ${spec.count === 'ic' ? 'IC' : 'Units'}, ${measure.toLowerCase()}`, rows,
        ...(value.note ? { note: value.note } : {}) }];
    },
  };
}
