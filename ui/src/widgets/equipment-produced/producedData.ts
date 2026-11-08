import { contentTitle } from '../../dashboard/contentTitles.ts';
/** The Equipment produced widget's data: the `equipment-produced` reply checked and
 * decoded, and each country's units at an hour. The product counts every unit a
 * country's production lines delivered from the exact per-line output, groups them
 * and measures them (running total, the week or the month before); see
 * `crates/dashboard/src/products/equipment_produced.rs`. An unknown value stays
 * unknown here: it is never drawn, ranked or summed as zero. */
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { Run, Span } from '../../charts/series';
import { shownRuns } from '../../charts/race/lines.ts';
import { runIndexAt, stretchStarts } from '../../charts/series.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatRate } from '../../charts/valueAxis.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { domainName, variantNameAt } from '../../dashboard/equipment.ts';
import { readAt } from '../../dashboard/reads.ts';
import { chartSummaryFrom } from '../../charts/chartSummary.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import type { CountryIdentityRun, EquipmentProducedReply, HourSpan, ProducedCoded, ProducedCount, ProducedGrouping, ProducedKey, ProducedMeasure, ProducedParams, SeriesView } from '../../generated/dashboard';

export type Measure = ProducedMeasure;
export type Grouping = ProducedGrouping;

/** The windows the widget offers, per day first (spec: per day by default). The
 * product also counts a week, which a reply may still name. */
export const MEASURES: readonly { value: Measure; label: string }[] = [
  { value: 'day', label: 'Per day' },
  { value: 'month', label: 'Per month' },
  { value: 'total', label: 'Since recording start' },
];
const REPLY_MEASURES: readonly Measure[] = ['total', 'day', 'week', 'month'];

export const GROUPINGS: readonly { value: Grouping; label: string; noun: string }[] = [
  { value: 'type', label: 'Equipment type', noun: 'type' },
  { value: 'variant', label: 'Exact variant', noun: 'variant' },
  { value: 'domain', label: 'Land, air and naval', noun: 'arm' },
];

const INT = (value: unknown): value is number => Number.isInteger(value);
const ascending = (values: unknown): values is number[] => Array.isArray(values) && values.every((value, index) => INT(value) && (index === 0 || values[index - 1] < value));
const text = (value: unknown) => value === null || typeof value === 'string';
const spans = (list: readonly HourSpan[]) => list.every((span, index) => INT(span.firstHour) && INT(span.lastHour) && span.firstHour <= span.lastHour
  && (index === 0 || list[index - 1].lastHour < span.firstHour));

/** A coded value: whole deltas, and unknown ranges as ascending `[start, end)` pairs. */
const coded = (values: ProducedCoded) => typeof values === 'object' && values != null && INT(values.from) && values.from >= 0
  && Array.isArray(values.deltas) && values.deltas.every(delta => Number.isSafeInteger(delta))
  && Array.isArray(values.unknown) && values.unknown.length % 2 === 0
  && values.unknown.every((index, position) => INT(index) && index >= 0 && (position === 0 || values.unknown[position - 1] < index));

/** Checks the reply's shape: samples in hour order without overlaps, breaks and
 * values pointing at samples and keys that exist, safe whole numbers, and names in
 * hour order. */
