import { dayLabel } from '../../charts/draw.ts';
import type { CasualtyParams, CasualtyReply, CasualtySeries, CasualtyValues, CountryGroupReply, HourSpan } from '../../generated/dashboard';
import { runAt } from '../../charts/series.ts';
import { IntervalIndex } from '../../charts/intervalIndex.ts';
import { formatFixed } from '../../charts/valueAxis.ts';
import { formatRatio } from '../../charts/butterfly/layout.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { groupMembers, groupRuns, groupUnobserved, resolveCountries } from '../../dashboard/groups.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { ResolvedCountries } from '../../dashboard/groups';
import type { CountryRaceEntry } from '../../charts/race/countryRace';
const RAW = /^-?\d{1,39}$/;
const hour = (value: unknown): value is number => Number.isInteger(value) && (value as number) >= -2147483648 && (value as number) <= 2147483647;
const spans = (value: unknown): value is HourSpan[] => Array.isArray(value) && value.every((span, i) => span != null
  && hour(span.firstHour) && hour(span.lastHour) && span.firstHour <= span.lastHour
  && (i === 0 || value[i - 1].lastHour < span.firstHour));
const raw = (value: unknown): value is string => typeof value === 'string' && RAW.test(value);
/** Index of the last start at or before an hour. */
function preceding<T>(values: readonly T[], at: number, start: (value: T) => number): number {
  let lo = 0, hi = values.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (start(values[mid]) <= at) lo = mid + 1; else hi = mid; }
  return lo - 1;
}
function values(column: CasualtyValues, count: number): bigint[] {
  if (!Array.isArray(column) || column.length !== 2 || !raw(column[0]) || !Array.isArray(column[1])) throw new Error('The casualty values are malformed.');
  let value = BigInt(column[0]); const result = count ? [value] : [];
  for (const entry of column[1]) {
    const repeated = Array.isArray(entry), delta = repeated ? entry[1] : entry, repeats = repeated ? entry[0] : 1;
    if (repeated && (entry.length !== 2 || !raw(delta)) || !Number.isSafeInteger(repeats) || repeats <= 0 || repeats > count - result.length
      || !(typeof delta === 'string' ? raw(delta) : Number.isSafeInteger(delta))) throw new Error('The casualty values are malformed.');
    const step = BigInt(delta);
    for (let i = 0; i < repeats; i++) { value += step; result.push(value); }
  }
  if (result.length !== count) throw new Error('The casualty values are malformed.');
  return result;
}
interface Point { firstHour: number; lastHour: number; endRead: number | null; dealt: bigint; taken: bigint; unknown: bigint | null; id: string; segment: string }
function points(series: CasualtySeries, observed: HourSpan[]): Point[] {
  if (!Array.isArray(series.reads) || !series.reads.every((read, i) => i === 0 ? hour(read) : Number.isSafeInteger(read) && read > 0 && read <= 4294967295)
    || !Array.isArray(series.ends) || !series.ends.every((end, i) => Array.isArray(end) && end.length === 3 && Number.isInteger(end[0]) && end[0] >= 0 && end[0] < series.reads.length && hour(end[1]) && (end[2] === null || hour(end[2])) && (i === 0 || series.ends[i - 1][0] < end[0]))
    || series.atLeastFrom !== null && !hour(series.atLeastFrom) || series.unknownUntil !== null && !hour(series.unknownUntil)) throw new Error('The casualty samples are malformed.');
  const count = series.reads.length, reads: number[] = []; let read = 0;
  for (const [i, delta] of series.reads.entries()) { read = i ? read + delta : delta; if (!hour(read)) throw new Error('The casualty dates are malformed.'); reads.push(read); }
  const dealt = values(series.dealt, count), taken = values(series.taken, count);
  const unknownCount = series.unknownUntil === null ? count : reads.filter(read => read < series.unknownUntil!).length;
  const unknown = series.unknown === null ? [] : values(series.unknown, unknownCount);
  const ends = new Map(series.ends.map(([index, last, confirmed]) => [index, { last, confirmed }])), result: Point[] = [];
  const spans = series.membership ?? observed;
  let cursor = 0, lastSegment = '', lastId = '', lastEnd = -Infinity;
  for (let i = 0; i < count; i++) {
    const endpoint = ends.get(i);
    const endRead = endpoint?.confirmed ?? null;
    const end = endpoint?.last ?? reads[i + 1] - 1;
    if (!hour(end) || reads[i] > end || i + 1 < count && end >= reads[i + 1]) throw new Error('The casualty endpoints are malformed.');
    if (endRead !== null && (endRead < reads[i] || endRead > end)) throw new Error('The casualty confirming reads are malformed.');
    const readSpan = runAt(observed, reads[i]);
    if (!readSpan || readSpan.lastHour < end) throw new Error('The casualty samples extend beyond recorded hours.');
    const identity = preceding(series.identities, reads[i], entry => entry.firstHour);
    if (identity < 0 || series.identities[identity + 1]?.firstHour <= end) throw new Error('The casualty identities are malformed.');
    const id = series.identities[identity].id;
    while (spans[cursor]?.lastHour < reads[i]) cursor++;
    for (let at = cursor; spans[at] && spans[at].firstHour <= end; at++) {
      const firstHour = Math.max(reads[i], spans[at].firstHour), lastHour = Math.min(end, spans[at].lastHour);
      const observedSpan = runAt(observed, firstHour);
      if (!observedSpan || observedSpan.lastHour < lastHour) throw new Error('The casualty spans are malformed.');
      if (id !== lastId || firstHour !== lastEnd + 1) lastSegment = JSON.stringify([id, firstHour]);
      result.push({ firstHour, lastHour, endRead, dealt: dealt[i], taken: taken[i], unknown: unknown[i] ?? null, id, segment: lastSegment });
      lastId = id; lastEnd = lastHour;
    }
  }
  return result;
}
const decoded = new WeakMap<CasualtyReply, Point[][]>();
/** Validate and accumulate exact columnar values once at the product boundary. */
export function validateCasualtyReply(value: unknown): CasualtyReply {
  const reply = value as CasualtyReply;
  if (reply == null || typeof reply !== 'object' || typeof reply.branchId !== 'string' || !spans(reply.observed)
    || typeof reply.hasGap !== 'boolean' || typeof reply.unknownLabel !== 'string' || !reply.unknownLabel
    || !Array.isArray(reply.warnings) || !reply.warnings.every(w => typeof w === 'string') || !Array.isArray(reply.series)) throw new Error('The casualty reply is malformed.');
  const tags = new Set<string>();
  for (const series of reply.series) {
    if (series == null || typeof series.tag !== 'string' || !series.tag || series.tag.includes(String.fromCharCode(0))
      || series.membership !== null && !spans(series.membership) || !Array.isArray(series.labels) || !series.labels.every(label => typeof label === 'string')
      || !Array.isArray(series.identities) || !series.identities.every((identity, i) => identity != null && hour(identity.firstHour) && typeof identity.id === 'string' && identity.id.length > 0
        && (i === 0 || series.identities[i - 1].firstHour < identity.firstHour))) throw new Error('The casualty series are malformed.');
    const key = JSON.stringify([series.tag, series.identities[0]?.firstHour, series.identities[0]?.id]);
    if (tags.has(key)) throw new Error('The casualty series are duplicated.');
    tags.add(key);
  }
  const all = reply.series.map(series => points(series, reply.observed));
  const holders = new Map<string, HourSpan[]>();
  for (const [i, series] of reply.series.entries()) {
    if (!all[i].length) continue;
    const ranges = holders.get(series.tag) ?? [];
    ranges.push({ firstHour: all[i][0].firstHour, lastHour: all[i][all[i].length - 1].lastHour });
    holders.set(series.tag, ranges);
  }
  for (const ranges of holders.values()) {
    ranges.sort((a, b) => a.firstHour - b.firstHour);
    if (ranges.some((range, i) => i > 0 && ranges[i - 1].lastHour >= range.firstHour)) throw new Error('The casualty tag holders overlap.');
  }
  decoded.set(reply, all);
  return reply;
}
interface IndexedSeries { series: CasualtySeries; points: Point[] }
type Membership = HourSpan & { entry: IndexedSeries };
export interface CasualtyIndex { observed: HourSpan[]; chosen: IndexedSeries[]; gap: IndexedSeries[]; membership: IntervalIndex<Membership> }
export function casualtyIndex(reply: CasualtyReply): CasualtyIndex {
  const all = decoded.get(reply) ?? reply.series.map(series => points(series, reply.observed));
  const entries = reply.series.map((series, i) => ({ series, points: all[i] }));
  const chosen = entries.filter(entry => entry.series.membership === null);
  const holders = new Set(chosen.flatMap(entry => entry.points.map(point => point.id)));
  // A clipped alias can still be the selected holder's last sampled name in a gap.
  const gap = entries.flatMap(({ series, points }) => {
    const selected = points.filter(point => holders.has(point.id));
    return selected.length ? [{ series, points: selected }] : [];
  });
  return { observed: reply.observed, chosen, gap,
    membership: new IntervalIndex(entries.flatMap(entry => (entry.series.membership ?? []).map(span => ({ ...span, entry })))) };
}

