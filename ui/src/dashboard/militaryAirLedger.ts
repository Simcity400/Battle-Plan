import type { MilitaryAirCountry, MilitaryAirLeaf, MilitaryAirPoint, MilitaryAirReply } from '../generated/dashboard';
import { clipToSpans, isOrderedSpans, runAt } from '../charts/series.ts';
import type { Run, Span } from '../charts/series';
import { formatFixed } from '../charts/valueAxis.ts';
import { object, opaqueTag, signed32, unsigned32 } from './validation.ts';
import { compareTags } from './tagOrder.ts';
import { worldFilter } from './groups.ts';
import type { CountryGroupReply } from '../generated/dashboard';
import type { CountrySelection } from './model';
import { formatDateTime } from './calendar.ts';
import { formatRatio } from '../charts/butterfly/layout.ts';
import type { LabelAt } from '../charts/race/lines';
import type { TooltipContent } from '../charts/tooltip';
import { pendingReplyAt, preparedPending } from './preparedPrefix.ts';

const signed128 = (value: bigint) => value >= -(1n << 127n) && value < (1n << 127n);
// Below 39 characters a decimal is under 10^38 < 2^127, so only longer text is parsed.
const exact128 = (value: unknown): value is string => typeof value === 'string' && /^(?:0|-?[1-9]\d*)$/.test(value)
  && (value.length < 39 || signed128(BigInt(value)));
const optional = <T>(value: unknown, valid: (value: unknown) => value is T): value is T | null => value === null || valid(value);
const text = (value: unknown): value is string => typeof value === 'string';
/** Whether ordered, disjoint `spans` hold all of `span` in one of them. */
const within = (span: Span, spans: readonly Span[]) => (runAt(spans, span.firstHour)?.lastHour ?? -Infinity) >= span.lastHour;

/** Both products preserve the same exact prefix, date, endpoint and coverage rules. */
function isMilitaryAirReply(value: unknown): value is MilitaryAirReply {
  if (!object(value) || !['airLosses', 'bombing'].includes(String(value.family)) || !signed32(value.firstHour) || !signed32(value.lastHour)
    || value.firstHour > value.lastHour || typeof value.available !== 'boolean' || !isOrderedSpans(value.observed) || !isOrderedSpans(value.incomplete)
    || !Array.isArray(value.countries) || !Array.isArray(value.leaves) || !Array.isArray(value.regions) || !Array.isArray(value.buildings)
    || !Array.isArray(value.warnings) || !value.warnings.every(text)) return false;
  const observed = value.observed, first = value.firstHour, lastHour = value.lastHour;
  if (observed.some(span => span.firstHour < first || span.lastHour > lastHour) || value.available !== (observed.length > 0)
    || !value.incomplete.every(span => within(span, observed))) return false;
  // Present spans by tag, then native identity.
  const countries = new Map<string, Map<string, Span[]>>();
  for (const country of value.countries) {
    if (!object(country) || !opaqueTag(country.tag) || !text(country.identity) || !country.identity || !isOrderedSpans(country.present)
      || !optional(country.membership, isOrderedSpans)) return false;
    const identities = countries.get(country.tag) ?? new Map<string, Span[]>();
    const present = country.present;
    if (identities.has(country.identity) || !present.every(span => within(span, observed))
      || country.membership?.some(span => !within(span, present))) return false;
    identities.set(country.identity, present);
    countries.set(country.tag, identities);
  }
  const keys = new Set<string>();
  for (const leaf of value.leaves) {
    if (!object(leaf) || !optional(leaf.tag, opaqueTag) || !optional(leaf.identity, text) || (leaf.tag === null) !== (leaf.identity === null)
      || typeof leaf.dealt !== 'boolean' || !optional(leaf.opponent, opaqueTag) || !optional(leaf.region, unsigned32) || !optional(leaf.buildingId, unsigned32)
      || !optional(leaf.buildingToken, text) || !optional(leaf.state, unsigned32) || !unsigned32(leaf.cause) || !Array.isArray(leaf.points)) return false;
    if (value.family === 'airLosses' ? leaf.region === null || leaf.buildingId !== null || leaf.buildingToken !== null || leaf.state !== null
      : leaf.region !== null || leaf.buildingId === null || leaf.state === null || leaf.opponent !== null) return false;
    const key = JSON.stringify([leaf.tag, leaf.identity, leaf.dealt, leaf.opponent, leaf.region, leaf.buildingId, leaf.buildingToken, leaf.state, leaf.cause]);
    if (keys.has(key)) return false;
    keys.add(key);
    const present = leaf.tag === null ? observed : countries.get(leaf.tag)?.get(leaf.identity as string);
    if (!present) return false;
    let last: number | null = null, read: number | null = null;
    for (const point of leaf.points) {
      if (!object(point) || !signed32(point.hour) || !signed32(point.readHour) || point.hour > point.readHour || !exact128(point.totalRaw)
        || read !== null && (point.readHour < read || point.readHour === read && last !== null && point.hour <= last)
        || !runAt(present, point.readHour)) return false;
      last = point.hour; read = point.readHour;
    }
  }
  for (const names of [value.regions, value.buildings]) {
    const ids = new Set<string>();
    for (const name of names) {
      if (!object(name) || !text(name.id) || !name.id || !optional(name.name, text) || !optional(name.missing, text)
        || (name.name === null) !== (name.missing !== null) || ids.has(name.id)) return false;
      ids.add(name.id);
    }
  }
  return true;
}

