import { contentTitle } from '../../dashboard/contentTitles.ts';
import { validCountryValues } from '../../charts/race/countryValues.ts';
import type { CountryValuesModel, CountryValueAt } from '../../charts/race/countryValues';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { dayLabel } from '../../charts/draw.ts';
import { usFiveDecimals, usNumber } from '../../charts/numberFormat.ts';
import type { BoundTrack } from '../../dashboard/model';
import { int, object, unsigned32 } from '../../dashboard/validation.ts';
import { isOrderedSpans, isSpan } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { OptionValue } from '../../dashboard/options';
import type { ArmyAttribution, ArmyMeasure, ArmySeries, ArmySizeParams, ArmySizeReply as ArmySizeAnswer } from '../../generated/dashboard';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { productRequest } from '../../dashboard/request.ts';

export type ArmySizeReply = Omit<ArmySizeAnswer, 'series'> & { series: (Omit<ArmySeries, 'country'> & { tag: string })[] };

export const MEASURES = [
  { value: 'manpower', label: 'Manpower in the field' },
  { value: 'divisions', label: 'Number of divisions' },
  { value: 'strength', label: 'Combined strength (hit points)' },
  { value: 'battalions', label: 'Battalions' },
] as const satisfies readonly { value: ArmyMeasure; label: string }[];

