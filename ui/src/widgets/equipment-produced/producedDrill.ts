import { countryKey } from '../../dashboard/countryKey.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { variantNameAt } from '../../dashboard/equipment.ts';
import { readAt } from '../../dashboard/reads.ts';
import { shownRuns } from '../../charts/race/lines.ts';
import { UNATTRIBUTED_PART } from '../../charts/race/partsRaceDrill.ts';
import type { CountryPartsData } from '../../charts/race/countryPartsDrillData';
import type { Run, Span } from '../../charts/series';
import type { TooltipRow } from '../../charts/tooltip';
import { countedSince, formatRaw, MEASURES, plotted, seriesModel, typeLabel, UNKNOWN_WORDS, validSeries, valueAt } from './producedData.ts';
import type { Count, SeriesModel } from './producedData';
import { validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import type { ProducedCountryReply, ProducedModel, ProducedUnknown } from '../../generated/dashboard';

export const CHART_TYPES = 6;
const OTHER_TYPES = { id: 'other types', label: 'Other types' };

export function validateCountryReply(data: unknown): ProducedCountryReply {
  const reply = data as ProducedCountryReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && (typeof reply.asked === 'string' || typeof reply.asked === 'number') && (reply.count === 'units' || reply.count === 'ic') && Number.isInteger(reply.decimals)
    && validCountryIdentityRuns(reply.identities) && validSeries(reply.total)
    && Array.isArray(reply.models) && reply.models.every(model => typeof model.variant === 'string' && typeof model.typeKey === 'string'
      && typeof model.typeRecorded === 'boolean' && Array.isArray(model.names) && validSeries(model.series))
    && Array.isArray(reply.reads) && Array.isArray(reply.observations) && Array.isArray(reply.warnings);
  if (!ok) throw new Error('The equipment produced breakdown reply is malformed.');
  return reply;
}

export interface ChartSpec { count: Count; type: string | null }

interface ChartPart { id: string; models: readonly number[]; label: (hour: number) => string }

const modelLabel = (model: ProducedModel, hour: number) => variantNameAt(model.names, hour, `Design ${model.variant}`);
const typeName = (model: ProducedModel) => model.typeRecorded ? typeLabel({ key: model.typeKey, name: model.typeName }) : 'Type not recorded';

function chartParts(reply: ProducedCountryReply, spec: ChartSpec): ChartPart[] {
  if (spec.type != null) {
    return reply.models.flatMap((model, index) => model.typeKey === spec.type ? [{ id: model.variant, models: [index], label: (hour: number) => modelLabel(model, hour) }] : []);
  }
  const byType = new Map<string, number[]>();
  reply.models.forEach((model, index) => byType.set(model.typeKey, [...(byType.get(model.typeKey) ?? []), index]));
  const peak = (indices: readonly number[]) => indices.reduce((total, index) =>
    total + reply.models[index].series.values.reduce<number>((most, value) => Math.max(most, Math.abs(value ?? 0)), 0), 0);
  const ranked = [...byType].map(([type, models]) => ({ type, models, peak: peak(models) })).sort((a, b) => b.peak - a.peak || a.type.localeCompare(b.type));
  const kept = ranked.slice(0, ranked.length > CHART_TYPES + 1 ? CHART_TYPES : ranked.length)
    .map(item => ({ id: item.type, models: item.models, label: () => typeName(reply.models[item.models[0]]) }));
  const rest = ranked.slice(kept.length).flatMap(item => item.models);
  return rest.length ? [...kept, { ...OTHER_TYPES, models: rest, label: () => OTHER_TYPES.label }] : kept;
}

interface PartsAt { values: (number | null)[]; total: number | null; reason: ProducedUnknown | null }

function partsAt(reply: ProducedCountryReply, models: readonly SeriesModel[], parts: readonly ChartPart[], spec: ChartSpec,
  read: (model: SeriesModel) => number | null | undefined, whole: number | null | undefined): PartsAt | null {
  const values = parts.map(part => part.models.reduce<number | null>((sum, index) => {
    const value = read(models[index]);
    return sum == null || value == null ? null : sum + value;
  }, 0));
  if (spec.type == null) {
    if (whole === undefined) return null;
    return { values, total: whole, reason: whole === null ? 'notRecorded' : null };
  }
  const recorded = reply.models.some(model => model.typeKey === spec.type && model.typeRecorded);
  const unsure = recorded && reply.models.some((model, index) => !model.typeRecorded && read(models[index]) !== 0);
  if (unsure) return { values, total: null, reason: 'typeNotRecorded' };
  const sum = values.reduce<number | null>((held, value) => held == null || value == null ? null : held + value, 0);
  return { values, total: sum, reason: sum === null ? 'notRecorded' : null };
}

function boundaries(series: readonly SeriesModel[]): number[] {
  const hours = new Set<number>();
  for (const model of series) {
    model.runs.forEach((run, index) => {
      hours.add(run.firstHour);
      if (index + 1 >= model.runs.length || model.runs[index + 1].firstHour !== run.lastHour + 1) hours.add(run.lastHour + 1);
    });
  }
  return [...hours].sort((a, b) => a - b);
}

export function producedDrillData(reply: ProducedCountryReply, spec: ChartSpec): CountryPartsData {
  const measure = MEASURES.find(item => item.value === reply.measure)?.label ?? 'Per week';
  const decimals = reply.decimals;
  const parts = chartParts(reply, spec);
  const models = reply.models.map(model => seriesModel(model.series));
  const total = seriesModel(reply.total);
  const asked = countryKey(reply.asked);
  const involved = spec.type == null ? [total, ...models] : models.filter((_, index) => reply.models[index].typeKey === spec.type || !reply.models[index].typeRecorded);
  const at = (tag: string, hour: number) => {
    if (tag !== asked) return null;
    const totalAt = valueAt(total, hour);
    if (!totalAt) return null;
    const held = partsAt(reply, models, parts, spec, model => { const found = valueAt(model, hour); return found ? found.value : undefined; }, totalAt.value);
    if (!held) return null;
    const since = reply.measure === 'total' ? countedSince(reply.observations, hour) : null;
    return { total: held.total, values: held.values, note: since == null ? '' : `Counted since ${formatDateTime(since)}.` };
  };
  const bands = (tag: string, range: Span): Run[][] => {
    const out: Run[][] = Array.from({ length: parts.length + 1 }, () => []);
    if (tag !== asked) return out;
    const hours = boundaries(involved);
    const breaks = new Set(reply.total.breaks.map(index => reply.total.firstHours[index]));
    for (let index = 0; index + 1 < hours.length; index++) {
      const [first, last] = [hours[index], hours[index + 1] - 1];
      const totalAt = valueAt(total, first);
      if (!totalAt) continue;
      const held = partsAt(reply, models, parts, spec, model => { const found = valueAt(model, first); return found ? found.value : undefined; }, totalAt.value);
      if (!held || held.total == null) continue;
      const known = held.values.map(value => value ?? 0);
      [...known, held.total - known.reduce((sum, value) => sum + value, 0)].forEach((raw, band) => {
        const value = plotted(raw, decimals);
        const runs = out[band];
        const previous = runs.at(-1);
        if (previous && !breaks.has(first) && previous.lastHour + 1 === first && previous.value === value) previous.lastHour = last;
        else runs.push({ firstHour: first, lastHour: last, value, ...(breaks.has(first) ? { breakBefore: true } : {}) });
      });
    }
    return out.map(runs => shownRuns(runs, null, range));
  };
  return {
    histories: [{ tag: reply.tag, identities: reply.identities }],
    warnings: reply.warnings,
    parts: hour => parts.map(part => ({ id: part.id, label: part.label(hour) })),
    at, bands,
    segmentAt: (tag, hour) => tag === asked ? valueAt(total, hour)?.stretch ?? null : null,
    readHour: (tag, hour) => tag === asked && valueAt(total, hour) ? readAt(reply.reads, hour) : null,
    keyHours: reply.reads,
    tooltip: (tag, hour, name, colours) => {
      const value = at(tag, hour);
      if (!value) return null;
      const amount = (raw: number | null) => raw == null ? 'Not known' : formatRaw(raw, decimals);
      const rows: TooltipRow[] = parts.flatMap((part, index) => value.values[index] === 0 ? [] : [{ label: part.label(hour),
        value: amount(value.values[index]), swatch: colours.get(part.id), ...(value.values[index] == null ? { tone: 'muted' as const } : {}) }]);
      const residual = value.total == null ? null : value.total - value.values.reduce<number>((sum, part) => sum + (part ?? 0), 0);
      if (residual) rows.push({ label: UNATTRIBUTED_PART.label, value: amount(residual), swatch: colours.get(UNATTRIBUTED_PART.id) });
      rows.push({ label: spec.type == null ? 'All equipment' : 'Total', value: amount(value.total), strong: true, divider: true });
      const reason = value.total == null ? partsAt(reply, models, parts, spec, model => valueAt(model, hour)?.value, valueAt(total, hour)?.value)?.reason : null;
      const note = [value.note, reason ? UNKNOWN_WORDS[reason] : ''].filter(Boolean).join(' ');
      return [{ title: name, subtitle: `${formatDateTime(hour)} · ${spec.count === 'ic' ? 'IC' : 'Units'}, ${measure.toLowerCase()}`, rows,
        ...(note ? { note } : {}) }];
    },
  };
}
