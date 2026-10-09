import type { CountryGroupReply, OperativePlacementReply, PlacementPeriod } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import { countryMatch, worldFilter } from '../../dashboard/groups.ts';
import { readAt } from '../../dashboard/reads.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { runAt } from '../../charts/series.ts';
import { foldChord } from '../../charts/chord/fold.ts';
import { chordTotals } from '../../charts/chord/totals.ts';
import type { FoldedChord } from '../../charts/chord/fold';
import type { ChordHit } from '../../charts/chord/layout';
import type { TooltipContent } from '../../charts/tooltip';

const tag = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && !v.includes('\0');
const span = (v: { firstHour: number; lastHour: number }) => Number.isSafeInteger(v?.firstHour) && Number.isSafeInteger(v?.lastHour) && v.firstHour <= v.lastHour;
const ordered = (runs: readonly { firstHour: number; lastHour: number }[]) => runs.every((r, i) => span(r) && (i === 0 || runs[i - 1].lastHour < r.firstHour));
const statuses = new Set(['recorded', 'featureAbsent', 'tablesNotRecorded']);

/** Reject inconsistent coverage too: a value/read cannot live inside a gap or
 * unavailable period, and equal pair identities must not be counted twice. */
export function validatePlacementsReply(value: unknown): OperativePlacementReply {
  const r = value as OperativePlacementReply;
  const fail = () => { throw new Error('The operative placements reply is malformed.'); };
  if (!r || typeof r !== 'object' || typeof r.branchId !== 'string' || !/^\d+$/.test(r.branchId)
    || !Array.isArray(r.pairs) || !Array.isArray(r.periods) || !Array.isArray(r.readHours)
    || !Array.isArray(r.warnings) || !r.warnings.every(w => typeof w === 'string')) return fail();
  if (!r.periods.every(p => p && statuses.has(p.availability)) || !ordered(r.periods)
    || !r.readHours.every((h, i) => Number.isSafeInteger(h) && (i === 0 || r.readHours[i - 1] < h))) return fail();
  const ids = new Set<string>();
  const endpoints = new Map<string, OperativePlacementReply['pairs'][number]['runs']>();
  const reads = new Set(r.readHours);
  const memberships = new Map<string, { firstHour: number; lastHour: number; name: string | null; unnamed: boolean }[]>();
  for (const pair of r.pairs) {
    if (!pair || !tag(pair.id) || !tag(pair.owner) || !tag(pair.target) || ids.has(pair.id) || !Array.isArray(pair.runs)
      || !pair.runs.every(run => run && Number.isSafeInteger(run.count) && run.count > 0 && run.count <= 4294967295
        && Array.isArray(run.operatives) && run.operatives.length === run.count
        && run.operatives.every(o => o && typeof o.id === 'string' && /^(0|[1-9]\d{0,19})$/.test(o.id) && BigInt(o.id) <= 18446744073709551615n
          && (o.name === null || typeof o.name === 'string' && o.name.length > 0)
          && typeof o.unnamed === 'boolean' && (!o.unnamed || o.name === null))
        && new Set(run.operatives.map(o => o.id)).size === run.count) || !ordered(pair.runs)) return fail();
    ids.add(pair.id);
    const endpointKey = JSON.stringify([pair.owner, pair.target]);
    const previous = endpoints.get(endpointKey) ?? [];
    endpoints.set(endpointKey, [...previous, ...pair.runs]);
    for (const run of pair.runs) {
      for (const o of run.operatives) { const runs = memberships.get(o.id) ?? []; runs.push({ ...o, firstHour: run.firstHour, lastHour: run.lastHour }); memberships.set(o.id, runs); }
      const period = runAt(r.periods, run.firstHour);
      // Hourly identity presentation may split held daily memberships.
      if (period?.availability !== 'recorded' || run.lastHour > period.lastHour) return fail();
    }
  }
  // An opaque tag may name successive identities, but never two pairs at once.
  for (const runs of endpoints.values()) if (!ordered(runs.sort((a, b) => a.firstHour - b.firstHour))) return fail();
  for (const runs of memberships.values()) {
    runs.sort((a, b) => a.firstHour - b.firstHour);
    if (!ordered(runs)) return fail();
    for (let i = 0; i < runs.length; i++) {
      const run = runs[i], previous = runs[i - 1], next = runs[i + 1];
      const same = (other: typeof run | undefined) => other?.name === run.name && other?.unnamed === run.unnamed;
      if (!reads.has(run.firstHour) && (previous?.lastHour !== run.firstHour - 1 || !same(previous))) return fail();
      const period = runAt(r.periods, run.lastHour)!;
      if (run.lastHour < period.lastHour && !reads.has(run.lastHour + 1)
        && (next?.firstHour !== run.lastHour + 1 || !same(next))) return fail();
    }
  }
  for (const period of r.periods) if (period.availability === 'recorded') {
    if (!reads.has(period.firstHour)) return fail();
  }
  if (r.readHours.some(h => runAt(r.periods, h)?.availability !== 'recorded')) return fail();
  return r;
}