export function validateArmySizeReply(value: unknown): ArmySizeReply {
  if (object(value) && Array.isArray(value.series) && value.series.some(series => object(series) && 'country' in series)) {
    value = { ...value, series: value.series.map(series => object(series) && (typeof series.country === 'string' || typeof series.country === 'number')
      ? (({ country, ...rest }) => ({ ...rest, tag: countryKey(country as ArmySeries['country']) }))(series) : series) };
  }
  if (!object(value) || typeof value.branchId !== 'string' || typeof value.available !== 'boolean'
    || !isOrderedSpans(value.windows) || !(value.window === null || (int(value.window) && value.window >= 0))
    || !(value.interval === null || isSpan(value.interval))
    || ((value.window === null) !== (value.interval === null))
    || !Array.isArray(value.measures) || !value.measures.every(measure => MEASURES.some(item => item.value === measure))
    || new Set(value.measures).size !== value.measures.length
    || !validCountryValues(value)
    || value.series.some(series => series.identities !== undefined && (!isOrderedSpans(series.identities)
      || series.identities.some(identity => !unsigned32(identity.slot) || !unsigned32(identity.countryId))))
    || value.types.some(type => typeof type.name !== 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The army size reply is malformed.');
  }
  return value as unknown as ArmySizeReply;
}

export function validateArmyWindow(value: unknown, manifest: ArmySizeReply): ArmySizeReply {
  const reply = validateArmySizeReply(value);
  const span = reply.window === null ? undefined : manifest.windows[reply.window];
  if (reply.branchId !== manifest.branchId || !span || !reply.interval
    || reply.interval.firstHour !== span.firstHour || reply.interval.lastHour !== span.lastHour
    || reply.windows.length || reply.series.some(series => series.runs.some(run =>
      !Array.isArray(run) || run[0] < span.firstHour || (run[2] ?? run[0]) > span.lastHour)
      || (series.mix ?? []).some(run => run.firstHour < span.firstHour || run.lastHour > span.lastHour)
      || series.membership?.some(member => member.firstHour < span.firstHour || member.lastHour > span.lastHour)
      || series.identities?.some(identity => identity.firstHour < span.firstHour || identity.lastHour > span.lastHour))) {
    throw new Error('The army size window does not match this recording.');
  }
  return reply;
}

export function armyWindows(manifest: ArmySizeReply | undefined, range: Span, moment: number,
  live: boolean, margin = false): number[] {
  const padding = margin ? Math.max(0, range.lastHour - range.firstHour) / 4 : 0;
  const wanted = manifest?.windows.flatMap((span, index) =>
    (range.firstHour <= range.lastHour && span.firstHour <= range.lastHour + padding
      && span.lastHour >= range.firstHour - padding)
      || (span.firstHour <= moment && moment <= span.lastHour) ? [index] : []) ?? [];
  const current = manifest?.windows.findIndex(span => span.firstHour <= moment && moment <= span.lastHour) ?? -1;
  if (live && manifest && current >= 0 && current + 1 < manifest.windows.length && !wanted.includes(current + 1)) {
    wanted.push(current + 1);
  }
  return wanted.sort((a, b) => a - b);
}

export function mergeArmyWindows(manifest: ArmySizeReply, windows: ReadonlyMap<number, ArmySizeReply>,
  wanted: readonly number[]): ArmySizeReply {
  const replies = [...new Set(wanted)].sort((a, b) => a - b)
    .flatMap(index => windows.has(index) ? [windows.get(index)!] : []);
  const catalogue = new Map(replies.flatMap(reply => reply.types.map(type => [type.key, type] as const)));
  const types = [...catalogue.values()].sort((a, b) => compareTags(a.key, b.key));
  const positions = new Map(types.map((type, index) => [type.key, index]));
  const series = new Map<string, ArmySizeReply['series'][number]>();
  const spans = (before: Span[], next: readonly Span[]) => {
    for (const span of next) {
      const previous = before.at(-1);
      if (previous && previous.firstHour === span.firstHour) previous.lastHour = Math.max(previous.lastHour, span.lastHour);
      else before.push({ ...span });
    }
  };
  for (const reply of replies) {
    for (const item of reply.series) {
      let target = series.get(item.tag);
      if (!target) {
        target = { tag: item.tag, runs: [], segments: [], ...(item.identities ? { identities: [] } : {}), membership: item.membership === null ? null : [], mix: [] };
        series.set(item.tag, target);
      }
      for (const run of item.runs) target.runs.push(run);
      spans(target.segments, item.segments);
      if (item.identities) {
        target.identities ??= [];
        for (const identity of item.identities) {
          const previous = target.identities.at(-1);
          if (previous && previous.lastHour + 1 === identity.firstHour && previous.slot === identity.slot
            && previous.countryId === identity.countryId) previous.lastHour = identity.lastHour;
          else target.identities.push({ ...identity });
        }
      }
      if (target.membership !== null && item.membership !== null) spans(target.membership, item.membership);
      for (const mix of item.mix ?? []) {
        const values: (number | null)[] = types.map(() => 0);
        const rawValues = mix.rawValues ? types.map<string | null>(() => '0') : undefined;
        reply.types.forEach((type, index) => {
          values[positions.get(type.key)!] = mix.values[index];
          if (rawValues) rawValues[positions.get(type.key)!] = mix.rawValues![index];
        });
        target.mix!.push({ ...mix, values, ...(rawValues ? { rawValues } : {}) });
      }
    }
  }
  return { ...manifest, available: replies.some(reply => reply.available), types,
    series: [...series.values()].sort((a, b) => compareTags(a.tag, b.tag)),
    warnings: [...new Set([...manifest.warnings, ...replies.flatMap(reply => reply.warnings)])] };
}

export const offersMeasure = (reply: ArmySizeReply | undefined, measure: ArmyMeasure) =>
  !!reply && reply.measures.includes(measure);

export function measureChoices(reply: ArmySizeReply | undefined): { value: string; label: string }[] {
  return MEASURES.filter(item => offersMeasure(reply, item.value)).map(item => ({ ...item }));
}

export function armyMeasure(value: OptionValue, measures: readonly ArmyMeasure[] | undefined): ArmyMeasure {
  if (measures?.includes(value as ArmyMeasure)) return value as ArmyMeasure;
  return MEASURES.find(item => measures?.includes(item.value))?.value ?? 'divisions';
}

export function armySizeRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, measure: ArmyMeasure,
  countFor: ArmyAttribution, byType: boolean, window: number | null = null): ArmySizeParams {
  return { branchId, countries: resolved.top == null ? resolved.tags.map(countryFromKey) : [], top: resolved.top ?? null,
    measure, countFor, byType, window };
}