/** Country identity/stretch bounds for dated looks, independent of top membership. */
export function casualtyLookSegments(reply: CasualtyReply): (tag: string, hour: number) => number | null {
  const all = decoded.get(reply) ?? reply.series.map(series => points(series, reply.observed));
  const byTag = new Map<string, HourSpan[]>();
  for (const [i, series] of reply.series.entries()) {
    const lastHour = all[i].at(-1)?.lastHour;
    if (lastHour === undefined) continue;
    const identities = byTag.get(series.tag) ?? [];
    for (const [j, identity] of series.identities.entries()) {
      const end = Math.min(lastHour, (series.identities[j + 1]?.firstHour ?? Infinity) - 1);
      if (identity.firstHour <= end) identities.push({ firstHour: identity.firstHour, lastHour: end });
    }
    byTag.set(series.tag, identities);
  }
  for (const identities of byTag.values()) identities.sort((a, b) => a.firstHour - b.firstHour);
  return (tag, hour) => {
    const stretch = runAt(reply.observed, hour), identity = runAt(byTag.get(tag) ?? [], hour);
    return stretch && identity ? Math.max(stretch.firstHour, identity.firstHour) : null;
  };
}

/** Legacy atLeast marks incomplete signed history, not a numeric lower bound. */
export interface CountryCasualties {
  id: string; tag: string; hour: number; segment: string; readHour: number | null; atLeast: boolean;
  dealt: bigint | null; taken: bigint | null; knownTaken: bigint | null; unknown: bigint | null;
}