export function validateProducedReply(data: unknown): EquipmentProducedReply {
  const reply = data as EquipmentProducedReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && REPLY_MEASURES.includes(reply.measure) && GROUPINGS.some(item => item.value === reply.grouping)
    && (reply.count === undefined || reply.count === 'units' || reply.count === 'ic')
    && (reply.decimals === undefined || (Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12))
    && Array.isArray(reply.types) && reply.types.every(type => typeof type.key === 'string' && type.key.length > 0 && text(type.name))
    && Array.isArray(reply.keys) && reply.keys.every(key => typeof key.key === 'string' && text(key.name) && text(key.typeKey)
      && Array.isArray(key.names) && key.names.every((name, index) => INT(name.firstHour) && text(name.name) && typeof name.known === 'boolean' && (index === 0 || key.names[index - 1].firstHour < name.firstHour)))
    && new Set(reply.keys.map(key => key.key)).size === reply.keys.length
    && Array.isArray(reply.series) && reply.series.every(series => {
      const count = Array.isArray(series.firstHours) ? series.firstHours.length : -1;
      return typeof series.tag === 'string' && count >= 0 && Array.isArray(series.lastHours) && series.lastHours.length === count
        && validCountryIdentityRuns(series.identities)
        && series.firstHours.every((first, index) => INT(first) && INT(series.lastHours[index]) && first <= series.lastHours[index]
          && (index === 0 || series.lastHours[index - 1] < first))
        && ascending(series.breaks) && series.breaks.every(index => index >= 0 && index < count)
        && Array.isArray(series.values) && series.values.every(values => INT(values.key) && values.key >= 0 && values.key < reply.keys.length
          && coded(values.values) && (values.equipmentValues === undefined || coded(values.equipmentValues)))
        && new Set(series.values.map(values => values.key)).size === series.values.length
        && (series.total === null || coded(series.total))
        && (series.membership === null || (Array.isArray(series.membership) && spans(series.membership)))
        && (series.summary === undefined || chartSummaryFrom(series.summary) !== null);
    })
    && new Set(reply.series.map(series => series.tag)).size === reply.series.length
    && Array.isArray(reply.known) && spans(reply.known)
    && ascending(reply.reads) && ascending(reply.observations)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The equipment produced reply is malformed.');
  return reply;
}

/** What the widget asks the product for: the top N by the measure shown of the
 * included types, or the chosen countries; with a chart's `view`, the samples it shows,
 * each line as its exact summary where they are denser than its width. */
export function producedRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, measure: Measure, grouping: Grouping,
  included: readonly string[], refits: boolean, count: ProducedCount = 'units', view?: SeriesView | null): ProducedParams {
  const common = { branchId, measure, count, grouping, included: [...included], refits, ...(view ? { view } : {}) };
  return resolved.top != null ? { ...common, tags: [], top: resolved.top } : { ...common, tags: [...resolved.tags], top: null };
}

/** The params as sent: units are the product's default, so a units request leaves
 * `count` out and a bridge from before the IC count answers it too. */
export function requestParams(params: ProducedParams): ProducedParams | Omit<ProducedParams, 'count'> {
  if (params.count !== 'units') return params;
  const { count: _units, ...rest } = params;
  return rest;
}

/** A sample: the hours one value holds. */
export interface Sample extends Span { breakBefore: boolean }

/** A coded value kept compact (finding: 1,000 constant columns of 3,650 samples for 30
 * countries would take about 877 MB expanded): the running sums of its deltas only, as
 * long as the reply's own deltas, read at a sample by `valueAt`. */
export interface Column { from: number; sums: Float64Array; unknown: readonly number[] }

/** A coded value ready to read: its deltas summed once. */
export function column(values: ProducedCoded): Column {
  const sums = new Float64Array(values.deltas.length);
  let sum = 0;
  values.deltas.forEach((delta, index) => { sum += delta; sums[index] = sum; });
  return { from: values.from, sums, unknown: values.unknown };
}

/** A column's value at a sample: NaN inside an unknown range; 0 before `from` (or with
 * no column: a known zero throughout); else the running sum there, the last holding
 * once the deltas end. */
export function valueAt(values: Column | null, index: number): number {
  if (!values) return 0;
  const unknown = values.unknown;
  // The ranges are few and ascending: the last start at or before the index decides.
  let low = 0;
  let high = unknown.length / 2 - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (unknown[middle * 2] <= index) {
      if (index < unknown[middle * 2 + 1]) return Number.NaN;
      low = middle + 1;
    } else high = middle - 1;
  }
  if (index < values.from || !values.sums.length) return 0;
  return values.sums[Math.min(index - values.from, values.sums.length - 1)];
}