export function validateMilitaryAirReply(value: unknown): MilitaryAirReply {
  if (!isMilitaryAirReply(value)) throw new Error('Invalid military air ledger reply');
  preparedPending(value);
  return value;
}

export const militaryAirPreparing = pendingReplyAt;

export interface LedgerFilters { regions: ReadonlySet<number>; causes: ReadonlySet<number> }
export interface LedgerTotal { readHour: number; nativeHour: number | null; dealt: bigint; taken: bigint }
export interface LedgerSeries { country: MilitaryAirCountry; points: LedgerTotal[]; leaves: MilitaryAirLeaf[] }
export interface LedgerIndex { reply: MilitaryAirReply; series: LedgerSeries[]; byTag: ReadonlyMap<string, LedgerSeries[]>; unknown: LedgerTotal[] }
const sharedIndices = new WeakMap<MilitaryAirReply, LedgerIndex>();

/** Accumulate exact per-read deltas once; playback then uses a binary search. */
export function ledgerTotals(leaves: readonly MilitaryAirLeaf[]): LedgerTotal[] {
  const deltas = new Map<number, { dealt: bigint; taken: bigint; nativeHour: number }>();
  for (const leaf of leaves) {
    let before = 0n;
    for (const point of leaf.points) {
      const current = BigInt(point.totalRaw), delta = current - before;
      before = current;
      const row = deltas.get(point.readHour) ?? { dealt: 0n, taken: 0n, nativeHour: point.hour };
      row[leaf.dealt ? 'dealt' : 'taken'] += delta;
      row.nativeHour = Math.max(row.nativeHour, point.hour);
      deltas.set(point.readHour, row);
    }
  }
  let dealt = 0n, taken = 0n;
  return [...deltas].sort(([a], [b]) => a - b).map(([readHour, delta]) => {
    dealt += delta.dealt; taken += delta.taken;
    return { readHour, nativeHour: delta.nativeHour, dealt, taken };
  });
}

