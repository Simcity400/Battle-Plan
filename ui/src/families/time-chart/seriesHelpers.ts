import { rankSource } from '../seriesRank.ts';
import { CHART_MEASURE_COLOURS, colour, nudgeApart, readableColour } from '../../charts/palette.ts';
import { auditRow, type FamilyAuditRow } from '../auditRow.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { CELL, cellState, cellValue, rowReason } from '../../dashboard/follow/message.ts';
import { followedRow, keyId } from '../../dashboard/keyedSeriesData.ts';
import { answerRowOf } from '../sceneTiles.ts';
import { flagChoiceArt } from '../art/provider.ts';
import { unavailableArt } from '../art/types.ts';
import { FutureDataError, type BoundedReadView, type BoundedSeries, type FamilyPlanContext, type SourceEntry } from '../contract.ts';
import { holderRunAt, type HolderRun, type HourRanges } from '../holders.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { identityName, LOADING_NAME } from '../table/identityCell.ts';
import type { ChartArtSlot, ChartEndLabel, ChartGap, ChartIdentityRun, ChartMarker, ChartMarkerItem, ChartMissingSpan, ChartSeriesLook, ChartStackBand, ChartStackModel } from './model.ts';
import type { PowerSpendMarker, PowerSpendMarkersParams, PowerSpendMarkersReply, SeriesRankReply } from '../../generated/dashboard';
import { formatCompactRational, formatCompactScalar, formatDateOnly, formatRational, formatScalar } from '../valueFormat.ts';

export interface RationalScale {
  readonly numerator?: bigint;
  readonly denominator?: bigint;
}

export interface KnownHeadReading {
  readonly value: number;
  readonly raw?: bigint;
  readonly denominator?: bigint;
}

export type HeadReading = KnownHeadReading | { readonly missing: string };

export interface HeadKey {
  readonly tag: string;
  readonly part?: string;
  readonly partBits?: number;
  readonly reading?: number;
}

interface PanelState {
  readonly country: string | number;
  readonly part?: string;
  readonly run: { readonly valueRaw: string } | null;
  readonly missing: string | null;
}

interface RankPanel {
  readonly measures: readonly { readonly decimals: number }[];
  readonly rows: readonly { readonly country: string | number; readonly readings: readonly (PanelState | undefined)[] }[];
}

const NOT_READ_AT_HOUR = 'Not recorded at this hour';

function scaledReading(rawInt: bigint, decimals: number, scale: number | RationalScale): KnownHeadReading | null {
  if (typeof scale === 'object') {
    const raw = rawInt * (scale.numerator ?? 1n);
    const denominator = (10n ** BigInt(decimals)) * (scale.denominator ?? 1n);
    const value = Number(raw) / Number(denominator);
    return Number.isFinite(value) ? { value, raw, denominator } : null;
  }
  if (Number.isSafeInteger(scale)) {
    const raw = rawInt * BigInt(scale);
    const denominator = 10n ** BigInt(decimals);
    const value = Number(raw) / Number(denominator);
    return Number.isFinite(value) ? { value, raw, denominator } : null;
  }
  const value = (Number(rawInt) / 10 ** decimals) * scale;
  return Number.isFinite(value) ? { value } : null;
}

export function rankedPanelKeys(readView: BoundedReadView, name: string): readonly string[] {
  const panel = readView.panel<Partial<RankPanel>>(name);
  return panel?.rows?.map(row => countryKey(row.country)) ?? [];
}

