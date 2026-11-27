import { rankSource } from '../seriesRank.ts';
import { resolveLineColors, tailColour } from '../../charts/palette.ts';
import { auditRow, type FamilyAuditRow } from '../auditRow.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { CELL, cellState, cellValue, rowReason } from '../../dashboard/follow/message.ts';
import { followedRow, keyId } from '../../dashboard/keyedSeriesData.ts';
import { answerRowOf, followedCountryRank } from '../sceneTiles.ts';
import { flagChoiceArt } from '../art/provider.ts';
import { unavailableArt } from '../art/types.ts';
import { FutureDataError, type ArtProvider, type BoundedReadView, type BoundedSeries, type FamilyPlanContext, type IdentityProvider, type SourceEntry } from '../contract.ts';
import type { ArtResolutions } from '../art/provider.ts';
import type { IdentityIndex } from '../identity/index.ts';
import { sceneReadRecorder, type CapturedSceneReads } from '../scene.ts';
import { timeChartCacheState } from './cacheControl.ts';
import { holderRunAt, type HolderRun, type HourRanges } from '../holders.ts';
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
  readonly rankRow?: number;
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

export function chartTagsOf(countries: FamilyPlanContext['countries']): readonly string[] {
  return countries.all ? countries.every : countries.tags;
}

export function rowsBySelectedKey<R extends { readonly tag: string | null }>(rows: readonly R[], slotKey: (row: R) => string): (key: string) => R | undefined {
  const bySlot = new Map(rows.map(row => [slotKey(row), row]));
  const byTag = new Map(rows.flatMap(row => (row.tag ? [[row.tag, row] as const] : [])));
  return key => bySlot.get(key) ?? byTag.get(key);
}

