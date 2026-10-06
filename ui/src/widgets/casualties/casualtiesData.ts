import type { CasualtyParams, CasualtyReply, CasualtySeries, CasualtyValues, CountryGroupReply, HourSpan } from '../../generated/dashboard';
import { runAt } from '../../charts/series.ts';
import { IntervalIndex } from '../../charts/intervalIndex.ts';
import { formatFixed } from '../../charts/valueAxis.ts';
import { formatRatio } from '../../charts/butterfly/layout.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { groupMembers, groupRuns, groupUnobserved, resolveCountries } from '../../dashboard/groups.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountrySelection } from '../../dashboard/model';
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
  if (!Array.isArray(column) || column.length !== 2 || !raw(column[0]) || !Array.isArray(column[1]) || column[1].length !== Math.max(0, count - 1)
    || !column[1].every(delta => typeof delta === 'string' ? raw(delta) : Number.isSafeInteger(delta))) throw new Error('The casualty values are malformed.');
  let value = BigInt(column[0]); const result = count ? [value] : [];
  for (const delta of column[1]) { value += BigInt(delta); result.push(value); }
  return result;
}
interface Point { firstHour: number; lastHour: number; read: number; endRead: number | null; dealt: bigint; taken: bigint; unknown: bigint | null; id: string; segment: string }
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
      result.push({ firstHour, lastHour, read: reads[i], endRead, dealt: dealt[i], taken: taken[i], unknown: unknown[i] ?? null, id, segment: lastSegment });
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
export interface CasualtyIndex { observed: HourSpan[]; chosen: IndexedSeries[]; membership: IntervalIndex<Membership> }
export function casualtyIndex(reply: CasualtyReply): CasualtyIndex {
  const all = decoded.get(reply) ?? reply.series.map(series => points(series, reply.observed));
  const entries = reply.series.map((series, i) => ({ series, points: all[i] }));
  return { observed: reply.observed, chosen: entries.filter(entry => entry.series.membership === null),
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

export interface CountryCasualties {
  id: string; tag: string; hour: number; segment: string; readHour: number | null; atLeast: boolean;
  dealt: bigint | null; taken: bigint | null; knownTaken: bigint | null; unknown: bigint | null;
}

/** One binary search per series; country totals and unknown recovery are native. */
export function casualtiesAt(index: CasualtyIndex, hour: number, includeUnknown: boolean): CountryCasualties[] {
  const observed = runAt(index.observed, hour);
  const members = index.membership.at(hour).map(member => member.entry);
  return [...index.chosen, ...members].flatMap(({ series, points }) => {
    const position = preceding(points, hour, point => point.firstHour);
    if (position < 0) return [];
    const point = points[position], readable = observed && hour <= point.lastHour;
    if (observed && !readable) return [];
    const dealt = readable ? point.dealt : null, knownTaken = readable ? point.taken : null, unknown = readable ? point.unknown : null;
    // Every day-end inside an equal-value run confirmed the value. Exceptional
    // endpoints also confirmed it, but membership clipping did not read anything.
    const confirmedUntil = Math.min(hour, point.endRead ?? hour);
    const dayEnd = confirmedUntil - ((confirmedUntil % 24 + 24) % 24) - 1;
    const dailyRead = (confirmedUntil % 24 + 24) % 24 === 23 ? confirmedUntil : dayEnd;
    const readHour = readable ? Math.max(point.read, dailyRead, point.endRead !== null && hour >= point.endRead ? point.endRead : point.read) : null;
    return [{ id: point.id, tag: series.tag, hour, segment: readable ? point.segment : JSON.stringify([point.id, 'gap']),
      readHour, atLeast: !!readable && series.atLeastFrom !== null && hour >= series.atLeastFrom, dealt, knownTaken, unknown,
      taken: includeUnknown ? knownTaken == null || unknown == null ? null : knownTaken + unknown : knownTaken }];
  });
}

/** Unknown losses are display-only, so changing that option needs no new reply. */
export function casualtyRequest(branchId: string, selection: CountrySelection, fallback: string | null,
  groups: CountryGroupReply | undefined, lastHour: number) {
  if (selection.kind === 'all') return { params: { branchId, tags: [], top: null } as CasualtyParams, reason: null };
  const resolved = resolveCountries(selection, fallback ? [fallback] : [], groups, lastHour, Number.MAX_SAFE_INTEGER);
  const top = resolved.top ?? null;
  const params: CasualtyParams = { branchId, tags: resolved.tags, top: top == null ? null : { count: top } };
  return { params, reason: resolved.reason };
}

/** Smart-group membership, then dealt + known taken, tied by opaque UTF-8 tags. */
export function selectCasualties(rows: readonly CountryCasualties[], selection: CountrySelection, groups: CountryGroupReply | undefined,
  hour: number, lastHour: number): CountryCasualties[] {
  let tags: Set<string> | null = null, limit = Infinity;
  if (selection.kind === 'tags') tags = new Set(selection.tags);
  else if (selection.kind === 'group') {
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

export const formatCasualties = (value: bigint | null, atLeast = false) => value == null ? '—' : `${atLeast ? '≥ ' : ''}${formatFixed(value, 0, 0)}`;
export const casualtyRatio = (row: CountryCasualties) => row.atLeast ? '—' : formatRatio(row.dealt, row.taken);

/** The selected countries race by known taken; unknown is display-only. */
export function casualtyRaceEntries(rows: readonly CountryCasualties[]): CountryRaceEntry[] {
  return rows.filter(row => row.taken !== null && row.knownTaken !== null).sort((a, b) => {
    const x = a.knownTaken!, y = b.knownTaken!;
    return x === y ? compareTags(a.tag, b.tag) : x > y ? -1 : 1;
  }).map(row => ({ tag: row.tag, value: Number(row.taken), label: formatCasualties(row.taken, row.atLeast), stretch: row.segment }));
}

export const GAP_NOTE = "Wars fought entirely while the recording was not running are not counted.";

export function casualtyTooltip(row: CountryCasualties, name: string, includeUnknown: boolean, unknownLabel: string, hasGap = false): TooltipContent {
  const dated = row.readHour == null ? 'No sample at this hour.' : `As of ${formatDateTime(row.readHour)}; casualties are sampled once a day.`;
  return { title: name, subtitle: formatDateTime(row.hour), rows: [
    { label: 'Dealt', value: formatCasualties(row.dealt, row.atLeast) },
    { label: 'Taken (recorded attacker)', value: formatCasualties(row.knownTaken, row.atLeast) },
    { label: unknownLabel, value: row.unknown == null ? 'Not recoverable exactly' : formatCasualties(row.unknown), tone: 'muted' },
    { label: 'Included in taken', value: includeUnknown ? 'Yes' : 'No', tone: 'muted' },
    { label: 'Taken', value: formatCasualties(row.taken, row.atLeast), strong: true },
    { label: 'Ratio', value: casualtyRatio(row), strong: true },
  ], note: `${dated} The game’s casualty totals across land, sea and air. ${hasGap ? GAP_NOTE + ' ' : ''}${row.atLeast
    ? 'At least: a war ended while the recording was not running, so its last losses are missing. ' : ''}${row.unknown == null
    ? 'Losses without a recorded attacker cannot be recovered exactly because earlier readings are missing or disagree.' : 'Losses with no recorded attacker add only to taken.'}` };
}