export function headReading(
  readView: BoundedReadView,
  name: string,
  key: HeadKey,
  decimals: number,
  scale: number | RationalScale = 1,
): HeadReading | null {
  const bounded = readView.series(name)?.get(key.part === undefined ? key.tag : keyId({ tag: key.tag, part: key.part }));
  const places = bounded?.decimals ?? decimals;
  const answer = readView.answer(name);
  if (answer) {
    if (answer.kind === 'notRecorded') return { missing: answer.reason };
    const row = key.part === undefined
      ? answerRowOf(answer, key.tag)
      : followedRow(answer, { tag: key.tag, part: key.part }, key.partBits ?? 0);
    if (row >= 0) {
      const at = row * answer.columns;
      if (row >= answer.rows || cellState(answer, at) !== CELL.value) {
        return { missing: rowReason(answer, row) ?? NOT_READ_AT_HOUR };
      }
      return scaledReading(cellValue(answer, at), places, scale) ?? { missing: rowReason(answer, row) ?? NOT_READ_AT_HOUR };
    }
  }
  const panel = readView.panel<{ readonly decimals?: number; readonly states?: readonly PanelState[] } & Partial<RankPanel>>(name);
  const ranked = panel?.rows?.find(row => countryKey(row.country) === key.tag);
  if (ranked) {
    const at = key.reading ?? 0;
    const reading = ranked.readings[at];
    if (!reading?.run) return { missing: reading?.missing ?? NOT_READ_AT_HOUR };
    return scaledReading(BigInt(reading.run.valueRaw), panel?.measures?.[at]?.decimals ?? places, scale) ?? { missing: NOT_READ_AT_HOUR };
  }
  const state = panel?.states?.find(item => countryKey(item.country) === key.tag && (key.part === undefined || item.part === key.part));
  if (state) {
    if (!state.run) return { missing: state.missing ?? NOT_READ_AT_HOUR };
    return scaledReading(BigInt(state.run.valueRaw), panel?.decimals ?? places, scale) ?? { missing: NOT_READ_AT_HOUR };
  }
  const head = bounded?.headValue;
  if (!head) return null;
  if (head.kind === 'notRecorded' || head.kind === 'notApplicable') return { missing: head.reason };
  if (typeof head.value === 'bigint') {
    return scaledReading(head.value, 0, scale) ?? { missing: NOT_READ_AT_HOUR };
  }
  if (places === 0 && typeof head.value === 'number' && Number.isSafeInteger(head.value)) {
    return scaledReading(BigInt(head.value), 0, scale) ?? { missing: NOT_READ_AT_HOUR };
  }
  const factor = typeof scale === 'object' ? Number(scale.numerator ?? 1n) / Number(scale.denominator ?? 1n) : scale;
  const value = Number(head.value) * factor;
  return Number.isFinite(value) ? { value } : { missing: NOT_READ_AT_HOUR };
}

export const headValueOf = (reading: HeadReading | null | undefined): number | null =>
  reading != null && 'value' in reading ? reading.value : null;

export function formatPowerAmount(reading: KnownHeadReading, decimals: number): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    const absWhole = (reading.raw < 0n ? -reading.raw : reading.raw) / reading.denominator;
    return absWhole >= 10_000n
      ? formatCompactRational(reading.raw, reading.denominator, { maxDecimals: 0 }).text
      : formatRational(reading.raw, reading.denominator, { maxDecimals: decimals, fixedDecimals: true });
  }
  return Math.abs(reading.value) >= 1e4
    ? formatCompactScalar(Math.trunc(reading.value), 'integer').text
    : formatScalar(reading.value, { decimals, maxDecimals: decimals, fixedDecimals: true });
}

export function auditReading(
  row: string,
  reading: HeadReading | null,
  format: (reading: KnownHeadReading) => string,
): FamilyAuditRow {
  return auditRow(row, reading === null ? NOT_READ_AT_HOUR : 'value' in reading ? format(reading) : reading.missing);
}

export interface ExactReading {
  readonly hour: number;
  readonly value: number | null;
  readonly raw?: bigint;
  readonly denominator?: bigint;
  readonly reason?: string;
}

interface ExactState {
  readonly country?: string | number;
  readonly hour: number;
  readonly run: { readonly valueRaw: string } | null;
  readonly missing: string | null;
}