export function militaryAirIndex(reply: MilitaryAirReply, filters: LedgerFilters): LedgerIndex {
  const unfiltered = !filters.regions.size && !filters.causes.size;
  const shared = unfiltered ? sharedIndices.get(reply) : undefined;
  if (shared) return shared;
  const byCountry = new Map<string, MilitaryAirLeaf[]>(), unknown: MilitaryAirLeaf[] = [];
  for (const leaf of reply.leaves) {
    if (filters.regions.size && (leaf.region === null || !filters.regions.has(leaf.region)) || filters.causes.size && !filters.causes.has(leaf.cause)) continue;
    if (leaf.tag === null) { unknown.push(leaf); continue; }
    const key = leaf.identity!, leaves = byCountry.get(key) ?? [];
    leaves.push(leaf); byCountry.set(key, leaves);
  }
  const byHolder = new Map<string, LedgerTotal[]>(), byTag = new Map<string, LedgerSeries[]>();
  const series = reply.countries.map(country => {
    // All aliases of the same native holder share totals, while their dated
    // presence and labels remain separate. A reused tag has a different holder.
    const leaves = byCountry.get(country.identity) ?? [];
    let points = byHolder.get(country.identity);
    if (!points) { points = ledgerTotals(leaves); byHolder.set(country.identity, points); }
    const row = { country, leaves, points };
    const aliases = byTag.get(country.tag) ?? [];
    aliases.push(row); byTag.set(country.tag, aliases);
    return row;
  });
  const index = { reply, series, byTag, unknown: ledgerTotals(unknown) };
  if (unfiltered) sharedIndices.set(reply, index);
  return index;
}

function at<T>(points: readonly T[], hour: number, date: (point: T) => number): T | null {
  let low = 0, high = points.length;
  while (low < high) { const mid = Math.floor((low + high) / 2); if (date(points[mid]) <= hour) low = mid + 1; else high = mid; }
  return low ? points[low - 1] : null;
}
const empty = (hour: number): LedgerTotal => ({ readHour: hour, nativeHour: null, dealt: 0n, taken: 0n });
export const ledgerComplete = (spans: readonly Span[], from: number, hour: number, incomplete: readonly Span[] = []) => {
  const span = runAt(spans, hour);
  return from <= hour && span !== null && span.firstHour <= from && !incomplete.some(read => read.firstHour <= hour && read.lastHour >= from);
};
export const ledgerDifference = (points: readonly LedgerTotal[], from: number, hour: number): LedgerTotal => {
  const last = at(points, hour, point => point.readHour) ?? empty(hour);
  const before = at(points, from - 1, point => point.readHour) ?? empty(from);
  return { ...last, readHour: hour, dealt: last.dealt - before.dealt, taken: last.taken - before.taken };
};

export interface LedgerRow extends LedgerTotal { tag: string; identity: string; segment: number; complete: boolean; series: LedgerSeries }
export function militaryAirAt(index: LedgerIndex, from: number, hour: number): LedgerRow[] {
  if (!runAt(index.reply.observed, hour)) return [];
  return index.series.flatMap(series => {
    const span = runAt(series.country.present, hour);
    if (!span) return [];
    return [{ ...ledgerDifference(series.points, from, hour), tag: series.country.tag, identity: series.country.identity,
      segment: span.firstHour, complete: ledgerComplete(index.reply.observed, from, hour, index.reply.incomplete), series }];
  });
}

/** A chosen tag follows its recorded holder through later changes of tag. */
export function ledgerCountryAt(index: LedgerIndex, tag: string, from: number, hour: number, identity?: string): LedgerRow | null {
  const chosen = identity ?? (index.byTag.get(tag) ?? []).reduce<MilitaryAirCountry | null>((first, series) => {
    const country = series.country;
    return !first || (country.present[0]?.firstHour ?? Infinity) < (first.present[0]?.firstHour ?? Infinity) ? country : first;
  }, null)?.identity;
  const rows = militaryAirAt(index, from, hour).filter(row => row.identity === chosen);
  return rows.find(row => row.tag === tag) ?? rows[0] ?? null;
}