export interface PlacementAt { id: string; owner: string; target: string; count: number; operatives: OperativePlacementReply['pairs'][number]['runs'][number]['operatives'] }
export interface PlacementsAt { period: PlacementPeriod | null; read: number | null; placements: PlacementAt[] }
export function placementsAt(reply: OperativePlacementReply, hour: number): PlacementsAt {
  const period = runAt(reply.periods, hour);
  if (period?.availability !== 'recorded') return { period, read: null, placements: [] };
  return { period, read: readAt(reply.readHours, hour), placements: reply.pairs.flatMap(pair => {
    const run = runAt(pair.runs, hour);
    return run ? [{ id: pair.id, owner: pair.owner, target: pair.target, count: run.count, operatives: run.operatives }] : [];
  }) };
}
export const countryNodeId = (country: string) => JSON.stringify(['country', country]);
export const OTHER_ID = JSON.stringify(['other']);

export function placementTotals(placements: readonly PlacementAt[]): Map<string, bigint> {
  return new Map([...chordTotals(placements, n => n.owner, n => n.target, n => BigInt(n.count))]
    .map(([id, total]) => [id, total.incoming + total.outgoing]));
}
export function rankedCountries(totals: ReadonlyMap<string, bigint>): string[] {
  return [...totals.keys()].sort((a, b) => totals.get(a)! === totals.get(b)! ? compareTags(a, b) : totals.get(a)! > totals.get(b)! ? -1 : 1);
}

/** All means every placement. A chosen country/group narrows either
 * endpoint; human membership is only an explicit view choice, never capture. */
export function selectPlacements(placements: readonly PlacementAt[], selection: CountrySelection, groups: CountryGroupReply | undefined, hour: number, end: number, selectedTag?: string | null): { placements: PlacementAt[]; reason: string | null } {
  if (selection.kind === 'group' && selection.group.type === 'top') {
    const tags = new Set(rankedCountries(placementTotals(placements)).slice(0, selection.group.count));
    return { placements: placements.filter(n => tags.has(n.owner) || tags.has(n.target)), reason: null };
  }
  const fallback = selectedTag ? [selectedTag] : [];
  const match = countryMatch(selection, fallback, groups, end);
  if (!match.ready) return { placements: [], reason: 'Loading native country identities…' };
  const filter = worldFilter(selection, fallback, groups, hour, end);
  if (filter.reason) return { placements: [], reason: filter.reason };
  return { placements: placements.filter(n => match.matches(n.owner, hour) || match.matches(n.target, hour)), reason: null };
}

export interface PlacementChord extends FoldedChord {
  placements: Map<string, PlacementAt>;
  countries: Map<string, string>;
  read: number;
  labels: CountryLabel;
  totals: Map<string, { incoming: bigint; outgoing: bigint }>;
}
/** Integer counts stay exact; totals use BigInt even for folded groups. */
export function placementChord(placements: readonly PlacementAt[], most: number, read: number, labels: CountryLabel, color: (tag: string) => string, muted: string): PlacementChord {
  const totals = placementTotals(placements), ranked = rankedCountries(totals);
  const keep = new Set(ranked.slice(0, most).map(countryNodeId));
  const countries = new Map([...totals.keys()].sort(compareTags).map(tag => [countryNodeId(tag), tag]));
  const data = { nodes: [...countries].map(([id, tag]) => {
      const label = labels(tag);
      return { id, label: label.name, flag: label.flag, color: color(tag),
        countryFlag: { flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason } };
    }),
    flows: placements.map(n => ({ id: n.id, source: countryNodeId(n.owner), target: countryNodeId(n.target), value: n.count })) };
  const folded = foldChord(data, keep, { id: OTHER_ID, label: 'Other countries', color: muted });
  return { ...folded, placements: new Map(placements.map(n => [n.id, n])), countries, read, labels,
    totals: chordTotals(placements, n => n.owner, n => n.target, n => BigInt(n.count)) };
}