export function exactHoverReadings(
  readView: BoundedReadView,
  name: string,
  decimals: number,
  scale: number | RationalScale = 1,
): readonly (ExactReading | undefined)[] | null {
  let hover;
  try {
    hover = readView.hover<readonly ExactState[] | { readonly states?: readonly ExactState[]; readonly decimals?: number }>(name);
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !hover.data) return null;
  const data = hover.data;
  const list: readonly ExactState[] | null = Array.isArray(data)
    ? data
    : 'states' in data && Array.isArray(data.states)
      ? data.states
      : null;
  if (!list) return null;
  const usedDecimals = !Array.isArray(data) && 'decimals' in data && typeof data.decimals === 'number' ? data.decimals : decimals;
  return list.map(state => {
    if (!state) return undefined;
    if (!state.run) return { hour: hover.hour, value: null, reason: state.missing ?? 'Not recorded' };
    const reading = scaledReading(BigInt(state.run.valueRaw), usedDecimals, scale);
    if (!reading) return { hour: hover.hour, value: null, reason: state.missing ?? 'Not recorded' };
    return {
      hour: hover.hour,
      value: reading.value,
      ...(reading.raw !== undefined && reading.denominator !== undefined ? { raw: reading.raw, denominator: reading.denominator } : {}),
    };
  });
}