export function rankedCountries(readView: BoundedReadView, name: string) {
  const answer = readView.answer(name);
  if (answer && answer.kind !== 'notRecorded') return followedCountryRank(answer);
  const panel = readView.panel<SeriesRankReply>(name);
  if (!panel) return null;
  const rows = new Map<string, number>();
  const tags = new Map<string, string>();
  const unnamed = new Map<string, string>();
  panel.rows.forEach((item, row) => {
    const key = countryKey(item.slot ?? item.country);
    rows.set(key, row);
    if (item.unnamed) unnamed.set(key, item.unnamed);
    else if (typeof item.country === 'string') tags.set(key, item.country);
  });
  return { rows, tags, unnamed };
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
  const column = key.reading ?? 0;
  if (answer) {
    if (answer.kind === 'notRecorded') return { missing: answer.reason };
    const row = key.rankRow ?? (key.part === undefined
      ? answerRowOf(answer, key.tag)
      : followedRow(answer, { tag: key.tag, part: key.part }, key.partBits ?? 0));
    if (row >= 0) {
      if (column >= answer.columns) return { missing: NOT_READ_AT_HOUR };
      const at = row * answer.columns + column;
      if (row >= answer.rows || cellState(answer, at) !== CELL.value) {
        return { missing: rowReason(answer, row) ?? NOT_READ_AT_HOUR };
      }
      return scaledReading(cellValue(answer, at), places, scale) ?? { missing: rowReason(answer, row) ?? NOT_READ_AT_HOUR };
    }
  }
  const panel = readView.panel<{ readonly decimals?: number; readonly states?: readonly PanelState[] } & Partial<RankPanel>>(name);
  const ranked = key.rankRow === undefined ? panel?.rows?.find(row => countryKey(row.country) === key.tag) : panel?.rows?.[key.rankRow];
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
  const head = column === 0 ? bounded?.headValue : undefined;
  if (!head) return null;
  if (head.kind === 'notRecorded' || head.kind === 'notApplicable' || head.kind === 'pending') return { missing: head.reason };
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
  readonly colouredCount?: number;
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

interface IdentityStretchesResult {
  readonly stretches: IdentityStretch[];
  readonly validFrom: number;
  readonly validThrough: number;
  readonly pending: boolean;
}

function identityStretches(
  key: string,
  holders: readonly HolderRun[] | undefined,
  readView: BoundedReadView,
  from: number,
  hour: number,
  withFlag: boolean,
): IdentityStretchesResult {
  const out: IdentityStretch[] = [];
  let at = Math.min(from, hour);
  let validFrom = from;
  let validThrough = Infinity;
  let pending = false;
  while (at <= hour) {
    const run = holderRunAt(holders, at);
    const identityTag = run?.tag ?? key;
    const who = readView.who('country', identityTag, at);
    if (
      who.holder.status === 'pending'
      || who.name.status === 'pending'
      || who.tag?.status === 'pending'
      || who.colour?.status === 'pending'
      || who.flagChoice?.status === 'pending'
    ) {
      pending = true;
    }
    const settled = who.name.status === 'ready' || who.name.status === 'absent';
    let uncappedLast = settled && who.validInterval[1] >= at ? who.validInterval[1] : hour;
    if (run) uncappedLast = Math.min(uncappedLast, run.lastHour);
    else if (holders) {
      for (const next of holders) {
        if (next.firstHour > at) {
          uncappedLast = Math.min(uncappedLast, next.firstHour - 1);
          break;
        }
      }
    }
    const last = Math.min(hour, uncappedLast);
    if (last === hour) {
      validFrom = Math.max(validFrom, at);
      validThrough = uncappedLast;
    }
    const name = who.name.status === 'ready' ? who.name.value : who.name.status === 'pending' ? LOADING_NAME : who.tag?.status === 'ready' ? who.tag.value : identityTag;
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
  return { stretches: out, validFrom, validThrough, pending };
}

interface CachedCountryLooks {
  readonly identity: IdentityProvider | undefined;
  readonly art: ArtProvider | undefined;
  readonly index: IdentityIndex | null | undefined;
  readonly artResolutions: ArtResolutions | undefined;
  readonly artRevision: number;
  readonly ordered: readonly string[];
  readonly holdersRef: Readonly<Record<string, readonly HolderRun[]>> | null | undefined;
  readonly holderRuns: readonly (readonly HolderRun[] | undefined)[];
  readonly from: number;
  readonly surface: string;
  readonly withFlag: boolean;
  readonly colouredCount: number | undefined;
  readonly singleColor: string | undefined;
  readonly tailColor: string | undefined;
  readonly validFrom: number;
  readonly validThrough: number;
  readonly reads: CapturedSceneReads;
  readonly base: ReadonlyMap<string, CountrySeriesLook>;
  lastHour: number;
  lastMap: ReadonlyMap<string, CountrySeriesLook>;
}

const COUNTRY_LOOKS_CACHE_MAX = 32;
const countryLooksCache: CachedCountryLooks[] = [];

function sameOrderedStrings(a: readonly string[], b: readonly string[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function sameHolderRuns(
  entry: CachedCountryLooks,
  ordered: readonly string[],
  holders: Readonly<Record<string, readonly HolderRun[]>> | null | undefined,
): boolean {
  if (entry.holdersRef === holders) return true;
  if (!entry.holdersRef && !holders) return true;
  for (let i = 0; i < ordered.length; i++) {
    if (entry.holderRuns[i] !== holders?.[ordered[i]!]) return false;
  }
  return true;
}

function extendLooksToHour(base: ReadonlyMap<string, CountrySeriesLook>, hour: number): ReadonlyMap<string, CountrySeriesLook> {
  const out = new Map<string, CountrySeriesLook>();
  for (const [tag, look] of base) {
    const len = look.identities.length;
    if (len === 0 || look.identities[len - 1]!.lastHour === hour) {
      out.set(tag, look);
    } else {
      const identities = look.identities.slice();
      const tail = identities[len - 1]!;
      identities[len - 1] = { ...tail, lastHour: hour };
      out.set(tag, { ...look, identities });
    }
  }
  return out;
}

function computeCountrySeriesLooks(
  ordered: readonly string[],
  readView: BoundedReadView,
  input: CountrySeriesLooksInput,
): { readonly looks: ReadonlyMap<string, CountrySeriesLook>; readonly validFrom: number; readonly validThrough: number; readonly pending: boolean } {
  const hour = readView.hour;
  let validFrom = input.from;
  let validThrough = Infinity;
  let pending = false;
  const stretches: IdentityStretch[][] = new Array(ordered.length);
  for (let i = 0; i < ordered.length; i++) {
    const tag = ordered[i]!;
    const res = identityStretches(tag, input.holders?.[tag], readView, input.from, hour, input.withFlag);
    stretches[i] = res.stretches;
    if (res.validFrom > validFrom) validFrom = res.validFrom;
    if (res.validThrough < validThrough) validThrough = res.validThrough;
    if (res.pending) pending = true;
  }
  const coloured = stretches.slice(0, Math.min(COLOURED_SERIES, Math.max(0, input.colouredCount ?? COLOURED_SERIES)));
  const keys = ordered.slice(0, coloured.length).map(countryFromKey);
  const positions = coloured.map(() => 0);
  const runs = coloured.map((): ChartIdentityRun[] => []);
  for (let first = Math.min(input.from, hour); coloured.length > 0 && first <= hour;) {
    const active = coloured.map((history, i) => history[positions[i]!]!);
    const last = Math.min(...active.map(stretch => stretch.lastHour));
    const colors = ordered.length === 1 && input.singleColor !== undefined
      ? new Map([[keys[0]!, input.singleColor]])
      : resolveLineColors(active.map((stretch, i) => ({ key: keys[i]!, color: stretch.mapColour })), input.surface);
    active.forEach((stretch, i) => {
      const color = colors.get(keys[i]!)!;
      const line = runs[i]!;
      const previous = line.at(-1);
      if (previous && previous.lastHour >= stretch.firstHour && previous.color === color) {
        line[line.length - 1] = { ...previous, lastHour: last };
      } else {
        line.push({ firstHour: first, lastHour: last, name: stretch.name, color, ...(stretch.flag ? { flag: stretch.flag } : {}) });
      }
      if (stretch.lastHour === last) positions[i] = positions[i]! + 1;
    });
    first = last + 1;
  }
  const tail = input.tailColor ?? tailColour(input.surface);
  const out = new Map<string, CountrySeriesLook>();
  ordered.forEach((tag, i) => {
    const identities = runs[i] ?? stretches[i]!.map(stretch => ({
      firstHour: stretch.firstHour, lastHour: stretch.lastHour, name: stretch.name, color: tail, ...(stretch.flag ? { flag: stretch.flag } : {}),
    }));
    const now = identities.at(-1);
    out.set(tag, { name: now?.name ?? tag, color: now?.color ?? tail, ...(now?.flag ? { flag: now.flag } : {}), identities });
  });
  return { looks: out, validFrom, validThrough, pending };
}

export function countrySeriesLooks(
  ordered: readonly string[],
  readView: BoundedReadView,
  input: CountrySeriesLooksInput,
): ReadonlyMap<string, CountrySeriesLook> {
  if (ordered.length === 0) return new Map();
  const rec = timeChartCacheState.enabled ? sceneReadRecorder(readView) : undefined;
  const artRevision = rec?.readers.artRevision;
  const hour = readView.hour;
  if (rec && artRevision !== undefined && hour >= input.from) {
    for (let i = countryLooksCache.length - 1; i >= 0; i--) {
      const entry = countryLooksCache[i]!;
      if (
        entry.artRevision === artRevision
        && entry.identity === rec.readers.identity
        && entry.art === rec.readers.art
        && entry.index === rec.readers.index
        && entry.artResolutions === rec.readers.artResolutions
        && entry.from === input.from
        && entry.surface === input.surface
        && entry.withFlag === input.withFlag
        && entry.colouredCount === input.colouredCount
        && entry.singleColor === input.singleColor
        && entry.tailColor === input.tailColor
        && hour >= entry.validFrom
        && hour <= entry.validThrough
        && sameOrderedStrings(entry.ordered, ordered)
        && sameHolderRuns(entry, ordered, input.holders)
      ) {
        rec.replay(entry.reads);
        if (i !== countryLooksCache.length - 1) {
          countryLooksCache.splice(i, 1);
          countryLooksCache.push(entry);
        }
        if (entry.lastHour === hour) return entry.lastMap;
        entry.lastHour = hour;
        entry.lastMap = extendLooksToHour(entry.base, hour);
        return entry.lastMap;
      }
    }
    const captured = rec.capture(() => computeCountrySeriesLooks(ordered, readView, input));
    const { looks, validFrom, validThrough, pending } = captured.value;
    if (!pending && validThrough >= hour) {
      if (countryLooksCache.length >= COUNTRY_LOOKS_CACHE_MAX) countryLooksCache.shift();
      countryLooksCache.push({
        identity: rec.readers.identity,
        art: rec.readers.art,
        index: rec.readers.index,
        artResolutions: rec.readers.artResolutions,
        artRevision,
        ordered: ordered.slice(),
        holdersRef: input.holders,
        holderRuns: ordered.map(tag => input.holders?.[tag]),
        from: input.from,
        surface: input.surface,
        withFlag: input.withFlag,
        colouredCount: input.colouredCount,
        singleColor: input.singleColor,
        tailColor: input.tailColor,
        validFrom,
        validThrough,
        reads: captured.reads,
        base: looks,
        lastHour: hour,
        lastMap: looks,
      });
    }
    return looks;
  }
  return computeCountrySeriesLooks(ordered, readView, input).looks;
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
  if (marker.chargeRaw !== null && marker.unknown < marker.payments) parts.push(formatSpend(Number(marker.chargeRaw) / 10 ** decimals));
  if (marker.unknown > 0) parts.push(marker.unknown === marker.payments ? 'Cost not recorded' : `${marker.unknown} costs not recorded`);
  if (marker.since !== null) parts.push(`${marker.payments} payments from ${formatDateOnly(marker.since)} to ${formatDateOnly(marker.hour)}`);
  if (!marker.dated) parts.push(`Paid by ${formatDateOnly(marker.hour)}`);
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
  const warnings = reply.unrecorded.filter(gap => gap.firstHour <= readView.hour).map(gap => {
    const last = Math.min(gap.lastHour, readView.hour);
    return `${title} not recorded ${gap.firstHour === last ? `on ${formatDateOnly(gap.firstHour)}` : `from ${formatDateOnly(gap.firstHour)} to ${formatDateOnly(last)}`}: ${gap.reason}`;
  });
  return { markers, warnings };
}

const missingSpansCache = new WeakMap<object, readonly ChartMissingSpan[]>();
const partialSpansCache = new WeakMap<object, readonly ChartMissingSpan[]>();

export function missingSpansOf(bounded: BoundedSeries | null | undefined): readonly ChartMissingSpan[] | undefined {
  if (!bounded || bounded.missing.length === 0) return undefined;
  const cached = missingSpansCache.get(bounded.missing);
  if (cached) return cached;
  const out = [...bounded.missing].map(sp => ({
    firstHour: sp.firstHour,
    lastHour: sp.lastHour,
    reason: sp.reason ?? 'Not recorded',
  }));
  missingSpansCache.set(bounded.missing, out);
  return out;
}

export function partialSpansOf(bounded: BoundedSeries | null | undefined): readonly ChartMissingSpan[] | undefined {
  if (!bounded?.partial || bounded.partial.length === 0) return undefined;
  const cached = partialSpansCache.get(bounded.partial);
  if (cached) return cached;
  const out = [...bounded.partial].map(sp => ({
    firstHour: sp.firstHour,
    lastHour: sp.lastHour,
    reason: sp.reason ?? 'Partial',
  }));
  partialSpansCache.set(bounded.partial, out);
  return out;
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
  headTags?: ReadonlyMap<string, string>,
): ReadonlyMap<string, CountrySeriesLook> {
  if (unnamed.size === 0 && !headTags?.size) return looks;
  const out = new Map(looks);
  for (const [key, look] of looks) {
    const reason = unnamed.get(key);
    const name = reason ? UNNAMED_COUNTRY : headTags?.get(key);
    if (!name || (!reason && look.name !== key && look.name !== LOADING_NAME)) continue;
    const flag = reason && look.flag ? { art: unavailableArt(reason), tag: key } : look.flag;
    const identities = look.identities.slice();
    const last = identities.pop();
    if (last) {
      const firstHour = reason ? last.firstHour : hour;
      if (last.firstHour < firstHour) identities.push({ ...last, lastHour: firstHour - 1 });
      identities.push({ ...last, firstHour, name, flag });
    }
    out.set(key, { ...look, name, flag, identities });
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
  const missing = series.missing;
  const len = missing.length;
  for (let i = 0; i < len; i++) {
    const span = missing.at(i)!;
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

function buildBoundedStackUncached(inputs: readonly BoundedStackBandInput[], hour: number): ChartStackModel {
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
  const outsideByBand: (boolean[] | null)[] = inputs.map(() => null);
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
        (outsideByBand[b] ??= new Array<boolean>(hours.length).fill(false))[i] = true;
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
    ...(outsideByBand[idx] ? { outside: outsideByBand[idx]! } : {}),
    ...(input.endLabel !== undefined ? { endLabel: input.endLabel } : {}),
    ...(input.flag ? { flag: input.flag } : {}),
  }));
  return { hours, bands };
}

interface CachedStackPart {
  readonly hasSeries: boolean;
  readonly useRuns: boolean;
  readonly sliceSource: object | null;
  readonly missingSource: object | null;
  readonly from: number;
  readonly binHours: number;
  readonly within: readonly (readonly [number, number])[] | null;
  scannedClosed: number;
  prevLast: number;
  nextMissingIdx: number;
  straddlingBucketIdx: number;
  tailRunActive: boolean;
  tailRunIdx: number;
  tailRunFirst: number;
  tailRunMinLast: number;
  tailRunValue: number;
  tailRunBreakBefore: boolean;
}

interface CachedStackBand {
  readonly valueScale: number;
  readonly parts: readonly CachedStackPart[];
}

interface CachedBoundedStack {
  readonly bands: readonly CachedStackBand[];
  hasClosedPrefix: boolean;
  closedThrough: number;
  hours: number[];
  valuesByBand: (number | null)[][];
  outsideByBand: (boolean[] | null)[];
}

const STACK_CACHE_MAX = 8;
const stackPrefixCache: CachedBoundedStack[] = [];

function sameWithinRanges(
  cached: readonly (readonly [number, number])[] | null,
  live: readonly (readonly [number, number])[] | undefined,
): boolean {
  if (cached === null) return live === undefined;
  if (live === undefined || cached.length !== live.length) return false;
  for (let i = 0; i < cached.length; i++) {
    if (cached[i]![0] !== live[i]![0] || cached[i]![1] !== live[i]![1]) return false;
  }
  return true;
}

function matchesStackCache(entry: CachedBoundedStack, inputs: readonly BoundedStackBandInput[]): boolean {
  if (entry.bands.length !== inputs.length) return false;
  for (let b = 0; b < inputs.length; b++) {
    const input = inputs[b]!;
    const cb = entry.bands[b]!;
    if (!Object.is(cb.valueScale, input.valueScale ?? 1) || cb.parts.length !== input.parts.length) return false;
    for (let p = 0; p < input.parts.length; p++) {
      const part = input.parts[p]!;
      const cp = cb.parts[p]!;
      const s = part.series;
      if (cp.hasSeries !== (s !== undefined)) return false;
      if (s) {
        const useRuns = Boolean(s.runs && s.runs.length > 0);
        const sliceSource = useRuns ? s.runs!.source : s.buckets.source;
        if (
          cp.useRuns !== useRuns
          || cp.sliceSource !== sliceSource
          || cp.missingSource !== s.missing.source
          || cp.from !== s.from
          || cp.binHours !== s.binHours
        ) return false;
      }
      if (!sameWithinRanges(cp.within, part.within)) return false;
    }
  }
  return true;
}

function createStackCacheEntry(inputs: readonly BoundedStackBandInput[]): CachedBoundedStack {
  const bands: CachedStackBand[] = inputs.map(input => ({
    valueScale: input.valueScale ?? 1,
    parts: input.parts.map(part => {
      const s = part.series;
      const useRuns = Boolean(s?.runs && s.runs.length > 0);
      return {
        hasSeries: s !== undefined,
        useRuns,
        sliceSource: s ? (useRuns ? s.runs!.source : s.buckets.source) : null,
        missingSource: s ? s.missing.source : null,
        from: s ? s.from : 0,
        binHours: s ? s.binHours : 1,
        within: part.within !== undefined ? part.within.map(([a, b]) => [a, b] as const) : null,
        scannedClosed: 0,
        prevLast: -Infinity,
        nextMissingIdx: 0,
        straddlingBucketIdx: -1,
        tailRunActive: false,
        tailRunIdx: -1,
        tailRunFirst: 0,
        tailRunMinLast: -Infinity,
        tailRunValue: 0,
        tailRunBreakBefore: false,
      };
    }),
  }));
  return {
    bands,
    hasClosedPrefix: false,
    closedThrough: -Infinity,
    hours: [],
    valuesByBand: inputs.map(() => []),
    outsideByBand: inputs.map(() => null),
  };
}

function resetStackCacheEntry(entry: CachedBoundedStack): void {
  entry.hasClosedPrefix = false;
  entry.closedThrough = -Infinity;
  entry.hours = [];
  for (let b = 0; b < entry.bands.length; b++) {
    entry.valuesByBand[b] = [];
    entry.outsideByBand[b] = null;
    for (const part of entry.bands[b]!.parts) {
      part.scannedClosed = 0;
      part.prevLast = -Infinity;
      part.nextMissingIdx = 0;
      part.straddlingBucketIdx = -1;
      part.tailRunActive = false;
      part.tailRunIdx = -1;
      part.tailRunMinLast = -Infinity;
    }
  }
}

function lowerBoundNumber(sorted: readonly number[], target: number): number {
  let lo = 0;
  let hi = sorted.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (sorted[mid]! < target) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

function hasHourSorted(sorted: readonly number[], target: number): boolean {
  const idx = lowerBoundNumber(sorted, target);
  return idx < sorted.length && sorted[idx] === target;
}

function computeStackClosedThrough(
  entry: CachedBoundedStack,
  inputs: readonly BoundedStackBandInput[],
  hour: number,
): number {
  const maxClosed = Math.floor(hour) - 1;
  let closedThrough = maxClosed;
  for (let b = 0; b < inputs.length; b++) {
    const input = inputs[b]!;
    const cb = entry.bands[b]!;
    for (let p = 0; p < input.parts.length; p++) {
      const s = input.parts[p]!.series;
      if (!s) continue;
      const cp = cb.parts[p]!;
      if (Number.isFinite(s.validInterval[1])) {
        const validHi = Math.floor(s.validInterval[1]);
        if (validHi < closedThrough) closedThrough = validHi;
      }
      const missingTail = s.missing.tail;
      if (missingTail && missingTail.firstHour <= maxClosed && missingTail.lastHour < closedThrough) {
        closedThrough = missingTail.lastHour;
      }
      if (cp.useRuns) {
        const tail = s.runs!.tail;
        if (tail && tail.firstHour <= maxClosed) {
          const tailLimit = tail.lastHour - 1;
          if (tailLimit < closedThrough) closedThrough = tailLimit;
        }
      } else if (s.straddling !== 'none' || s.buckets.tail !== undefined) {
        const binEnd = s.from + s.buckets.closedLength * s.binHours - 1;
        if (binEnd < closedThrough) closedThrough = binEnd;
      }
    }
  }
  return closedThrough;
}

function collectSeriesClosedHours(
  series: BoundedSeries,
  cp: CachedStackPart,
  hasAfter: boolean,
  afterHour: number,
  untilHour: number,
  earlyTarget: Set<number>,
  target: Set<number>,
  prefixHours: readonly number[],
): boolean {
  const add = (h: number) => {
    if ((!hasAfter || h > afterHour) && h <= untilHour) target.add(h);
  };
  if (cp.useRuns) {
    const runs = series.runs!;
    const closedLen = runs.closedLength;
    const limit = runs.length;
    let pLast = cp.prevLast;
    let nextScanned = cp.scannedClosed;
    for (let i = cp.scannedClosed; i < limit; i++) {
      const run = runs.at(i)!;
      if (run.firstHour > untilHour) break;
      if (pLast !== -Infinity && (run.breakBefore || run.firstHour > pLast + 1)) {
        const gapHour = pLast + 1;
        if (gapHour < run.firstHour) {
          if (hasAfter && gapHour <= afterHour) {
            if (!hasHourSorted(prefixHours, gapHour)) earlyTarget.add(gapHour);
          } else {
            add(gapHour);
          }
        }
      }
      if (hasAfter && run.firstHour <= afterHour && !(cp.tailRunActive && i === cp.tailRunIdx)) return true;
      add(run.firstHour);
      if (i < closedLen && run.lastHour <= untilHour) {
        if (run.lastHour > run.firstHour) add(run.lastHour);
        pLast = run.lastHour;
        nextScanned = i + 1;
        cp.prevLast = run.lastHour;
        if (cp.tailRunActive && i === cp.tailRunIdx) {
          cp.tailRunActive = false;
          cp.tailRunIdx = -1;
          cp.tailRunMinLast = -Infinity;
        }
      } else {
        cp.tailRunActive = true;
        cp.tailRunIdx = i;
        cp.tailRunFirst = run.firstHour;
        cp.tailRunMinLast = Math.min(untilHour, run.lastHour);
        cp.tailRunValue = run.value;
        cp.tailRunBreakBefore = run.breakBefore === true;
        break;
      }
    }
    cp.scannedClosed = nextScanned;
  } else {
    const buckets = series.buckets;
    const binCap = series.binHours > 0 && Number.isFinite(untilHour)
      ? Math.max(0, Math.floor((untilHour - series.from) / series.binHours) + 1)
      : buckets.closedLength;
    const limit = Math.min(buckets.closedLength, binCap);
    const minStep = Math.max(1, series.binHours);
    let pLast = cp.prevLast;
    let nextScanned = cp.scannedClosed;
    for (let i = cp.scannedClosed; i < limit; i++) {
      const b = buckets.at(i);
      if (!b) {
        if (series.from + (i + 1) * series.binHours - 1 <= untilHour) {
          nextScanned = i + 1;
        }
        continue;
      }
      if (b.firstHour > untilHour) break;
      if (pLast !== -Infinity && (b.firstBreak || b.firstHour > pLast + minStep)) {
        const gapHour = pLast + 1;
        if (gapHour < b.firstHour) {
          if (hasAfter && gapHour <= afterHour) {
            if (!hasHourSorted(prefixHours, gapHour)) earlyTarget.add(gapHour);
          } else {
            add(gapHour);
          }
        }
      }
      if (hasAfter && b.firstHour <= afterHour && i !== cp.straddlingBucketIdx) return true;
      add(b.firstHour);
      if (b.lastHour <= untilHour) {
        if (!b.hasBreak && b.lastHour > b.firstHour) add(b.lastHour);
        pLast = b.lastHour;
        nextScanned = i + 1;
        cp.prevLast = b.lastHour;
        if (cp.straddlingBucketIdx === i) cp.straddlingBucketIdx = -1;
      } else {
        cp.straddlingBucketIdx = i;
        break;
      }
    }
    cp.scannedClosed = nextScanned;
  }
  const missing = series.missing;
  const mLen = missing.length;
  let nextMissing = cp.nextMissingIdx;
  let advanceMissing = true;
  for (let i = cp.nextMissingIdx; i < mLen; i++) {
    const span = missing.at(i)!;
    if (span.firstHour > untilHour) {
      advanceMissing = false;
      continue;
    }
    if (hasAfter && span.firstHour <= afterHour) return true;
    add(span.firstHour);
    if (advanceMissing) nextMissing = i + 1;
  }
  cp.nextMissingIdx = nextMissing;
  return false;
}

function collectSeriesTailHours(
  series: BoundedSeries,
  cp: CachedStackPart,
  hasAfter: boolean,
  afterHour: number,
  hour: number,
  earlyTarget: Set<number>,
  tailTarget: Set<number>,
  prefixHours: readonly number[],
): void {
  const emit = (h: number) => {
    if (h > hour) return;
    if (!hasAfter || h > afterHour) {
      tailTarget.add(h);
    } else if (!hasHourSorted(prefixHours, h)) {
      earlyTarget.add(h);
    }
  };
  if (cp.useRuns) {
    const runs = series.runs!;
    const limit = runs.length;
    let pLast = cp.prevLast;
    for (let i = cp.scannedClosed; i < limit; i++) {
      const run = runs.at(i)!;
      if (run.firstHour > hour) break;
      if (pLast !== -Infinity && (run.breakBefore || run.firstHour > pLast + 1)) {
        const gapHour = Math.min(hour, pLast + 1);
        if (gapHour < run.firstHour) emit(gapHour);
      }
      emit(run.firstHour);
      const end = Math.min(hour, run.lastHour);
      if (end > run.firstHour) emit(end);
      pLast = end;
    }
  } else {
    const buckets = series.buckets;
    const binCap = series.binHours > 0 && Number.isFinite(hour)
      ? Math.max(0, Math.floor((hour - series.from) / series.binHours) + 1)
      : buckets.length;
    const limit = Math.min(buckets.length, binCap);
    const minStep = Math.max(1, series.binHours);
    let pLast = cp.prevLast;
    for (let i = cp.scannedClosed; i < limit; i++) {
      const b = buckets.at(i);
      if (!b) continue;
      if (b.firstHour > hour) break;
      if (pLast !== -Infinity && (b.firstBreak || b.firstHour > pLast + minStep)) {
        const gapHour = Math.min(hour, pLast + 1);
        if (gapHour < b.firstHour) emit(gapHour);
      }
      emit(b.firstHour);
      if (!b.hasBreak) {
        const end = Math.min(hour, b.lastHour);
        if (end > b.firstHour) emit(end);
      }
      pLast = Math.min(hour, b.lastHour);
    }
  }
  const missing = series.missing;
  const mLen = missing.length;
  for (let i = cp.nextMissingIdx; i < mLen; i++) {
    const span = missing.at(i)!;
    if (span.firstHour <= hour) emit(span.firstHour);
  }
}

function collectWithinHoursWindow(
  within: readonly (readonly [number, number])[] | null,
  hasAfter: boolean,
  afterHour: number,
  untilHour: number,
  hour: number,
  target: Set<number>,
): void {
  if (!within) return;
  for (let i = 0; i < within.length; i++) {
    const first = within[i]![0];
    const last = within[i]![1];
    if ((!hasAfter || first > afterHour) && first <= untilHour) target.add(first);
    if (last < hour) {
      if ((!hasAfter || last > afterHour) && last <= untilHour) target.add(last);
      if ((!hasAfter || last + 1 > afterHour) && last + 1 <= untilHour) target.add(last + 1);
    }
  }
}

function sampleStackRow(
  inputs: readonly BoundedStackBandInput[],
  h: number,
  hour: number,
  row: number[],
  outsideFlags: boolean[],
): boolean {
  outsideFlags.fill(false);
  let present = 0;
  for (let b = 0; b < inputs.length; b++) {
    const input = inputs[b]!;
    const parts = input.parts;
    let covered = false;
    let value: number | null = null;
    if (h === hour && input.head) {
      for (let p = 0; p < parts.length; p++) {
        if (covers(parts[p]!, h)) { covered = true; break; }
      }
      if (!covered) {
        row[b] = 0;
        outsideFlags[b] = true;
        continue;
      }
      present++;
      outsideFlags[b] = false;
      value = 'value' in input.head ? input.head.value : null;
    } else {
      const scale = input.valueScale ?? 1;
      for (let p = 0; p < parts.length; p++) {
        const part = parts[p]!;
        if (!covers(part, h)) continue;
        covered = true;
        if (value === null && part.series) value = sampleOneBounded(part.series, h, scale);
      }
      if (!covered) {
        row[b] = 0;
        outsideFlags[b] = true;
        continue;
      }
      present++;
      outsideFlags[b] = false;
    }
    if (value === null) return false;
    row[b] = value;
  }
  return present > 0;
}

export function buildBoundedStack(inputs: readonly BoundedStackBandInput[], hour: number): ChartStackModel {
  if (!timeChartCacheState.enabled || !Number.isFinite(hour) || inputs.length === 0) {
    return buildBoundedStackUncached(inputs, hour);
  }
  let entry: CachedBoundedStack | undefined;
  let hitIdx = -1;
  for (let i = stackPrefixCache.length - 1; i >= 0; i--) {
    if (matchesStackCache(stackPrefixCache[i]!, inputs)) {
      entry = stackPrefixCache[i]!;
      hitIdx = i;
      break;
    }
  }
  if (!entry) {
    entry = createStackCacheEntry(inputs);
    if (stackPrefixCache.length >= STACK_CACHE_MAX) stackPrefixCache.shift();
    stackPrefixCache.push(entry);
  } else if (hitIdx !== stackPrefixCache.length - 1) {
    stackPrefixCache.splice(hitIdx, 1);
    stackPrefixCache.push(entry);
  }

  let shrunk = false;
  for (let b = 0; b < inputs.length && !shrunk; b++) {
    const input = inputs[b]!;
    const cb = entry.bands[b]!;
    for (let p = 0; p < input.parts.length; p++) {
      const s = input.parts[p]!.series;
      if (!s) continue;
      const cp = cb.parts[p]!;
      if (cp.useRuns) {
        const runs = s.runs!;
        if (runs.closedLength < cp.scannedClosed) { shrunk = true; break; }
        if (cp.tailRunActive) {
          const checkRun = cp.tailRunIdx < runs.length ? runs.at(cp.tailRunIdx) : undefined;
          if (
            !checkRun
            || checkRun.firstHour !== cp.tailRunFirst
            || checkRun.lastHour < cp.tailRunMinLast
            || !Object.is(checkRun.value, cp.tailRunValue)
            || (checkRun.breakBefore === true) !== cp.tailRunBreakBefore
          ) {
            shrunk = true;
            break;
          }
        }
      } else if (s.buckets.closedLength < cp.scannedClosed) {
        shrunk = true;
        break;
      }
      if (s.missing.length < cp.nextMissingIdx) { shrunk = true; break; }
      if (entry.hasClosedPrefix) {
        for (let mi = cp.nextMissingIdx; mi < s.missing.length; mi++) {
          if (s.missing.at(mi)!.firstHour <= entry.closedThrough) { shrunk = true; break; }
        }
        if (shrunk) break;
      }
    }
  }

  const closedThrough = computeStackClosedThrough(entry, inputs, hour);
  if (shrunk || (entry.hasClosedPrefix && closedThrough < entry.closedThrough)) {
    resetStackCacheEntry(entry);
  }

  const bandCount = inputs.length;
  const row = new Array<number>(bandCount);
  const outsideFlags = new Array<boolean>(bandCount);

  const commitPrefix = () => {
    if (!Number.isFinite(closedThrough) || (entry.hasClosedPrefix && closedThrough <= entry.closedThrough)) return false;
    const earlyClosedSet = new Set<number>();
    const newSet = new Set<number>();
    const prevHasClosed = entry.hasClosedPrefix;
    const prevClosed = entry.closedThrough;
    for (let b = 0; b < bandCount; b++) {
      const input = inputs[b]!;
      const cb = entry.bands[b]!;
      for (let p = 0; p < input.parts.length; p++) {
        const part = input.parts[p]!;
        const cp = cb.parts[p]!;
        if (part.series && collectSeriesClosedHours(part.series, cp, prevHasClosed, prevClosed, closedThrough, earlyClosedSet, newSet, entry.hours)) {
          return true;
        }
        collectWithinHoursWindow(cp.within, prevHasClosed, prevClosed, closedThrough, hour, newSet);
      }
    }
    if (earlyClosedSet.size > 0) {
      const earlyAdded = earlyClosedSet.size > 1 ? [...earlyClosedSet].sort((a, b) => a - b) : [...earlyClosedSet];
      for (let i = 0; i < earlyAdded.length; i++) {
        const h = earlyAdded[i]!;
        const ins = lowerBoundNumber(entry.hours, h);
        const valid = sampleStackRow(inputs, h, hour, row, outsideFlags);
        if (ins === entry.hours.length) {
          entry.hours.push(h);
          for (let b = 0; b < bandCount; b++) {
            entry.valuesByBand[b]!.push(valid ? row[b]! : null);
            if (outsideFlags[b]) {
              let out = entry.outsideByBand[b];
              if (!out) {
                out = new Array<boolean>(ins + 1).fill(false);
                entry.outsideByBand[b] = out;
              } else {
                out.push(true);
              }
              out[ins] = true;
            } else if (entry.outsideByBand[b]) {
              entry.outsideByBand[b]!.push(false);
            }
          }
        } else {
          entry.hours.splice(ins, 0, h);
          for (let b = 0; b < bandCount; b++) {
            entry.valuesByBand[b]!.splice(ins, 0, valid ? row[b]! : null);
            if (outsideFlags[b]) {
              let out = entry.outsideByBand[b];
              if (!out) {
                out = new Array<boolean>(entry.hours.length).fill(false);
                entry.outsideByBand[b] = out;
                out[ins] = true;
              } else {
                out.splice(ins, 0, true);
              }
            } else if (entry.outsideByBand[b]) {
              entry.outsideByBand[b]!.splice(ins, 0, false);
            }
          }
        }
      }
    }
    if (newSet.size > 0) {
      const added = [...newSet].sort((a, b) => a - b);
      for (let i = 0; i < added.length; i++) {
        const h = added[i]!;
        const idx = entry.hours.length;
        entry.hours.push(h);
        const valid = sampleStackRow(inputs, h, hour, row, outsideFlags);
        for (let b = 0; b < bandCount; b++) {
          entry.valuesByBand[b]!.push(valid ? row[b]! : null);
          if (outsideFlags[b]) {
            let out = entry.outsideByBand[b];
            if (!out) {
              out = new Array<boolean>(idx + 1).fill(false);
              entry.outsideByBand[b] = out;
            } else {
              out.push(true);
            }
            out[idx] = true;
          } else if (entry.outsideByBand[b]) {
            entry.outsideByBand[b]!.push(false);
          }
        }
      }
    }
    entry.hasClosedPrefix = true;
    entry.closedThrough = closedThrough;
    return false;
  };

  if (commitPrefix()) {
    resetStackCacheEntry(entry);
    commitPrefix();
  }

  const earlySet = new Set<number>();
  const tailSet = new Set<number>();
  const hasClosed = entry.hasClosedPrefix;
  const afterClosed = entry.closedThrough;
  for (let b = 0; b < bandCount; b++) {
    const input = inputs[b]!;
    const cb = entry.bands[b]!;
    for (let p = 0; p < input.parts.length; p++) {
      const part = input.parts[p]!;
      const cp = cb.parts[p]!;
      if (part.series) {
        collectSeriesTailHours(part.series, cp, hasClosed, afterClosed, hour, earlySet, tailSet, entry.hours);
      }
      collectWithinHoursWindow(cp.within, hasClosed, afterClosed, hour, hour, tailSet);
    }
  }
  if ((entry.hours.length > 0 || earlySet.size > 0 || tailSet.size > 0 || inputs.some(input => input.head)) && (!hasClosed || hour > afterClosed)) {
    tailSet.add(hour);
  }
  const earlyHours = earlySet.size > 1 ? [...earlySet].sort((a, b) => a - b) : [...earlySet];
  const tailHours = tailSet.size > 1 ? [...tailSet].sort((a, b) => a - b) : [...tailSet];
  const prefixLen = entry.hours.length;
  const earlyLen = earlyHours.length;
  const tailLen = tailHours.length;
  const totalLen = prefixLen + earlyLen + tailLen;

  let hours: number[];
  const valuesByBand: (number | null)[][] = new Array(bandCount);
  const outsideByBand: (boolean[] | null)[] = new Array(bandCount);

  if (earlyLen === 0) {
    hours = entry.hours.slice();
    for (let i = 0; i < tailLen; i++) hours.push(tailHours[i]!);
    for (let b = 0; b < bandCount; b++) {
      const dstVals = entry.valuesByBand[b]!.slice();
      for (let i = 0; i < tailLen; i++) dstVals.push(null);
      valuesByBand[b] = dstVals;
      const srcOut = entry.outsideByBand[b];
      if (srcOut) {
        const dstOut = srcOut.slice();
        for (let i = 0; i < tailLen; i++) dstOut.push(false);
        outsideByBand[b] = dstOut;
      } else {
        outsideByBand[b] = null;
      }
    }
    for (let i = 0; i < tailLen; i++) {
      const h = tailHours[i]!;
      const idx = prefixLen + i;
      const valid = sampleStackRow(inputs, h, hour, row, outsideFlags);
      for (let b = 0; b < bandCount; b++) {
        if (outsideFlags[b]) {
          (outsideByBand[b] ??= new Array<boolean>(totalLen).fill(false))[idx] = true;
        }
        if (valid) valuesByBand[b]![idx] = row[b]!;
      }
    }
  } else {
    hours = new Array<number>(totalLen);
    for (let b = 0; b < bandCount; b++) {
      valuesByBand[b] = new Array<number | null>(totalLen).fill(null);
      outsideByBand[b] = entry.outsideByBand[b] ? new Array<boolean>(totalLen).fill(false) : null;
    }
    let pi = 0;
    let ei = 0;
    let outIdx = 0;
    while (pi < prefixLen || ei < earlyLen) {
      if (ei >= earlyLen || (pi < prefixLen && entry.hours[pi]! < earlyHours[ei]!)) {
        hours[outIdx] = entry.hours[pi]!;
        for (let b = 0; b < bandCount; b++) {
          valuesByBand[b]![outIdx] = entry.valuesByBand[b]![pi]!;
          const srcOut = entry.outsideByBand[b];
          if (srcOut) outsideByBand[b]![outIdx] = srcOut[pi]!;
        }
        pi++;
      } else {
        const h = earlyHours[ei]!;
        hours[outIdx] = h;
        const valid = sampleStackRow(inputs, h, hour, row, outsideFlags);
        for (let b = 0; b < bandCount; b++) {
          if (outsideFlags[b]) {
            (outsideByBand[b] ??= new Array<boolean>(totalLen).fill(false))[outIdx] = true;
          }
          if (valid) valuesByBand[b]![outIdx] = row[b]!;
        }
        ei++;
      }
      outIdx++;
    }
    for (let i = 0; i < tailLen; i++) {
      const h = tailHours[i]!;
      const idx = outIdx + i;
      hours[idx] = h;
      const valid = sampleStackRow(inputs, h, hour, row, outsideFlags);
      for (let b = 0; b < bandCount; b++) {
        if (outsideFlags[b]) {
          (outsideByBand[b] ??= new Array<boolean>(totalLen).fill(false))[idx] = true;
        }
        if (valid) valuesByBand[b]![idx] = row[b]!;
      }
    }
  }

  const bands: ChartStackBand[] = inputs.map((input, idx) => ({
    id: input.id,
    key: input.key,
    name: input.name,
    color: input.color,
    ...(input.opacity !== undefined ? { opacity: input.opacity } : {}),
    values: valuesByBand[idx]!,
    ...(outsideByBand[idx] ? { outside: outsideByBand[idx]! } : {}),
    ...(input.endLabel !== undefined ? { endLabel: input.endLabel } : {}),
    ...(input.flag ? { flag: input.flag } : {}),
  }));
  return { hours, bands };
}