/** A pinned native holder belongs to this campaign, branch and selected tag. */
export function ledgerPinnedIdentity(value: unknown, campaign: string, branch: string, tag: string): string | undefined {
  if (typeof value !== 'string') return undefined;
  try {
    const saved: unknown = JSON.parse(value);
    return Array.isArray(saved) && saved.length === 4 && saved[0] === campaign && saved[1] === branch && saved[2] === tag
      && typeof saved[3] === 'string' && saved[3] ? saved[3] : undefined;
  } catch { return undefined; }
}

export function ledgerCountries(rows: readonly LedgerRow[], selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, lastHour: number, metric: 'dealt' | 'taken' | 'total' = 'total', countries: readonly MilitaryAirCountry[] = []): { rows: LedgerRow[]; reason: string | null } {
  const ranked = [...rows].sort((a, b) => {
    const av = metric === 'total' ? a.dealt + a.taken : a[metric], bv = metric === 'total' ? b.dealt + b.taken : b[metric];
    return av === bv ? compareTags(a.tag, b.tag) : av > bv ? -1 : 1;
  });
  if (selection.kind === 'group' && selection.group.type === 'top') return { rows: ranked.filter(row => row.series.country.membership === null || runAt(row.series.country.membership, hour)).slice(0, selection.group.count), reason: null };
  const chosen = selection.kind === 'tags' ? selection.tags : selection.kind === 'selected' ? fallback : null;
  if (chosen) {
    if (!chosen.length) return { rows: [], reason: 'Choose a country in the widget settings.' };
    const roots = new Set(chosen);
    const identities = new Set(countries.filter(country => roots.has(country.tag)).map(country => country.identity));
    return { rows: ranked.filter(row => identities.has(row.identity)),
      reason: identities.size ? null : 'Selected countries are unavailable in this campaign.' };
  }
  const filter = worldFilter(selection, fallback, groups, hour, lastHour);
  return { rows: ranked.filter(row => filter.tags === null || filter.tags.has(row.tag)), reason: filter.reason };
}

export const ledgerShare = (raw: bigint, total: bigint) => total === 0n ? '—' : `${formatFixed(raw * 1000n / total, 1, 1)}%`;
export const ledgerScale = (family: MilitaryAirReply['family']) => family === 'airLosses' ? 100_000 : 10_000_000;
export const ledgerFormat = (raw: bigint | null, family: MilitaryAirReply['family']) => raw === null ? 'Not recorded' : formatFixed(raw, family === 'airLosses' ? 5 : 7, family === 'airLosses' ? 5 : 7);
export const AIR_LEDGER_NOTE = 'Loss totals include corrections.';
export const LEDGER_GAP_NOTE = 'Some losses may be missing.';
export const BOMBING_GAP_NOTE = 'Some damage may be missing.';
export const airCauseName = (cause: number) => ({ 1: 'Aircraft combat', 2: 'Anti-air / raid effect', 3: 'Air accidents', 4: 'Airbase bombing', 5: 'Train anti-air', 6: 'Kamikaze anti-air' } as Record<number, string>)[cause] ?? 'Other';

/** Draw only complete, present intervals; prefix totals never create a bridge. */
export function ledgerRuns(series: LedgerSeries, from: number, to: number, dealt: boolean, scale: number, membership: readonly Span[] | null = null): Run[] {
  const runs: Run[] = [];
  for (const span of series.country.present) {
    const first = Math.max(from, span.firstHour), last = Math.min(to, span.lastHour);
    if (first > last) continue;
    const changes = [first, ...series.points.filter(point => point.readHour > first && point.readHour <= last).map(point => point.readHour)];
    for (let i = 0; i < changes.length; i++) {
      const value = ledgerDifference(series.points, from, changes[i]);
      runs.push({ firstHour: changes[i], lastHour: i + 1 < changes.length ? changes[i + 1] - 1 : last,
        value: Number(dealt ? value.dealt : value.taken) / scale, breakBefore: i === 0 });
    }
  }
  return membership ? clipToSpans(runs, membership) : runs;
}