export function exactHoverByCountry(
  readView: BoundedReadView,
  name: string,
  decimals: number,
): ReadonlyMap<string, ExactReading> | null {
  let hover;
  try {
    hover = readView.hover<{ readonly states?: readonly ExactState[]; readonly decimals?: number }>(name);
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  const states = hover?.data?.states;
  if (!hover || !Array.isArray(states)) return null;
  const usedDecimals = typeof hover.data.decimals === 'number' ? hover.data.decimals : decimals;
  const out = new Map<string, ExactReading>();
  for (const state of states) {
    if (!state || state.country === undefined) continue;
    if (!state.run) {
      out.set(countryKey(state.country), { hour: hover.hour, value: null, reason: state.missing ?? 'Not recorded' });
      continue;
    }
    const reading = scaledReading(BigInt(state.run.valueRaw), usedDecimals, 1);
    out.set(countryKey(state.country), reading
      ? {
          hour: hover.hour,
          value: reading.value,
          ...(reading.raw !== undefined && reading.denominator !== undefined ? { raw: reading.raw, denominator: reading.denominator } : {}),
        }
      : { hour: hover.hour, value: null, reason: state.missing ?? 'Not recorded' });
  }
  return out;
}

export interface CountrySeriesLook extends ChartSeriesLook {
  readonly identities: readonly ChartIdentityRun[];
}

export interface CountrySeriesLooksInput {
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null | undefined;
  readonly from: number;
  readonly surface: string;
  readonly withFlag: boolean;
  readonly recordedOrdinal: (tag: string) => number;
  readonly singleColor?: string;
  readonly tailColor?: string;
}

interface IdentityStretch {
  readonly firstHour: number;
  lastHour: number;
  readonly identityTag: string;
  readonly name: string;
  readonly mapColour: string | null;
  readonly flagChoice: string | null;
  readonly flag?: ChartArtSlot;
}

const COLOURED_SERIES = 8;
const FALLBACK_RGB = { r: 120, g: 160, b: 220 };

function identityStretches(
  key: string,
  holders: readonly HolderRun[] | undefined,
  readView: BoundedReadView,
  from: number,
  hour: number,
  withFlag: boolean,
): IdentityStretch[] {
  const out: IdentityStretch[] = [];
  let at = Math.min(from, hour);
  while (at <= hour) {
    const run = holderRunAt(holders, at);
    const identityTag = run?.tag ?? key;
    const who = readView.who('country', identityTag, at);
    let last = who.validInterval[1] > at && who.name.status !== 'pending' ? Math.min(hour, who.validInterval[1]) : hour;
    if (run) last = Math.min(last, run.lastHour);
    else if (holders) {
      for (const next of holders) {
        if (next.firstHour > at) {
          last = Math.min(last, next.firstHour - 1);
          break;
        }
      }
    }
    const name = who.name.status === 'ready' ? who.name.value : who.name.status === 'pending' ? LOADING_NAME : identityTag;
    const mapColour = who.colour?.status === 'ready' ? who.colour.value : null;
    const flagChoice = who.flagChoice?.status === 'ready' ? who.flagChoice.value : null;
    const prev = out[out.length - 1];
    if (prev && prev.identityTag === identityTag && prev.name === name && prev.mapColour === mapColour && prev.flagChoice === flagChoice) {
      prev.lastHour = last;
    } else {
      out.push({
        firstHour: at,
        lastHour: last,
        identityTag,
        name,
        mapColour,
        flagChoice,
        ...(withFlag ? { flag: { art: flagChoiceArt(readView, who.flagChoice), tag: identityTag } } : {}),
      });
    }
    at = last + 1;
  }
  return out;
}

export function countrySeriesLooks(
  ordered: readonly string[],
  readView: BoundedReadView,
  input: CountrySeriesLooksInput,
): ReadonlyMap<string, CountrySeriesLook> {
  const hour = readView.hour;
  const stretches = ordered.map(tag => identityStretches(tag, input.holders?.[tag], readView, input.from, hour, input.withFlag));
  const single = ordered.length === 1 && input.singleColor !== undefined;
  const bg = parseColor(input.surface) ?? { r: 17, g: 17, b: 17 };
  const coloured = Math.min(ordered.length, COLOURED_SERIES);
  const asked: { r: number; g: number; b: number }[] = [];
  const slotOf = new Map<string, number>();
  const ask = (tag: string, stretch: IdentityStretch) => {
    const hex = stretch.mapColour !== null ? readableColour(stretch.mapColour, input.surface) : colour(input.surface, input.recordedOrdinal(tag));
    asked.push(parseColor(hex) ?? FALLBACK_RGB);
  };
  const currentOf = (index: number) => stretches[index]![stretches[index]!.length - 1];
  if (!single) {
    for (let index = 0; index < coloured; index++) {
      const current = currentOf(index);
      if (current) ask(ordered[index]!, current);
    }
    for (let index = 0; index < coloured; index++) {
      const current = currentOf(index);
      for (const stretch of stretches[index]!) {
        if (stretch === current || stretch.mapColour === current?.mapColour) continue;
        const key = `${index}\u0000${stretch.mapColour}`;
        if (slotOf.has(key)) continue;
        slotOf.set(key, asked.length);
        ask(ordered[index]!, stretch);
      }
    }
  }
  const distinct = single ? [] : nudgeApart(asked, bg);
  const tailColor = input.tailColor ?? CHART_MEASURE_COLOURS.tailLine;
  const out = new Map<string, CountrySeriesLook>();
  let currentSlot = 0;
  ordered.forEach((tag, index) => {
    const current = currentOf(index);
    const currentColour = single
      ? input.singleColor!
      : index < coloured
        ? (current ? toHex(distinct[currentSlot++]!) : tailColor)
        : tailColor;
    const identities: ChartIdentityRun[] = stretches[index]!.map(stretch => {
      const historical = !single && index < coloured && stretch !== current && stretch.mapColour !== current?.mapColour;
      const color = historical ? toHex(distinct[slotOf.get(`${index}\u0000${stretch.mapColour}`)!]!) : currentColour;
      return { firstHour: stretch.firstHour, lastHour: stretch.lastHour, name: stretch.name, color, ...(stretch.flag ? { flag: stretch.flag } : {}) };
    });
    const now = identities[identities.length - 1];
    out.set(tag, {
      name: now?.name ?? tag,
      color: currentColour,
      ...(now?.flag ? { flag: now.flag } : {}),
      identities,
    });
  });
  return out;
}

export function countryNameAt(readView: BoundedReadView, key: string, hour: number): string {
  return identityName(readView, 'country', key, hour);
}

export function memberAt(
  membership: Readonly<Record<string, HourRanges>> | null | undefined,
  tag: string,
  hour: number,
): boolean {
  if (!membership) return true;
  const ranges = membership[tag];
  if (!ranges) return false;
  for (const [first, last] of ranges) {
    if (hour >= first && hour <= last) return true;
  }
  return false;
}

export function branchGapsOf(branch: unknown): ChartGap[] {
  const raw = (branch as { gaps?: readonly { firstHour: number; lastHour: number; reason?: string }[] } | null | undefined)?.gaps;
  return raw?.length
    ? raw.map(g => ({ firstHour: g.firstHour, lastHour: g.lastHour, reason: g.reason ?? 'Recording gap' }))
    : [];
}

export function withinMapOf(
  membership: Readonly<Record<string, readonly (readonly [number, number])[]>> | null | undefined,
): ReadonlyMap<string, readonly { readonly firstHour: number; readonly lastHour: number }[]> | undefined {
  if (!membership) return undefined;
  const entries = Object.entries(membership)
    .filter(([t]) => t.length > 0)
    .map(([t, ranges]) => [t, ranges.map(([firstHour, lastHour]) => ({ firstHour, lastHour }))] as const);
  return entries.length > 0 ? new Map(entries) : undefined;
}

export type SpendCurrency = PowerSpendMarkersParams['currency'];

export function spendMarkerEntry(ctx: FamilyPlanContext, currency: SpendCurrency): SourceEntry | null {
  const tags = ctx.countries.tags;
  if (tags.length !== 1 || ctx.options.spendTicks === false) return null;
  const params = { branchId: ctx.branch.id, country: countryFromKey(tags[0]!), currency };
  return {
    name: 'spending',
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'point',
      channel: 'panel',
      coverage: { mode: 'keyed', keys: tags },
      product: 'power-spend-markers',
      followQuery: { panel: 'power-spend-markers', ...params },
      params,
    },
  };
}