/** One country: its samples, where each unbroken stretch of them starts, each group's
 * column (none for a group that is 0 at every sample), its exact total (the product's,
 * known however the variants split) and, for a top-N reply, the hours it was among
 * the top N (its samples are cut to them). */
export interface ProducedSeriesModel {
  tag: string;
  samples: readonly Sample[];
  stretch: readonly number[];
  values: ReadonlyMap<number, Column>;
  equipmentValues: ReadonlyMap<number, Column>;
  total: Column | null;
  membership: readonly HourSpan[] | null;
  identities: readonly CountryIdentityRun[];
  /** A dense view's exact summary of the total, drawn in place of its samples. */
  summary: ChartSummary | null;
}

/** A reply ready for lookups, built once per reply. */
export interface ProducedModel {
  reply: EquipmentProducedReply;
  series: ReadonlyMap<string, ProducedSeriesModel>;
  tags: readonly string[];
}

/** A group's value in a series at a sample (NaN where unknown). */
export const groupValue = (series: ProducedSeriesModel, key: number, index: number) => valueAt(series.values.get(key) ?? null, index);

/** Available only when the native reply separately retained equipment lines.
 * It never supplies the combined chart or ranking amount. */
export const equipmentGroupValue = (series: ProducedSeriesModel, key: number, index: number) => {
  const value = series.equipmentValues.get(key);
  return value ? valueAt(value, index) : Number.NaN;
};

/** The numbers the model holds for its values: bounded by the reply's own coded size,
 * never by samples × groups. */
export function storedValues(model: ProducedModel): number {
  let count = 0;
  for (const series of model.series.values()) {
    for (const values of series.values.values()) count += values.sums.length + values.unknown.length;
    for (const values of series.equipmentValues.values()) count += values.sums.length + values.unknown.length;
    if (series.total) count += series.total.sums.length + series.total.unknown.length;
  }
  return count;
}

export function producedModel(reply: EquipmentProducedReply): ProducedModel {
  const series = new Map(reply.series.map(item => {
    const breaks = new Set(item.breaks);
    const samples = item.firstHours.map((firstHour, index) => ({ firstHour, lastHour: item.lastHours[index], breakBefore: breaks.has(index) }));
    const values = new Map(item.values.map(entry => [entry.key, column(entry.values)] as const));
    const equipmentValues = new Map(item.values.flatMap(entry => entry.equipmentValues === undefined ? [] : [[entry.key, column(entry.equipmentValues)] as const]));
    return [item.tag, { tag: item.tag, samples, stretch: stretchStarts(samples), values, equipmentValues, total: item.total ? column(item.total) : null,
      membership: item.membership, identities: item.identities ?? [], summary: item.summary ? chartSummaryFrom(item.summary) : null }] as const;
  }));
  return { reply, series, tags: reply.series.map(item => item.tag) };
}

/** A value, or null where unknown. */
const known = (value: number) => Number.isNaN(value) ? null : value;

/** A country at an hour: its total (null where unknown), the sample it was read in and
 * the unbroken stretch that sample belongs to. Null where no sample holds the hour. */
export interface ProducedAt { total: number | null; index: number; stretch: number }

export function producedAt(model: ProducedModel, tag: string, hour: number): ProducedAt | null {
  const series = model.series.get(tag);
  if (!series) return null;
  const index = runIndexAt(series.samples, Math.floor(hour));
  if (index < 0) return null;
  return { total: known(valueAt(series.total, index)), index, stretch: series.stretch[index] };
}

/** The countries ranked by the value shown at an hour, highest first (ties in the
 * product's tag order, `compareTags`, so a tie ranks as the product ranked it), at
 * most `count`; a country not recorded then, or whose value is unknown, is left out,
 * never ranked as zero. With `produced`, as for the top N, a country ranks only once
 * it has produced something: a zero is left out too (user decision, 29 Sep). */