type PlacementFact = CountryFlagLook & { name: string; color: string };

/** Reuse the exact data object until values, the read, membership or recorded
 * presentation change. Hour ticks alone never rebuild or refold the graph. */
export function placementChordMemo() {
  let previous: { placements: readonly PlacementAt[]; most: number; read: number; muted: string;
    facts: Map<string, PlacementFact>; chord: PlacementChord } | null = null;
  return (placements: readonly PlacementAt[], most: number, read: number, labels: CountryLabel,
    color: (tag: string) => string, muted: string): PlacementChord => {
    const facts = new Map<string, PlacementFact>();
    for (const n of placements) for (const tag of [n.owner, n.target]) if (!facts.has(tag)) {
      const label = labels(tag);
      facts.set(tag, { name: label.name, flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason, color: color(tag) });
    }
    if (previous && previous.most === most && previous.read === read && previous.muted === muted
      && previous.placements.length === placements.length && placements.every((n, i) => {
        const old = previous!.placements[i];
        return n.id === old.id && n.owner === old.owner && n.target === old.target && n.count === old.count && n.operatives.length === old.operatives.length && n.operatives.every((o, j) => o.id === old.operatives[j].id && o.name === old.operatives[j].name && o.unnamed === old.operatives[j].unnamed);
      }) && facts.size === previous.facts.size && [...facts].every(([tag, fact]) => {
        const old = previous!.facts.get(tag);
        return old?.name === fact.name && old.flag === fact.flag && old.flagPending === fact.flagPending
          && old.flagReason === fact.flagReason && old.color === fact.color;
      })) return previous.chord;
    const chord = placementChord(placements, most, read, tag => facts.get(tag)!, tag => facts.get(tag)!.color, muted);
    previous = { placements, most, read, muted, facts, chord };
    return chord;
  };
}

export function placementTooltip(hit: ChordHit, chord: PlacementChord): TooltipContent | null {
  const note = `Read ${formatDateTime(chord.read)}; operative placements are read once a day.`;
  const title = (n: PlacementAt) => `${chord.labels(n.owner).name} has ${n.count} ${n.count === 1 ? 'operative' : 'operatives'} in ${chord.labels(n.target).name}`;
  const names = (n: PlacementAt) => n.operatives.map(o => ({ label: o.unnamed ? 'Unnamed operative' : o.name ?? `Name not recorded (operative ${o.id})` }));
  if (hit.kind === 'flow') {
    const members = chord.flows.get(hit.id)?.map(id => chord.placements.get(id)!);
    if (!members?.length) return null;
    if (members.length === 1) return { title: title(members[0]), rows: names(members[0]), note };
    return { title: 'Operative placements', rows: [
      ...members.flatMap(n => [{ label: title(n), strong: true }, ...names(n)]),
      { label: 'Total operatives', value: members.reduce((sum, n) => sum + BigInt(n.count), 0n).toString(), strong: true },
    ], note };
  }
  const memberIds = chord.nodes.get(hit.id);
  if (!memberIds) return null;
  const tags = memberIds.map(id => chord.countries.get(id)!);
  let incoming = 0n, outgoing = 0n;
  for (const tag of tags) { const total = chord.totals.get(tag)!; outgoing += total.outgoing; incoming += total.incoming; }
  return { title: hit.id === OTHER_ID ? 'Other countries' : chord.labels(tags[0]).name,
    rows: [{ label: 'Operatives sent', value: outgoing.toString() }, { label: 'Operatives received', value: incoming.toString() },
      ...(hit.id === OTHER_ID ? tags.map(tag => ({ label: chord.labels(tag).name, countryFlag: chord.labels(tag), tone: 'muted' as const })) : [])], note };
}