export interface SpendMarkers {
  readonly markers: readonly ChartMarker[];
  readonly warnings: readonly string[];
}

const formatSpend = (value: number) => `−${formatScalar(Math.abs(value), { decimals: 0, maxDecimals: 2 })}`;

function spendDetail(marker: PowerSpendMarker, decimals: number): string {
  const parts: string[] = [];
  if (marker.chargeRaw !== null) parts.push(formatSpend(Number(marker.chargeRaw) / 10 ** decimals));
  if (marker.unknown > 0) parts.push(marker.unknown === marker.payments ? 'Cost not recorded' : `${marker.unknown} costs not recorded`);
  if (marker.since !== null) parts.push(`${marker.payments} payments from ${formatDateOnly(marker.since)} to ${formatDateOnly(marker.hour)}`);
  if (!marker.dated) parts.push(`Paid ${formatDateOnly(marker.hour)}`);
  return parts.join(' · ');
}

export function spendMarkersOf(readView: BoundedReadView, title: string, extentStart: number): SpendMarkers {
  const reply = readView.panel<PowerSpendMarkersReply>('spending');
  if (!reply) return { markers: [], warnings: [] };
  const markers: ChartMarker[] = [];
  let at = 0;
  while (at < reply.markers.length) {
    const hour = reply.markers[at]!.hour;
    const day: ChartMarkerItem[] = [];
    for (; at < reply.markers.length && reply.markers[at]!.hour === hour; at++) {
      const marker = reply.markers[at]!;
      day.push({ title: marker.title, detail: spendDetail(marker, reply.decimals) });
    }
    if (hour < extentStart || hour > readView.hour) continue;
    markers.push({ id: `spend:${hour}`, hour, title: day.length === 1 ? day[0]!.title : title, items: day });
  }
  const warnings = reply.unrecorded.map(gap => `${title} not recorded ${gap.firstHour === gap.lastHour ? `on ${formatDateOnly(gap.firstHour)}` : `from ${formatDateOnly(gap.firstHour)} to ${formatDateOnly(gap.lastHour)}`}: ${gap.reason}`);
  return { markers, warnings };
}

export function missingSpansOf(bounded: BoundedSeries | null | undefined): readonly ChartMissingSpan[] | undefined {
  if (!bounded || bounded.missing.length === 0) return undefined;
  return [...bounded.missing].map(sp => ({
    firstHour: sp.firstHour,
    lastHour: sp.lastHour,
    reason: sp.reason ?? 'Not recorded',
  }));
}

export interface CardRow {
  readonly label: string;
  readonly value?: string;
  readonly tone?: TooltipListItem['tone'];
  readonly strong?: boolean;
  readonly swatch?: string;
  readonly divider?: boolean;
}