export function rankProduced(model: ProducedModel, candidates: readonly string[], hour: number, count: number,
  produced = false): { tag: string; at: ProducedAt & { total: number } }[] {
  return candidates.flatMap(tag => {
    const at = producedAt(model, tag, hour);
    return at && at.total !== null && !(produced && at.total === 0) ? [{ tag, at: { ...at, total: at.total } }] : [];
  }).sort((a, b) => b.at.total - a.at.total || compareTags(a.tag, b.tag)).slice(0, Math.max(0, count));
}

/** An empty top-N race means zero only where the product knows at least one
 * requested total and no present requested country's total is unknown. */
export function emptyRace(model: ProducedModel, hour: number): 'nothing' | 'unknown' {
  return runIndexAt(model.reply.known, Math.floor(hour)) >= 0 ? 'nothing' : 'unknown';
}

/** A key's name at an hour: a type's content name (else its token), a variant as
 * recorded then (never a later name), or an arm. */
export function keyLabel(model: ProducedModel, key: number, hour: number): string {
  const item: ProducedKey | undefined = model.reply.keys[key];
  if (!item) return '';
  if (model.reply.grouping === 'domain') return domainName(item.key);
  if (model.reply.grouping === 'variant') return variantNameAt(item.names, hour, `Variant ${item.key}`);
  return contentTitle(item.name, item.key, 'Equipment type name unavailable');
}

/** A type's name in the settings list: content's, else its recorded token. */
export const typeLabel = (type: { key: string; name: string | null }) => contentTitle(type.name, type.key, 'Equipment type name unavailable');

/** How keys are coloured: each its own palette colour while they fit, else the
 * largest keep theirs (in the reply's order) and the rest share "Other" (spec: a
 * ninth category folds into Other). */
export interface KeyBand { id: string; keys: readonly number[]; color: string; other: boolean }

/** The largest magnitude a column reaches. */
function peakOf(values: Column): number {
  let peak = 0;
  for (const sum of values.sums) if (Math.abs(sum) > peak) peak = Math.abs(sum);
  return peak;
}

/** Bands over `keys` (in the reply's order): each its own palette colour while they
 * fit, else the largest by `peak` keep theirs and the rest share "Other". */
function bandsOf(keys: readonly number[], peak: (key: number) => number, palette: readonly string[], otherColor: string): KeyBand[] {
  if (keys.length <= palette.length) return keys.map((key, index) => ({ id: String(key), keys: [key], color: palette[index], other: false }));
  const largest = keys.map(key => ({ key, peak: peak(key) })).sort((a, b) => b.peak - a.peak || a.key - b.key)
    .slice(0, palette.length - 1).map(item => item.key).sort((a, b) => a - b);
  const kept = new Set(largest);
  return [...largest.map((key, index) => ({ id: String(key), keys: [key], color: palette[index], other: false })),
    { id: 'other', keys: keys.filter(key => !kept.has(key)), color: otherColor, other: true }];
}

/** The reply's bands: the largest groups over every country keep a colour. */
export function keyBands(model: ProducedModel, palette: readonly string[], otherColor: string): KeyBand[] {
  const peaks = new Map<number, number>();
  for (const series of model.series.values()) {
    for (const [key, values] of series.values) peaks.set(key, Math.max(peaks.get(key) ?? 0, peakOf(values)));
  }
  return bandsOf(model.reply.keys.map((_, key) => key), key => peaks.get(key) ?? 0, palette, otherColor);
}

/** One country's own bands (user decision, 29 Sep: by exact variant, each country's
 * bar keeps its own largest variants in colour): its groups only, the largest it made
 * keeping a colour and the rest in "Other". */
export function seriesBands(model: ProducedModel, tag: string, palette: readonly string[], otherColor: string): KeyBand[] {
  const series = model.series.get(tag);
  if (!series) return [];
  const keys = [...series.values.keys()].sort((a, b) => a - b);
  return bandsOf(keys, key => peakOf(series.values.get(key)!), palette, otherColor);
}

