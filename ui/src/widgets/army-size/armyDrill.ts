import { countryValueAt, countryValueMix, countryValuePartsReply, countryValueSegmentAt, countryValuesModel, rankCountryValues } from '../../charts/race/countryValues.ts';
import type { CountryValueAt, CountryValuesModel } from '../../charts/race/countryValues';
import { clipToSpans } from '../../charts/series.ts';
import { countryDrillSpans } from '../../dashboard/countryIdentity.ts';
import type { Span } from '../../charts/series';
import type { ArmyMeasure } from '../../generated/dashboard';
import type { ArmySizeReply } from './armySizeData';
import { formatFixed, rescale } from '../../charts/valueAxis.ts';
import { formatArmyRaw, formatArmyValue, typeName } from './armySizeData.ts';
import { UNATTRIBUTED_PART } from '../../charts/race/partsRaceDrill.ts';
export const ARMY_REMAINDER = UNATTRIBUTED_PART;

function rawTotal(at: CountryValueAt, measure: ArmyMeasure): bigint | null {
  if (at.total === null) return null;
  if (at.rawTotal !== undefined) return BigInt(at.rawTotal);
  const raw = at.total * (measure === 'strength' ? 100000 : 1);
  return Number.isSafeInteger(raw) ? BigInt(raw) : null;
}

export function armyDrillValue(model: CountryValuesModel, tag: string, hour: number, measure: ArmyMeasure) {
  const at = countryValueAt(model, tag, hour);
  return at ? armyDrillAmount(at, model.reply.types, measure) : null;
}

function armyDrillAmount(at: CountryValueAt, types: CountryValuesModel['reply']['types'], measure: ArmyMeasure) {
  const totalRaw = rawTotal(at, measure);
  const share = (raw: bigint | null) => raw === null || totalRaw === null || totalRaw === 0n ? 'Not known'
    : `${formatFixed(rescale((raw < 0n ? -raw : raw) * 100n, totalRaw < 0n ? -totalRaw : totalRaw, 2), 2, 2)}%`;
  const parts = measure === 'battalions' ? types.map((type, index) => {
    const value = at.values[index];
    const raw = value === null ? null : at.rawValues?.[index] != null ? BigInt(at.rawValues[index]!)
      : Number.isSafeInteger(value) ? BigInt(value) : null;
    return { id: type.key, label: typeName(type), value, raw,
      text: raw === null ? 'Not known' : formatArmyRaw(raw.toString(), measure),
      muted: type.key.startsWith('unknown:'), shareText: share(raw) };
  }) : [];
  const identifiedRaw = parts.reduce((sum, part) => sum + (part.raw ?? 0n), 0n);
  const residualRaw = totalRaw === null ? null : totalRaw - identifiedRaw;
  const residual = at.total === null ? null : residualRaw !== null
    ? Number(residualRaw) / (measure === 'strength' ? 100000 : 1)
    : at.total - parts.reduce((sum, part) => sum + (part.value ?? 0), 0);
  return { total: at.total, totalText: at.total === null ? 'Not known' : totalRaw !== null
    ? formatArmyRaw(totalRaw.toString(), measure) : formatArmyValue(at.total), parts,
    residual, residualRaw, residualShare: share(residualRaw), residualText: residual === null ? 'Not known' : residualRaw !== null
      ? formatArmyRaw(residualRaw.toString(), measure) : formatArmyValue(residual),
    missing: parts.filter(part => part.value === null).map(part => part.label) };
}

export function armyDrillBands(reply: ArmySizeReply, tag: string, measure: ArmyMeasure, range: Span) {
  const projection = countryValuePartsReply(reply);
  const source = projection.series.find(series => series.tag === tag);
  const types = [...(measure === 'battalions' ? reply.types : []), { key: ARMY_REMAINDER.id, name: ARMY_REMAINDER.label }];
  const series = source ? [{ ...reply.series.find(series => series.tag === tag)!, mix: source.runs.flatMap(run => {
    const amount = armyDrillAmount({ ...run, stretch: run.firstHour, readHour: run.firstHour }, reply.types, measure);
    return amount.total === null ? [] : [{ firstHour: run.firstHour, lastHour: run.lastHour,
      values: [...amount.parts.map(part => part.value ?? 0), amount.residual] }];
  }) }] : [];
  return countryValueMix(countryValuesModel({ types, series }), tag, null, range);
}

export function armyDrillIdentityReply(reply: ArmySizeReply, tag: string, originHour: number): ArmySizeReply {
  const byTag = countryDrillSpans(reply.series, tag, originHour);
  const aliases = reply.series.map(series => {
    const spans = byTag.get(series.tag) ?? [];
    const runs = clipToSpans(series.runs.map(run => ({ firstHour: run[0], lastHour: run[2] ?? run[0], run })), spans)
      .map(({ firstHour, lastHour, run }) => [firstHour, run[1], lastHour, ...(run[3] === undefined ? [] : [run[3]])] as typeof run);
    return { runs, mix: clipToSpans(series.mix ?? [], spans), segments: spans };
  });
  return { ...reply, series: [{ tag, membership: null,
    runs: aliases.flatMap(series => series.runs).sort((a, b) => a[0] - b[0]),
    mix: aliases.flatMap(series => series.mix).sort((a, b) => a.firstHour - b.firstHour),
    segments: aliases.flatMap(series => series.segments).sort((a, b) => a.firstHour - b.firstHour) }] };
}

export function armyDrillActors(model: CountryValuesModel, candidates: readonly string[], hour: number, top?: number): string[] {
  return top === undefined ? candidates.filter(tag => countryValueSegmentAt(model, tag, hour) !== null)
    : rankCountryValues(model, candidates, hour, top).map(entry => entry.tag);
}

export function armyDrillNumbers(reply: ArmySizeReply): ArmySizeReply {
  return { ...reply, warnings: [], windows: [], window: null, interval: null,
    types: reply.types.map(type => ({ ...type, name: 'Battalion group name unavailable' })) };
}