export function cardItems(rows: readonly CardRow[], heading: boolean): TooltipListItem[] {
  return rows.map((row, index) => ({
    label: row.label,
    ...(row.value !== undefined ? { value: row.value } : {}),
    ...(row.tone ? { tone: row.tone } : {}),
    ...(row.strong ? { strong: true } : {}),
    ...(row.swatch ? { swatch: row.swatch } : {}),
    ...(row.divider ? { divider: true } : {}),
    ...(heading && index > 0 && !row.strong ? { indent: true } : {}),
  }));
}

export function rankEntry(
  ctx: Pick<FamilyPlanContext, 'track' | 'branch'>,
  family: Readonly<Record<string, unknown>>,
  tags: readonly string[],
  name = 'rank',
): SourceEntry | null {
  if (tags.length < 2) return null;
  return rankSource(ctx, name, family, { mode: 'keyed', keys: tags }, tags)[0] ?? null;
}

export interface RankedOrder {
  readonly order: readonly string[];
  readonly warnings: readonly string[];
  readonly unnamed: ReadonlyMap<string, string>;
}

const NO_UNNAMED: ReadonlyMap<string, string> = new Map();

export function rankedOrder(readView: BoundedReadView, tags: readonly string[], name = 'rank'): RankedOrder {
  const reply = readView.panel<SeriesRankReply>(name);
  if (!reply) return { order: tags, warnings: [], unnamed: NO_UNNAMED };
  const selected = new Set(tags);
  const seen = new Set<string>();
  const order: string[] = [];
  const unnamed = new Map<string, string>();
  for (const row of reply.rows) {
    const key = countryKey(row.country);
    if (!selected.has(key) || seen.has(key)) continue;
    seen.add(key);
    order.push(key);
    if (row.unnamed) unnamed.set(key, row.unnamed);
  }
  for (const tag of tags) if (!seen.has(tag)) order.push(tag);
  return { order, warnings: reply.refused === null ? [] : [`Not in rank order: ${reply.refused}`], unnamed };
}

export const UNNAMED_COUNTRY = 'Country not recorded';

export function unnamedLooks(
  looks: ReadonlyMap<string, CountrySeriesLook>,
  unnamed: ReadonlyMap<string, string>,
  hour: number,
): ReadonlyMap<string, CountrySeriesLook> {
  if (unnamed.size === 0) return looks;
  const out = new Map(looks);
  for (const [tag, reason] of unnamed) {
    const look = looks.get(tag);
    if (!look) continue;
    const flag: ChartArtSlot | undefined = look.flag ? { art: unavailableArt(reason), tag } : undefined;
    const atHour = (run: ChartIdentityRun) => run.firstHour <= hour && hour <= run.lastHour;
    out.set(tag, {
      ...look,
      name: UNNAMED_COUNTRY,
      ...(flag ? { flag } : {}),
      identities: look.identities.map(run => (atHour(run) ? { ...run, name: UNNAMED_COUNTRY, ...(flag ? { flag } : {}) } : run)),
    });
  }
  return out;
}

export interface StackPart {
  readonly series?: BoundedSeries;
  readonly within?: readonly (readonly [number, number])[];
}

export interface BoundedStackBandInput {
  readonly id: string;
  readonly key: string;
  readonly name: string;
  readonly color: string;
  readonly opacity?: number;
  readonly parts: readonly StackPart[];
  readonly valueScale?: number;
  readonly head?: HeadReading | null;
  readonly endLabel?: ChartEndLabel | null;
  readonly flag?: ChartArtSlot;
}

const covers = (part: StackPart, hour: number) =>
  !part.within || part.within.some(([first, last]) => first <= hour && hour <= last);

function inMissing(series: BoundedSeries, hour: number): boolean {
  for (const span of series.missing) {
    if (hour >= span.firstHour && hour <= span.lastHour) return true;
  }
  return false;
}

