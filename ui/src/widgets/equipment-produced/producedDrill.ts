import { formatDateTime } from '../../dashboard/calendar.ts';
import { readAt } from '../../dashboard/reads.ts';
import { shownRuns } from '../../charts/race/lines.ts';
import { countryPartsRemainder, exactPartAmount } from '../../charts/race/countryPartsDrillData.ts';
import type { CountryPartsData } from '../../charts/race/countryPartsDrillData';
import type { Run, Span } from '../../charts/series';
import { countedSince, equipmentGroupValue, groupValue, keyLabel, MEASURES, producedAt, valueAt } from './producedData.ts';
import type { ProducedModel } from './producedData';
import { UNATTRIBUTED_PART } from '../../charts/race/partsRaceDrill.ts';
import type { TooltipRow } from '../../charts/tooltip';
import type { EquipmentProducedReply } from '../../generated/dashboard';

export const producedDrillNumbers = (reply: EquipmentProducedReply): EquipmentProducedReply => ({ ...reply, warnings: [],
  types: reply.types.map(type => ({ key: type.key, name: null, domain: type.domain })),
  keys: reply.keys.map(key => ({ key: key.key, typeKey: key.typeKey, domain: key.domain, name: null, names: [] })) });

/** One compact run scan, without materializing sample × group values. Unknown classifications
 * occupy the independent total's residual; a missing total leaves the whole stack blank. */
export function producedDrillBands(model: ProducedModel, tag: string, range: Span): Run[][] {
  const bands: Run[][] = Array.from({ length: model.reply.keys.length + 1 }, () => []);
  const series = model.series.get(tag);
  if (!series) return bands;
  series.samples.forEach((sample, index) => {
    const total = valueAt(series.total, index);
    if (Number.isNaN(total)) return;
    let sum = 0n;
    bands.forEach((runs, key) => {
      const raw = key < model.reply.keys.length ? groupValue(series, key, index) : Number(BigInt(total) - sum);
      const value = Number.isNaN(raw) ? 0 : raw;
      if (key < model.reply.keys.length && !Number.isNaN(raw)) sum += BigInt(raw);
      const previous = runs.at(-1);
      if (previous && !sample.breakBefore && previous.lastHour + 1 === sample.firstHour && previous.value === value) previous.lastHour = sample.lastHour;
      else runs.push({ ...sample, value });
    });
  });
  return bands.map(runs => shownRuns(runs, null, range));
}

/** Units are the product's exact delivered counts from logistics accounting,
 * never inferred from active lines. */
export function producedDrillData(model: ProducedModel): CountryPartsData {
  const measure = MEASURES.find(item => item.value === model.reply.measure)!;
  const parts = (hour: number) => model.reply.keys.map((key, index) => ({ id: key.key, label: keyLabel(model, index, hour) }));
  const at = (tag: string, hour: number) => {
    const at = producedAt(model, tag, hour);
    const series = model.series.get(tag);
    if (!at || !series) return null;
    const values = model.reply.keys.map((_, key) => {
      const value = groupValue(series, key, at.index);
      return Number.isNaN(value) ? null : value;
    });
    const since = model.reply.measure === 'total' ? countedSince(model.reply, hour) : null;
    return { total: at.total, values, note: since == null ? '' : `Counted since ${formatDateTime(since)}.` };
  };
  return {
    histories: [...model.series.values()].map(series => ({ tag: series.tag, identities: series.identities })),
    warnings: model.reply.warnings, parts, at, bands: (tag, range) => producedDrillBands(model, tag, range),
    segmentAt: (tag, hour) => producedAt(model, tag, hour)?.stretch ?? null,
    readHour: (tag, hour) => producedAt(model, tag, hour) ? readAt(model.reply.reads, hour) : null,
    keyHours: model.reply.reads,
    tooltip: (tag, hour, name, colours) => {
      const value = at(tag, hour);
      if (!value) return null;
      const rows: TooltipRow[] = parts(hour).map((part, index) => ({ label: part.label,
        value: value.values[index] == null ? 'Not known' : exactPartAmount(value.values[index]!), swatch: colours.get(part.id),
        ...(value.values[index] == null ? { tone: 'muted' as const } : {}) }));
      const sample = producedAt(model, tag, hour)!;
      const series = model.series.get(tag)!;
      const equipment = parts(hour).flatMap((part, index) => {
        if (value.values[index] != null) return [];
        const subtotal = equipmentGroupValue(series, index, sample.index);
        return Number.isNaN(subtotal) ? [] : [{ label: `${part.label} — Equipment lines`, value: exactPartAmount(subtotal), swatch: colours.get(part.id) }];
      });
      rows.push(...equipment);
      const remainder = countryPartsRemainder(value);
      if (remainder) rows.push({ label: UNATTRIBUTED_PART.label, value: exactPartAmount(remainder), swatch: colours.get('') });
      rows.push({ label: measure.label, value: value.total == null ? 'Not known' : exactPartAmount(value.total), strong: true, divider: true });
      return [{ title: name, subtitle: `${formatDateTime(hour)} · ${measure.label}`, rows,
        note: `${value.note}${equipment.length ? ' Total unavailable; equipment subtotal shown.' : ''}` }];
    },
  };
}