/** One binary search per series; country totals and unknown recovery are native. */
export function casualtiesAt(index: CasualtyIndex, hour: number, includeUnknown: boolean): CountryCasualties[] {
  const observed = runAt(index.observed, hour);
  const members = index.membership.at(hour).map(member => member.entry);
  const gapRows = observed ? null : new Map<string, { firstHour: number; row: CountryCasualties }>();
  const rows = (observed ? [...index.chosen, ...members] : index.gap).flatMap(({ series, points }) => {
    const position = preceding(points, hour, point => point.firstHour);
    if (position < 0) return [];
    const point = points[position], readable = observed && hour <= point.lastHour;
    if (observed && !readable) return [];
    const dealt = readable ? point.dealt : null, knownTaken = readable ? point.taken : null, unknown = readable ? point.unknown : null;
    // Equal consecutive hourly reads confirm at this hour; held tails retain
    // their supplying date. Membership clipping never creates a read.
    const readHour = readable ? Math.min(hour, point.endRead ?? hour) : null;
    const row: CountryCasualties = { id: point.id, tag: series.tag, hour, segment: readable ? point.segment : JSON.stringify([point.id, 'gap']),
      readHour, atLeast: !!readable && series.atLeastFrom !== null && hour >= series.atLeastFrom, dealt, knownTaken, unknown,
      taken: includeUnknown ? knownTaken == null || unknown == null ? null : knownTaken + unknown : knownTaken };
    if (gapRows) {
      // A holder's earlier tags are aliases, not additional unknown countries.
      const previous = gapRows.get(point.id);
      if (!previous || previous.firstHour < point.firstHour) gapRows.set(point.id, { firstHour: point.firstHour, row });
      return [];
    }
    return [row];
  });
  return gapRows ? [...gapRows.values()].map(value => value.row) : rows;
}

/** Unknown losses are display-only, so changing that option needs no new reply. `resolved`
 * is the countries chosen (null for every country), with why none are and when they are known. */