/** Whether each country's bars and mix fold their own groups (by exact variant, where
 * every country makes its own designs) rather than the reply's. */
export const foldsPerCountry = (model: ProducedModel) => model.reply.grouping === 'variant';

/** A band's value at a sample: the sum of its keys, NaN where any is unknown. */
export function bandValue(series: ProducedSeriesModel, band: KeyBand, index: number): number {
  let sum = 0;
  for (const key of band.keys) sum += groupValue(series, key, index);
  return sum;
}

export function equipmentBandValue(series: ProducedSeriesModel, band: KeyBand, index: number): number {
  let sum = 0;
  for (const key of band.keys) {
    const combined = groupValue(series, key, index);
    if (!Number.isNaN(combined)) return Number.NaN;
    sum += equipmentGroupValue(series, key, index);
  }
  return sum;
}

export const bandLabel = (model: ProducedModel, band: KeyBand, hour: number) => band.other ? 'Other' : keyLabel(model, band.keys[0], hour);

export const formatUnits = (value: number) => formatAmount(BigInt(Math.round(value)), 0);

/** A recorded integer as the number the charts plot, in the count's own unit. */
export const plotted = (raw: number, decimals: number) => raw / 10 ** decimals;

/** A recorded integer written exactly: units whole; IC by the rate rule (whole from
 * 10, two significant figures below), so a small nonzero amount never reads as 0. */
export const formatRaw = (raw: number, decimals: number) =>
  decimals === 0 ? formatAmount(BigInt(raw), 0, 0) : formatRate(BigInt(raw), decimals);

/** A plotted value written as `formatRaw` writes its recorded integer, recovered
 * exactly before formatting. */
export const formatProduced = (value: number, decimals: number) => formatRaw(Math.round(value * 10 ** decimals), decimals);

/** Ranked production and its optional groups for the shared view. A
 * band whose value is not known then (a type an unresolved variant could belong to)
 * has no part of its own: what the known bands leave of the exact total is hatched. */
export function raceEntries(model: ProducedModel, ranked: readonly { tag: string; at: ProducedAt & { total: number } }[],
  look: { split: boolean; bands: (tag: string) => readonly KeyBand[]; unknown: string; decimals?: number }): CountryRaceEntry[] {
  const decimals = look.decimals ?? 0;
  return ranked.map(({ tag, at }) => {
    const series = model.series.get(tag)!;
    const parts = () => {
      const parts = look.bands(tag).map(band => ({ id: band.id, color: band.color, value: bandValue(series, band, at.index) }));
      const shown = parts.filter(part => !Number.isNaN(part.value));
      const rest = at.total - shown.reduce((sum, part) => sum + part.value, 0);
      return shown.length === parts.length ? shown : [...shown, { id: 'unknown', color: look.unknown, value: 0, hatched: rest }];
    };
    return {
      tag, value: plotted(at.total, decimals), label: formatRaw(at.total, decimals), stretch: at.stretch,
      ...(look.split ? { parts: parts() } : {}),
    };
  });
}

/** Values over time as plotting runs: an unknown sample leaves a gap and the next
 * known one starts a new line; equal neighbours merge. */
function valueRuns(series: ProducedSeriesModel, pick: (index: number) => number): Run[] {
  const runs: Run[] = [];
  let broken = true;
  series.samples.forEach((sample, index) => {
    const value = pick(index);
    if (Number.isNaN(value)) { broken = true; return; }
    const previous = runs.at(-1);
    const breakBefore = broken || sample.breakBefore;
    if (previous && !breakBefore && previous.lastHour + 1 === sample.firstHour && previous.value === value) previous.lastHour = sample.lastHour;
    else runs.push({ firstHour: sample.firstHour, lastHour: sample.lastHour, value, ...(breakBefore ? { breakBefore: true } : {}) });
    broken = false;
  });
  return runs;
}

