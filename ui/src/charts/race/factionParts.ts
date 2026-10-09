import { runAt } from '../series.ts';
import type { Run, Span } from '../series';
import { CLOSED, pinnedBreakdown, reduceDrill } from '../drill/drillState.ts';
import type { DrillState } from '../drill/drillState';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { BoundTrack, GroupMembership, WidgetSpec } from '../../dashboard/model';
import type { CountryIdentityRun, FactionMemberTrack, FactionTotalsReply } from '../../generated/dashboard';

export const FACTION_MEMBERS = { id: 'members', label: 'Members' };
export const FACTION_SELECTION = 'memberFaction';
type ValueRun = Run & { raw: bigint | null };
type Witness = CountryIdentityRun & { tag: string };
export interface FactionCountry { id: string; witnesses: Witness[] }
export interface FactionPart { id: string; runs: ValueRun[]; counted: Span[]; witnesses: Witness[] }
export interface FactionParts { parts: FactionPart[]; census: FactionCountry[]; total: Run[]; remainder: ValueRun[]; unidentified: number }
type Delta = { value: bigint; unknown: number; fresh: boolean };

/** Localised names and diagnostics never own a numeric plot or pending pick. */
export function factionNumbers(reply: FactionTotalsReply): FactionTotalsReply {
  return { ...reply, warnings: [], series: reply.series.map(series => ({ ...series, names: [] })) };
}

function delta(events: Map<number, Delta>, hour: number, value: bigint, unknown: number, fresh = false) {
  const before = events.get(hour) ?? { value: 0n, unknown: 0, fresh: false };
  events.set(hour, { value: before.value + value, unknown: before.unknown + unknown, fresh: before.fresh || fresh });
}

function append(runs: ValueRun[], firstHour: number, lastHour: number, raw: bigint | null, fresh = false) {
  if (lastHour < firstHour) return;
  const previous = runs.at(-1);
  if (!fresh && previous && previous.lastHour + 1 === firstHour && previous.raw === raw) previous.lastHour = lastHour;
  else runs.push({ firstHour, lastHour, value: Number(raw ?? 0n), raw, ...(fresh ? { breakBefore: true } : {}) });
}

function eventRuns(events: Map<number, Delta>, extent: Span): ValueRun[] {
  const runs: ValueRun[] = [];
  const hours = [...new Set([extent.firstHour, extent.lastHour + 1, ...events.keys()])].sort((a, b) => a - b);
  let value = 0n;
  let unknown = 0;
  for (let index = 0; index < hours.length - 1; index++) {
    const change = events.get(hours[index]);
    if (change) { value += change.value; unknown += change.unknown; }
    append(runs, Math.max(extent.firstHour, hours[index]), Math.min(extent.lastHour, hours[index + 1] - 1), unknown ? null : value, change?.fresh);
  }
  return runs;
}

/** Each tag walks only its own roster, value and identity boundaries. Different
 * tags of one native country share a part; a reused tag never shares one. */
export function factionParts(reply: FactionTotalsReply, factionId: string, extent: Span): FactionParts | null {
  if (reply.memberTracks === undefined) return null;
  const faction = reply.series.find(series => series.id === factionId);
  if (!faction) return null;
  const counted = new Map<string, Span[]>();
  for (const member of faction.countedMembers) {
    const spans = counted.get(member.tag) ?? [];
    spans.push(member); counted.set(member.tag, spans);
  }
  const tracks = new Map(reply.memberTracks.map(track => [track.tag, track]));
  const countries = new Map<string, Witness[]>();
  for (const track of reply.memberTracks) for (const identity of track.identities) {
    const id = JSON.stringify([identity.slot, identity.countryId]);
    const witnesses = countries.get(id) ?? [];
    witnesses.push({ ...identity, tag: track.tag }); countries.set(id, witnesses);
  }
  const census = [...countries].sort(([a], [b]) => a.localeCompare(b)).map(([id, witnesses]) =>
    ({ id, witnesses: witnesses.sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag)) }));
  const renames = new Map(census.map(country => [country.id, new Set(country.witnesses.flatMap((witness, index) => {
    const previous = country.witnesses[index - 1];
    return previous && previous.tag !== witness.tag && previous.lastHour + 1 === witness.firstHour ? [witness.firstHour] : [];
  }))]));
  const native = new Map<string, { events: Map<number, Delta>; counted: Span[]; witnesses: Witness[] }>();
  let unidentified = 0;
  for (const [tag, spans] of counted) {
    spans.sort((a, b) => a.firstHour - b.firstHour);
    const track: FactionMemberTrack | undefined = tracks.get(tag);
    const hours = [...new Set([...spans, ...(track?.runs ?? []), ...(track?.identities ?? [])]
      .flatMap(run => [run.firstHour, run.lastHour + 1]))].sort((a, b) => a - b);
    let missingIdentity = false;
    for (let index = 0; index < hours.length - 1; index++) {
      const firstHour = Math.max(hours[index], extent.firstHour);
      const lastHour = Math.min(hours[index + 1] - 1, extent.lastHour);
      if (lastHour < firstHour || !runAt(spans, firstHour)) continue;
      const identity = runAt(track?.identities ?? [], firstHour);
      if (!identity) { missingIdentity = true; continue; }
      const id = JSON.stringify([identity.slot, identity.countryId]);
      const part = native.get(id) ?? { events: new Map<number, Delta>(), counted: [], witnesses: [] };
      part.counted.push({ firstHour, lastHour });
      const supplying = runAt(track?.runs ?? [], firstHour);
      const value = supplying?.value;
      const raw = value == null ? 0n : BigInt(value);
      const unknown = value == null ? 1 : 0;
      delta(part.events, firstHour, raw, unknown, supplying?.startsSegment && supplying.firstHour === firstHour
        && !renames.get(id)?.has(firstHour));
      delta(part.events, lastHour + 1, -raw, -unknown);
      // Keep the identity's supplying interval, including zero or unknown values.
      const previous = part.witnesses.at(-1);
      if (!previous || previous.tag !== tag || previous.firstHour !== identity.firstHour) part.witnesses.push({ ...identity, tag });
      native.set(id, part);
    }
    if (missingIdentity) unidentified++;
  }
  const parts = [...native].map(([id, part]) => {
    const counted: Span[] = [];
    for (const span of part.counted.sort((a, b) => a.firstHour - b.firstHour)) {
      const previous = counted.at(-1);
      if (previous && span.firstHour <= previous.lastHour + 1) previous.lastHour = Math.max(previous.lastHour, span.lastHour);
      else counted.push({ ...span });
    }
    return { id, runs: eventRuns(part.events, extent), counted,
      witnesses: part.witnesses.sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag)) };
  });
  const total = faction.runs.map(run => ({ ...run, ...(run.startsSegment ? { breakBefore: true } : {}) }));
  // Sweep exact known contributions once. A known whole can retain an exact
  // unassigned amount; an unknown whole always breaks the stack.
  const events = new Map<number, Delta>();
  for (const part of parts) for (const run of part.runs) if (run.raw != null) {
    delta(events, run.firstHour, run.raw, 0); delta(events, run.lastHour + 1, -run.raw, 0);
  }
  const known = eventRuns(events, extent);
  const hours = [...new Set([...known, ...total].flatMap(run => [run.firstHour, run.lastHour + 1]))].sort((a, b) => a - b);
  const remainder: ValueRun[] = [];
  for (let index = 0; index < hours.length - 1; index++) {
    const firstHour = Math.max(extent.firstHour, hours[index]);
    const lastHour = Math.min(extent.lastHour, hours[index + 1] - 1);
    const whole = runAt(total, firstHour);
    if (whole) append(remainder, firstHour, lastHour, BigInt(whole.value) - (runAt(known, firstHour)?.raw ?? 0n), whole.breakBefore && whole.firstHour === firstHour);
  }
  return { parts, census, total, remainder, unidentified };
}