export const measureName = (measure: ArmyMeasure) => MEASURES.find(item => item.value === measure)!.label;
export const formatArmyValue = (value: number) => usFiveDecimals.format(value);
function grouped(digits: string): string {
  if (digits.length <= 3) return digits;
  const head = digits.length % 3 || 3;
  let out = digits.slice(0, head);
  for (let at = head; at < digits.length; at += 3) out += `,${digits.slice(at, at + 3)}`;
  return out;
}

function formatSafeArmy(raw: number, measure: ArmyMeasure): string | null {
  if (!Number.isSafeInteger(raw)) return null;
  const sign = raw < 0 ? '-' : '', magnitude = Math.abs(raw);
  if (measure !== 'strength') return sign + grouped(String(magnitude));
  const fraction = String(magnitude % 100000).padStart(5, '0').replace(/0+$/, '');
  return `${sign}${grouped(String(Math.floor(magnitude / 100000)))}${fraction ? `.${fraction}` : ''}`;
}

export function formatArmyRawFast(raw: bigint | number, measure: ArmyMeasure): string {
  return (typeof raw === 'number' || (raw >= -9_007_199_254_740_991n && raw <= 9_007_199_254_740_991n) ? formatSafeArmy(Number(raw), measure) : null)
    ?? formatArmyRaw(raw.toString(), measure);
}

export function formatArmyRaw(raw: string, measure: ArmyMeasure): string {
  const value = BigInt(raw);
  if (measure !== 'strength') return usNumber.format(value);
  const magnitude = value < 0n ? -value : value;
  const fraction = (magnitude % 100000n).toString().padStart(5, '0').replace(/0+$/, '');
  return `${value < 0n ? '-' : ''}${usNumber.format(magnitude / 100000n)}${fraction ? `.${fraction}` : ''}`;
}
export const formatArmyPart = (at: CountryValueAt, index: number) =>
  at.values[index] === null ? 'Not known' : at.rawValues?.[index] != null
    ? formatArmyRaw(at.rawValues[index]!, 'battalions') : formatArmyValue(at.values[index]);
export const typeName = (type: { key: string; name: string | null }) => contentTitle(type.name, type.key, 'Battalion group name unavailable');

export function* armyTooltips(model: CountryValuesModel, name: string, at: CountryValueAt, hour: number, measure: ArmyMeasure,
  colours: ReadonlyMap<string, string>): Generator<TooltipContent> {
  const total: TooltipRow = { label: measureName(measure), value: at.total === null ? '—' : at.rawTotal !== undefined ? formatArmyRaw(at.rawTotal, measure) : formatArmyValue(at.total), strong: true, divider: true };
  const rows: TooltipRow[] = model.reply.types.map((type, index) => ({ label: typeName(type),
    value: at.values[index] === null ? '—' : formatArmyPart(at, index), swatch: colours.get(type.key),
    ...(at.values[index] === null ? { tone: 'muted' as const } : {}) }));
  const base = { title: name, subtitle: `${dayLabel(hour)} · ${measureName(measure)}` };
  yield { ...base, rows: [...rows, total] };
  yield { ...base, rows: [total] };
}

const METADATA_MEASURE: ArmyMeasure = 'divisions';

export const armyMetadataParams = (branchId: string) => armySizeRequest(branchId, { tags: [] }, METADATA_MEASURE, 'owner', false);

export const armyMetadataRequest = (track: BoundTrack | undefined) => track ? productRequest({
  sourceId: track.source.id, revision: track.source.revision, product: 'army-size', params: armyMetadataParams(track.branch.id),
}) : null;

export const armyAbsent = (manifest: ArmySizeReply) => !manifest.available;