/** Country-look requests need only that tag's holders, rather than a world scan. */
export const ledgerSegmentAt = (index: LedgerIndex) => (tag: string, hour: number): number | null => {
  for (const series of index.byTag.get(tag) ?? []) {
    const span = runAt(series.country.present, hour);
    if (span) return span.firstHour;
  }
  return null;
};

export function airLossParts(row: LedgerRow, from: number, hour: number) {
  const causes = new Map<number, { dealt: bigint; taken: bigint }>();
  let operational = 0n, unknownOpponent = 0n;
  for (const leaf of row.series.leaves) {
    const amount = leafAmount(leaf, from, hour), part = causes.get(leaf.cause) ?? { dealt: 0n, taken: 0n };
    part[leaf.dealt ? 'dealt' : 'taken'] += amount; causes.set(leaf.cause, part);
    if (!leaf.dealt) {
      if (leaf.cause === 3) operational += amount;
      else if (leaf.opponent === null) unknownOpponent += amount;
    }
  }
  return { causes, operational, unknownOpponent };
}

/** Exact text is shared by race, butterfly and line hovers; plotting numbers never supply it. */
export function militaryAirTooltip(row: LedgerRow, from: number, family: MilitaryAirReply['family'], name: string): TooltipContent {
  const air = family === 'airLosses', parts = air ? airLossParts(row, from, row.readHour) : null;
  return { title: name,
    rows: [{ label: air ? 'Own losses' : 'Damage suffered', value: ledgerFormat(row.taken, family) },
      { label: air ? 'Enemy losses' : 'Damage inflicted', value: ledgerFormat(row.dealt, family) },
      { label: 'Ratio', value: !row.complete ? 'Not available' : row.taken === 0n ? '— (zero losses)' : formatRatio(row.dealt, row.taken) },
      ...(parts ? [{ label: 'Operational losses (accidents)', value: ledgerFormat(parts.operational, family) },
        { label: 'Other losses with unknown opponent', value: ledgerFormat(parts.unknownOpponent, family) },
        ...[...parts.causes].sort(([a], [b]) => a - b).map(([cause, value]) => ({ label: airCauseName(cause),
          value: ledgerFormat(value.taken, family) + ' lost · ' + ledgerFormat(value.dealt, family) + ' credited' }))] : []),
    ],
    note: [!row.complete ? air ? LEDGER_GAP_NOTE : BOMBING_GAP_NOTE : '', air ? AIR_LEDGER_NOTE : 'Building levels.'].filter(Boolean).join(' ') };
}

export function militaryAirLineTooltip(index: LedgerIndex, from: number, hour: number, tags: readonly string[], labelAt: LabelAt,
  colour: (tag: string) => string): TooltipContent {
  const chosen = new Set(tags), label = labelAt(hour);
  const rows = militaryAirAt(index, from, hour).filter(row => chosen.has(row.tag));
  return { title: formatDateTime(hour), subtitle: 'Aircraft losses', rows: rows.length ? rows.flatMap((row, i) => {
    const named = label(row.tag), detail = militaryAirTooltip(row, from, index.reply.family, named.name);
    return [{ label: named.name, icon: named.flag, countryFlag: named, swatch: colour(row.tag), strong: true, divider: i > 0 },
      ...detail.rows.map(part => ({ ...part, indent: true }))];
  }) : [{ label: 'Not recorded', tone: 'muted' }],
    note: [AIR_LEDGER_NOTE, rows.some(row => !row.complete) ? LEDGER_GAP_NOTE : ''].filter(Boolean).join(' ') };
}

/** Range replies already select the native lower bound; prefixes gate supplying reads. */
export function leafAmount(leaf: MilitaryAirLeaf, from: number, hour: number): bigint {
  const total = (point: MilitaryAirPoint | null) => point ? BigInt(point.totalRaw) : 0n;
  return total(at(leaf.points, hour, point => point.readHour)) - total(at(leaf.points, from - 1, point => point.readHour));
}