/** Request a name only at an hour proved to belong to this native identity. */
export function factionPartWitness(part: FactionCountry, hour: number): { tag: string; hour: number; firstHour: number } | null {
  let last: Witness | null = null;
  for (const witness of part.witnesses) {
    if (witness.firstHour > hour) break;
    if (!last || witness.lastHour > last.lastHour) last = witness;
    if (witness.lastHour >= hour) return { tag: witness.tag, hour: Math.floor(hour), firstHour: witness.firstHour };
  }
  return last ? { tag: last.tag, hour: last.lastHour, firstHour: last.firstHour } : null;
}

export function factionPartShare(raw: bigint | null, total: bigint): string {
  if (raw == null || total === 0n) return '—';
  const numerator = raw * 1000n;
  const negative = (numerator < 0n) !== (total < 0n);
  const absolute = numerator < 0n ? -numerator : numerator;
  const divisor = total < 0n ? -total : total;
  const tenths = (absolute + divisor / 2n) / divisor;
  return `${negative && tenths ? '-' : ''}${tenths / 10n}${tenths % 10n ? `.${tenths % 10n}` : ''}%`;
}

export interface FactionPick extends DrillState { key: unknown; origin: number; ids: readonly string[]; selected: string | null }
export function factionPick(key: unknown, hour: number, id: string | undefined, choices?: readonly string[],
  following?: { playing: boolean; playhead: number | null }): FactionPick {
  const ids = [...new Set(choices ?? (id ? [id] : []))];
  const selected = ids.length === 1 ? ids[0] : null;
  const state = reduceDrill(CLOSED, { type: 'open', breakdown: FACTION_MEMBERS.id, hour,
    playing: !!selected && (following?.playing ?? false), playhead: selected ? following?.playhead ?? null : null });
  return { ...state, key, origin: Math.floor(hour), ids, selected };
}
export function chooseFaction(pick: FactionPick, key: unknown, id: string): FactionPick {
  return pick.key === key && pick.ids.includes(id) ? { ...pick, selected: id } : pick;
}

/** The native faction key is campaign and branch scoped, including its proven
 * origin hour. A saved choice cannot select another campaign's reused handle. */
export function factionSelection(track: BoundTrack, factionId: string, hour: number): string {
  return setToken(JSON.stringify([track.source.metadata.campaignKey, track.branch.id, factionId, Math.floor(hour)]));
}
export function savedFactionSelection(value: unknown, track: BoundTrack): { id: string; origin: number } | null {
  if (typeof value !== 'string' || !value) return null;
  try {
    const [campaign, branch, id, origin] = JSON.parse(contentKey(value));
    return campaign === track.source.metadata.campaignKey && branch === track.branch.id && typeof id === 'string'
      && /^\d+$/.test(id) && Number.isInteger(origin) && origin >= track.branch.firstHour && origin <= track.branch.lastHour
      ? { id, origin } : null;
  } catch { return null; }
}
export function pinFactionParts(widget: WidgetSpec, track: BoundTrack, factionId: string, origin: number,
  hour: number, follow: boolean, membership: GroupMembership): WidgetSpec {
  const pinned = pinnedBreakdown(widget, FACTION_MEMBERS, track.id);
  const offset = Math.floor(hour) - track.branch.firstHour;
  return { ...pinned, countries: { kind: 'group', group: { type: 'faction', id: factionId }, aggregate: true, membership },
    time: follow ? { mode: 'follow' } : { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } },
    options: { ...pinned.options, view: 'race', [FACTION_SELECTION]: factionSelection(track, factionId, origin) } };
}