export function casualtyRequest(branchId: string, selection: CountrySelection, fallback: string | null,
  groups: CountryGroupReply | undefined, lastHour: number): { params: CasualtyParams; resolved: ResolvedCountries | null } {
  if (selection.kind === 'all') return { params: { branchId, tags: [], top: null }, resolved: null };
  const resolved = resolveCountries(selection, fallback ? [fallback] : [], groups, lastHour, Number.MAX_SAFE_INTEGER);
  const top = resolved.top ?? null;
  const params: CasualtyParams = { branchId, tags: resolved.tags, top: top == null ? null : { count: top } };
  return { params, resolved };
}

/** Manual picks are projected by native holder in the reply. Smart groups still
 * apply their recorded tag membership locally, then rank by dealt + known taken. */
export function selectCasualties(rows: readonly CountryCasualties[], selection: CountrySelection, groups: CountryGroupReply | undefined,
  hour: number, lastHour: number): CountryCasualties[] {
  let tags: Set<string> | null = null, limit = Infinity;
  if (selection.kind === 'group') {
    if (selection.group.type === 'top') limit = selection.group.count;
    else {
      if (!groups) return [];
      const runs = groupRuns(groups, selection.group);
      const members = runs && groupMembers(runs, selection.membership, hour, lastHour, Infinity, groupUnobserved(groups, selection.group));
      if (!members) return [];
      tags = new Set(members);
    }
  }
  const total = (row: CountryCasualties) => row.dealt == null || row.knownTaken == null ? null : row.dealt + row.knownTaken;
  return rows.filter(row => !tags || tags.has(row.tag)).sort((a, b) => {
    const x = total(a), y = total(b);
    return x == null ? y == null ? compareTags(a.tag, b.tag) : 1 : y == null ? -1 : x === y ? compareTags(a.tag, b.tag) : x > y ? -1 : 1;
  }).slice(0, limit);
}

export const formatCasualties = (value: bigint | null, incomplete = false) => value == null ? '—' : `${incomplete ? '† ' : ''}${formatFixed(value, 0, 0)}`;
const signedCorrection = (row: CountryCasualties) => row.dealt !== null && row.dealt < 0n || row.taken !== null && row.taken < 0n;
export const casualtyRatio = (row: CountryCasualties) => row.atLeast || signedCorrection(row) ? '—' : formatRatio(row.dealt, row.taken);
/** Exact signed labels and ranking stay separate from positive-only bar geometry. */
export const casualtyBarValue = (value: bigint | null) => value === null ? null : Number(value > 0n ? value : 0n);
/** A nonpositive read clears old positive geometry immediately, without changing native identity. */
export const casualtyBarStretch = (row: CountryCasualties) => JSON.stringify([row.segment,
  row.taken !== null && row.taken > 0n, row.dealt !== null && row.dealt > 0n]);

/** The selected countries race by known taken; unknown is display-only. */
export function casualtyRaceEntries(rows: readonly CountryCasualties[]): CountryRaceEntry[] {
  return rows.filter(row => row.taken !== null && row.knownTaken !== null).sort((a, b) => {
    const x = a.knownTaken!, y = b.knownTaken!;
    return x === y ? compareTags(a.tag, b.tag) : x > y ? -1 : 1;
  }).map(row => ({ tag: row.tag, value: casualtyBarValue(row.taken)!, label: formatCasualties(row.taken, row.atLeast), stretch: casualtyBarStretch(row) }));
}

export const SUBTOTAL_NOTE = '† Incomplete total; ratio unavailable.';

export const GAP_NOTE = "Some losses may be missing.";

export function casualtyTooltip(row: CountryCasualties, name: string, unknownLabel: string, hasGap = false): TooltipContent {
  return { title: name, subtitle: dayLabel(row.hour), rows: [
    { label: 'Dealt', value: formatCasualties(row.dealt, row.atLeast) },
    { label: 'Taken (known attackers)', value: formatCasualties(row.knownTaken, row.atLeast) },
    { label: unknownLabel, value: row.unknown == null ? 'Unknown' : formatCasualties(row.unknown), tone: 'muted' },
    { label: 'Taken', value: formatCasualties(row.taken, row.atLeast), strong: true },
    { label: 'Ratio', value: casualtyRatio(row), strong: true },
  ], note: [hasGap ? GAP_NOTE : '', row.atLeast ? SUBTOTAL_NOTE : '',
    signedCorrection(row) ? 'Ratio unavailable for corrections.' : '',
    row.unknown == null ? 'Losses from unknown attackers are unavailable.' : ''].filter(Boolean).join(' ') || undefined };
}