/** A country's value over time, cut to the hours it is shown (`membership`: a group's
 * hours; null: whenever recorded) and to the hours the widget shows (`range`), as the
 * race kit's `shownRuns` cuts. */
export function totalRuns(model: ProducedModel, tag: string, membership: readonly Span[] | null, range: Span, decimals = 0): Run[] {
  const series = model.series.get(tag);
  return series ? shownRuns(valueRuns(series, index => plotted(valueAt(series.total, index), decimals)), membership, range) : [];
}

/** One country's bands as runs to stack, cut as `totalRuns` cuts. */
export function mixBands(model: ProducedModel, tag: string, bands: readonly KeyBand[], membership: readonly Span[] | null, range: Span): Run[][] {
  const series = model.series.get(tag);
  if (!series) return bands.map(() => []);
  return bands.map(band => shownRuns(valueRuns(series, index => bandValue(series, band, index)), membership, range));
}

/** Where the running totals began counting at an hour: the latest observation then. */
export function countedSince(reply: EquipmentProducedReply, hour: number): number | null {
  return readAt(reply.observations, hour);
}

/** The tooltip for a country at an hour, fullest first: every key it made (most
 * first, unknown ones last), then the bands alone, each with the total and when the
 * value was read. */
export function* producedTooltips(model: ProducedModel, tag: string, name: string, at: ProducedAt, hour: number, date: string,
  bands: readonly KeyBand[], what?: string, decimals = 0): Generator<TooltipContent> {
  const series = model.series.get(tag)!;
  const measure = MEASURES.find(item => item.value === model.reply.measure) ?? { label: 'Per week' };
  const formatUnits = (raw: number) => formatRaw(raw, decimals);
  const colorOf = new Map(bands.flatMap(band => band.keys.map(key => [key, band.color] as const)));
  const since = model.reply.measure === 'total' ? countedSince(model.reply, hour) : null;
  const notes = since !== null ? [`Counted since ${formatDateTime(since)}.`] : [];
  const total: TooltipRow = { label: 'Total', value: at.total === null ? 'Not known' : formatUnits(at.total), strong: true, divider: true };
  const base = { title: name, subtitle: `${date} · ${what ?? measure.label}`, ...(notes.length ? { note: notes.join(' ') } : {}) };
  const parts = [...series.values.keys()].flatMap(key => {
    const value = groupValue(series, key, at.index);
    return value !== 0 ? [{ key, value: known(value) }] : [];
  }).sort((a, b) => (a.value === null ? 1 : 0) - (b.value === null ? 1 : 0) || (b.value ?? 0) - (a.value ?? 0) || a.key - b.key);
  const row = (label: string, value: number | null, swatch: string | undefined): TooltipRow =>
    ({ label, value: value === null ? 'Not known' : formatUnits(value), ...(swatch ? { swatch } : {}), ...(value === null ? { tone: 'muted' as const } : {}) });
  const empty: TooltipRow[] = parts.length ? [] : [{ label: 'Nothing delivered', tone: 'muted' }];
  const equipment = parts.flatMap(part => {
    if (part.value !== null) return [];
    const subtotal = equipmentGroupValue(series, part.key, at.index);
    return Number.isNaN(subtotal) ? [] : [row(`${keyLabel(model, part.key, hour)} — Equipment lines`, subtotal, colorOf.get(part.key))];
  });
  const detailNote = equipment.length ? `${base.note ?? ''} Total unavailable; equipment subtotal shown.`.trim() : base.note;
  yield { ...base, note: detailNote, rows: [...empty, ...parts.map(part => row(keyLabel(model, part.key, hour), part.value, colorOf.get(part.key))), ...equipment, total] };
  const shown = bands.map(band => ({ band, value: known(bandValue(series, band, at.index)) })).filter(item => item.value !== 0);
  yield { ...base, note: detailNote, rows: [...empty, ...shown.map(item => row(bandLabel(model, item.band, hour), item.value, item.band.color)), total] };
}