function sampleOneBounded(series: BoundedSeries, hour: number, scale: number): number | null {
  if (inMissing(series, hour)) return null;
  if (series.runs && series.runs.length > 0) {
    let lo = 0;
    let hi = series.runs.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >>> 1;
      const run = series.runs.at(mid)!;
      if (hour < run.firstHour) hi = mid - 1;
      else if (hour > run.lastHour) lo = mid + 1;
      else return Number.isFinite(run.value) ? run.value * scale : null;
    }
    return null;
  }
  const idx = Math.floor((hour - series.from) / series.binHours);
  if (idx < 0 || idx >= series.buckets.length) return null;
  const b = series.buckets.at(idx);
  if (!b || b.hasBreak || hour < b.firstHour || hour > b.lastHour) return null;
  const raw = hour === b.lastHour ? b.lastValue : b.firstValue;
  return Number.isFinite(raw) ? raw * scale : null;
}

function collectSeriesHours(series: BoundedSeries, until: number, target: Set<number>): void {
  if (series.runs && series.runs.length > 0) {
    let prevLast = -Infinity;
    for (const run of series.runs) {
      if (run.firstHour > until) break;
      if (prevLast !== -Infinity && (run.breakBefore || run.firstHour > prevLast + 1)) {
        const gapHour = Math.min(until, prevLast + 1);
        if (gapHour < run.firstHour) target.add(gapHour);
      }
      target.add(run.firstHour);
      const end = Math.min(until, run.lastHour);
      if (end > run.firstHour) target.add(end);
      prevLast = end;
    }
  } else {
    let prevLast = -Infinity;
    for (const b of series.buckets) {
      if (!b || b.firstHour > until) continue;
      if (prevLast !== -Infinity && (b.firstBreak || b.firstHour > prevLast + Math.max(1, series.binHours))) {
        const gapHour = Math.min(until, prevLast + 1);
        if (gapHour < b.firstHour) target.add(gapHour);
      }
      target.add(b.firstHour);
      if (!b.hasBreak) {
        const end = Math.min(until, b.lastHour);
        if (end > b.firstHour) target.add(end);
      }
      prevLast = Math.min(until, b.lastHour);
    }
  }
  for (const span of series.missing) {
    if (span.firstHour <= until) target.add(span.firstHour);
  }
}

export function buildBoundedStack(inputs: readonly BoundedStackBandInput[], hour: number): ChartStackModel {
  const hourSet = new Set<number>();
  for (const input of inputs) {
    for (const part of input.parts) {
      if (part.series) collectSeriesHours(part.series, hour, hourSet);
      for (const [first, last] of part.within ?? []) {
        if (first <= hour) hourSet.add(first);
        if (last < hour) {
          hourSet.add(last);
          hourSet.add(last + 1);
        }
      }
    }
  }
  if (hourSet.size > 0 || inputs.some(input => input.head)) hourSet.add(hour);
  const hours = [...hourSet].filter(h => h <= hour).sort((a, b) => a - b);
  const valuesByBand: (number | null)[][] = inputs.map(() => new Array<number | null>(hours.length).fill(null));
  const row = new Array<number>(inputs.length);

  for (let i = 0; i < hours.length; i++) {
    const h = hours[i]!;
    let present = 0;
    let missing = false;
    for (let b = 0; b < inputs.length && !missing; b++) {
      const input = inputs[b]!;
      const owned = input.parts.filter(part => covers(part, h));
      if (owned.length === 0) {
        row[b] = 0;
        continue;
      }
      present++;
      let value: number | null = null;
      if (h === hour && input.head) {
        value = 'value' in input.head ? input.head.value : null;
      } else {
        const scale = input.valueScale ?? 1;
        for (const part of owned) {
          value = part.series ? sampleOneBounded(part.series, h, scale) : null;
          if (value !== null) break;
        }
      }
      if (value === null) missing = true;
      else row[b] = value;
    }
    if (missing || present === 0) continue;
    for (let b = 0; b < inputs.length; b++) valuesByBand[b]![i] = row[b]!;
  }

  const bands: ChartStackBand[] = inputs.map((input, idx) => ({
    id: input.id,
    key: input.key,
    name: input.name,
    color: input.color,
    ...(input.opacity !== undefined ? { opacity: input.opacity } : {}),
    values: valuesByBand[idx]!,
    ...(input.endLabel !== undefined ? { endLabel: input.endLabel } : {}),
    ...(input.flag ? { flag: input.flag } : {}),
  }));
  return { hours, bands };
}
