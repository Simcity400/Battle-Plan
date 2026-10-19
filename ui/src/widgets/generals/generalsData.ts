import { contentTitle } from '../../dashboard/contentTitles.ts';
import { clipToSpans, runAt } from '../../charts/series.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { stackedSpans } from '../../charts/timeline/layout.ts';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { firstHourOfMonth, formatDateTime, gameDate, monthIndex } from '../../dashboard/calendar.ts';
import { createWidget } from '../../dashboard/model.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options.ts';
import { GROW_WITH_PLAYBACK_OPTION } from '../general-career/definition.ts';
import type { NameAt } from '../../dashboard/countryFacts';
import { nameKey } from '../../dashboard/countryFacts.ts';
import type { LeaderSelection, LeaderChangeDate } from '../../generated/dashboard';
import type { LeaderCareer as WireLeaderCareer, LeaderPerson, LeaderRole as WireLeaderRole, LeaderValues, UnitLeaderPortraitsReply, UnitLeadersReply } from '../../generated/dashboard';

import { object, opaqueTag, signed32 as integer, unsigned32 as u32, unsigned64 as handle } from '../../dashboard/validation.ts';
import { isSpan as span, isOrderedSpans as ordered } from '../../charts/series.ts';

export interface LeaderRun { firstHour: number; lastHour: number; value: LeaderValues }
export interface TraitGain { hour: number; token: number; date: LeaderChangeDate }
export type LeaderCareer = Omit<WireLeaderCareer, 'runs' | 'traitGains'> & { runs: LeaderRun[]; traitGains: TraitGain[] };
export type LeaderRole = WireLeaderRole & { attributeNames: [string | null, string | null, string | null, string | null] };
export type LeaderObservation = UnitLeadersReply['observed'][number] & { landCommands: boolean; navalCommands: boolean };
export type UnitLeadersData = Omit<UnitLeadersReply, 'leaders' | 'roles' | 'observed'> & { leaders: LeaderCareer[]; roles: LeaderRole[]; observed: LeaderObservation[]; preparing?: string[] };
const text = (v: unknown) => v === null || typeof v === 'string';
const png = (v: unknown) => typeof v === 'string' && v.startsWith('data:image/png;base64,');
const ascending = (a: number, b: number) => a - b;
/** Hours coded first absolute, then as deltas. */
const decode = (deltas: readonly number[]) => { let hour = 0; return deltas.map(delta => hour += delta); };

export function validateLeaderPortraitsReply(input: unknown): UnitLeaderPortraitsReply {
  if (!object(input) || !Array.isArray(input.portraits)
    || !Array.isArray(input.warnings) || !input.warnings.every(warning => typeof warning === 'string')
    || !input.portraits.every(p => object(p) && handle(p.reference) && (p.image === null || png(p.image)) && text(p.missing))
    || new Set(input.portraits.map(p => p.reference)).size !== input.portraits.length) {
    throw new Error('The unit leader portraits reply is malformed.');
  }
  return input as unknown as UnitLeaderPortraitsReply;
}

/** Validate the compact wire format once, then expose ordinary runs to the kits. */
export function validateUnitLeadersReply(input: unknown): UnitLeadersData {
  const bad = () => { throw new Error('The unit leaders reply is malformed.'); };
  if (!object(input) || !handle(input.branchId) || !integer(input.firstHour) || !Array.isArray(input.leaders) || !Array.isArray(input.people)
    || !Array.isArray(input.roles) || !Array.isArray(input.traits)
    || !ordered(input.observed) || !input.observed.every(s => ['leaders', 'landBattles', 'navalBattles', 'landCommands', 'navalCommands'].every(k => typeof s[k] === 'boolean'))
    || !(input.selectionTags === undefined || Array.isArray(input.selectionTags) && input.selectionTags.every(opaqueTag))
    || typeof input.resultsMissing !== 'string' || typeof input.navyCommandsMissing !== 'string'
    || !Array.isArray(input.warnings) || !input.warnings.every(w => typeof w === 'string')) return bad();
  if (input.preparing !== undefined && (!Array.isArray(input.preparing) || !input.preparing.every(field => typeof field === 'string'))) return bad();
  const firstHour = input.firstHour;
  for (const p of input.people) if (!object(p) || !u32(p.characterId) || !text(p.token) || !text(p.name) || !integer(p.portraitHour) || !handle(p.portraitReference)) return bad();
  for (const t of input.traits) if (!object(t) || typeof t.token !== 'string' || !text(t.name) || !text(t.missing) || !(t.icon === null || png(t.icon))) return bad();
  for (const r of input.roles) if (!object(r) || !(r.owner === null || opaqueTag(r.owner)) || !integer(r.category) || !text(r.rank)
    || !Array.isArray(r.attributeNames) || r.attributeNames.length !== 4 || !r.attributeNames.every(text)
    || !(r.kind === 1 || r.kind === 2) || !(r.maximumSkill === null || integer(r.maximumSkill) && r.maximumSkill >= 0)) return bad();
  const keys = new Set<string>(); const leaders: LeaderCareer[] = [];
  const traitCount = input.traits.length;
  /** Sorted delta-coded hours, none before the first read. */
  const visibleHours = (deltas: unknown) => {
    let hour = 0;
    return Array.isArray(deltas) && deltas.every((delta, i) => Number.isSafeInteger(delta) && (!i || delta >= 0) && integer(hour += delta) && hour >= firstHour) ? decode(deltas) : null;
  };
  for (const l of input.leaders) {
    if (!object(l) || !handle(l.reference) || keys.has(String(l.reference)) || !Array.isArray(l.runs)
      || !Array.isArray(l.traitGains) || !Array.isArray(l.commands) || !(l.partialFrom === null || integer(l.partialFrom))) return bad();
    // Careers list every participation, leaderboards only when counts change, choices neither.
    const battleStarts = l.battleStarts ?? []; const startReads = l.battleStartReadHours ?? []; const unknownStarts = l.unknownBattleStarts ?? [];
    if (!Array.isArray(battleStarts) || !Array.isArray(startReads) || startReads.length !== battleStarts.length || !Array.isArray(unknownStarts)) return bad();
    keys.add(String(l.reference));
    const runs: LeaderRun[] = [];
    for (const r of l.runs) {
      if (!Array.isArray(r) || r.length !== 8 || !integer(r[0]) || !integer(r[1]) || r[0] > r[1]
        || !u32(r[2]) || r[2] >= input.people.length || !u32(r[3]) || r[3] >= input.roles.length || !(r[4] === null || integer(r[4]))
        || !Array.isArray(r[5]) || r[5].length !== 4 || !r[5].every(integer)
        || !(r[6] === null || Array.isArray(r[6]) && r[6].every(t => u32(t) && t < traitCount))) return bad();
      runs.push({ firstHour: r[0], lastHour: r[1], value: { person: r[2], role: r[3], skill: r[4], components: r[5] as [number, number, number, number], traits: r[6], progress: r[7] } });
    }
    for (const run of runs) {
      const progress = run.value.progress;
      const fixed = (value: unknown) => typeof value === 'string' && /^-?(0|[1-9][0-9]*)$/.test(value) && BigInt(value) >= -(1n << 63n) && BigInt(value) < 1n << 63n;
      if (progress !== null && (!object(progress) || !fixed(progress.experienceRaw)
        || !(progress.nextRequiredRaw === null || fixed(progress.nextRequiredRaw))
        || !(progress.training === null || Array.isArray(progress.training) && progress.training.every(t => object(t)
          && (t.token === null || text(t.token)) && (t.name === null || text(t.name)) && fixed(t.experienceRaw)
          && (t.requiredRaw === null || fixed(t.requiredRaw))))
        || !(progress.factors === null || Array.isArray(progress.factors) && progress.factors.every(t => object(t)
          && (t.token === null || text(t.token)) && (t.name === null || text(t.name)) && fixed(t.factorRaw))))) return bad();
    }
    const traitGains: TraitGain[] = [];
    for (const g of l.traitGains) {
      if (!Array.isArray(g) || g.length !== 3 || !integer(g[0]) || !u32(g[1]) || g[1] >= traitCount || !(g[2] === 0 || g[2] === 1 || g[2] === 2)) return bad();
      traitGains.push({ hour: g[0], token: g[1], date: g[2] === 0 ? 'heldBefore' : g[2] === 1 ? 'gained' : 'firstObserved' });
    }
    if (!ordered(runs)
      || !l.commands.every(c => span(c) && handle(c.formation) && (c.kind === 'army' || c.kind === 'armyGroup' || c.kind === 'fleet') && text(c.name)
        && (c.nameReadHour === null || integer(c.nameReadHour) && c.firstHour <= c.nameReadHour && c.nameReadHour <= c.lastHour) && ordered(c.nameUnknown))) return bad();
    let hour = 0;
    for (const [i, delta] of battleStarts.entries()) {
      if (typeof delta !== 'number' || !Number.isSafeInteger(delta) || i > 0 && delta < 0 || !integer(hour + delta)) return bad();
      hour += delta; const read = startReads[i];
      if (!integer(read) || read < hour || read < firstHour || hour < firstHour && !unknownStarts.includes(i)) return bad();
    }
    const battleCount = battleStarts.length;
    if (!unknownStarts.every((n, i, all) => u32(n) && n < battleCount && (!i || all[i - 1] < n))) return bad();
    const won = l.battleWon; const resultHours = l.battleResultHours; const resultReads = l.battleResultReadHours;
    if ((won === undefined) !== (resultHours === undefined) || (won === undefined) !== (resultReads === undefined)) return bad();
    if (won !== undefined && (!Array.isArray(won) || !Array.isArray(resultHours) || !Array.isArray(resultReads) || resultReads.length !== won.length || won.length !== battleCount || resultHours.length !== won.length
      || !won.every((w, i) => (w === null || typeof w === 'boolean') && (resultHours[i] === null || integer(resultHours[i])) && (w === null) === (resultHours[i] === null) && (w === null) === (resultReads[i] === null) && (resultReads[i] === null || integer(resultReads[i]) && resultReads[i] >= resultHours[i] && resultReads[i] >= firstHour)))) return bad();
    hour = 0;
    if (Array.isArray(resultHours) && resultHours.some((result, i) => {
      hour += battleStarts[i]; return result !== null && result < hour;
    })) return bad();
    // Every decided result needs an earlier or same-hour participation of its own.
    const foughtAt = l.foughtHours === undefined ? [] : visibleHours(l.foughtHours);
    const wonAt = l.wonHours === undefined ? [] : visibleHours(l.wonHours);
    const lostAt = l.lostHours === undefined ? [] : visibleHours(l.lostHours);
    if (!foughtAt || !wonAt || !lostAt || (l.wonHours === undefined) !== (l.lostHours === undefined)
      || (l.foughtHours === undefined ? l.wonHours !== undefined : l.battleStarts !== undefined)
      || [...wonAt, ...lostAt].sort(ascending).some((at, i) => !(foughtAt[i] <= at))) return bad();
    leaders.push({ ...l, runs, traitGains } as unknown as LeaderCareer);
  }
  if (new Set(input.traits.map(t => (t as Record<string, unknown>).token)).size !== input.traits.length) return bad();
  const reply = { ...input, leaders } as unknown as UnitLeadersData;
  for (const leader of reply.leaders) battleHours(leader);
  return reply;
}

export interface LeaderRow { key: string; hour: number; career: LeaderCareer; value: LeaderValues & LeaderRole; person: LeaderPerson; traitCatalogue: UnitLeadersData['traits']; fought: number | null; preparing: boolean; foughtPartial: boolean; won: number | null; lost: number | null; winRate: number | null; unresolved: number | null }

/** Current battle-table coverage, shared by every row of the same kind/hour. */
export function battleHistoryComplete(reply: UnitLeadersData, kind: number, hour: number): boolean {
  const read = runAt(reply.observed, hour);
  return !!read && (kind === 1 ? read.landBattles : read.navalBattles);
}
const startsCache = new WeakMap<LeaderCareer, { encoded: LeaderCareer['battleStarts']; starts: readonly number[] }>();
export function battleStarts(leader: LeaderCareer): readonly number[] {
  let cached = startsCache.get(leader);
  if (!cached || cached.encoded !== leader.battleStarts) {
    cached = { encoded: leader.battleStarts, starts: decode(leader.battleStarts ?? []) };
    startsCache.set(leader, cached);
  }
  return cached.starts;
}
const BATTLE_FIELDS = ['foughtHours', 'wonHours', 'lostHours', 'battleStarts', 'battleStartReadHours', 'battleWon', 'battleResultHours', 'battleResultReadHours'] as const;
interface BattleHours { from: LeaderCareer; fought: readonly number[]; won: readonly number[] | null; lost: readonly number[] | null }
const hoursCache = new WeakMap<LeaderCareer, BattleHours>();
/** Sorted hours each participation and known win or loss became visible: leaderboards list them; careers derive them from their records. */
function battleHours(leader: LeaderCareer): BattleHours {
  const cached = hoursCache.get(leader);
  if (cached && BATTLE_FIELDS.every(field => cached.from[field] === leader[field])) return cached;
  let hours: BattleHours;
  if (leader.foughtHours) {
    hours = { from: { ...leader }, fought: decode(leader.foughtHours), won: leader.wonHours ? decode(leader.wonHours) : null, lost: leader.lostHours ? decode(leader.lostHours) : null };
  } else {
    // A participation shows from its date and first read; a result also waits for its own date and read.
    const reads = leader.battleStartReadHours ?? []; const outcomes = leader.battleWon; const resultHours = leader.battleResultHours; const resultReads = leader.battleResultReadHours;
    const shown = battleStarts(leader).map((hour, i) => Math.max(hour, reads[i]));
    const results = (outcome: boolean) => outcomes && resultHours && resultReads
      ? shown.flatMap((hour, i) => outcomes[i] === outcome ? [Math.max(hour, resultHours[i]!, resultReads[i]!)] : []).sort(ascending) : null;
    const won = results(true); const lost = results(false);
    hours = { from: { ...leader }, fought: shown.sort(ascending), won, lost };
  }
  hoursCache.set(leader, hours);
  return hours;
}
/** How many sorted hours are at or before `hour`. */
const through = (hours: readonly number[], hour: number) => {
  let lo = 0; let hi = hours.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (hours[mid] <= hour) lo = mid + 1; else hi = mid; }
  return lo;
};
export function battleTotals(leader: LeaderCareer, hour: number) {
  const hours = battleHours(leader); const fought = through(hours.fought, hour);
  if (!hours.won || !hours.lost) return { fought, won: null, lost: null, winRate: null, unresolved: fought };
  const won = through(hours.won, hour); const lost = through(hours.lost, hour);
  const decided = won + lost;
  return { fought, won, lost, winRate: decided ? won * 100 / decided : null, unresolved: fought - decided };
}
export function leaderAt(reply: UnitLeadersData, leader: LeaderCareer, hour: number, coverage?: ReadonlyMap<number, boolean>): LeaderRow | null {
  const run = runAt(leader.runs, hour); if (!run) return null;
  const role = reply.roles[run.value.role]; const totals = battleTotals(leader, hour);
  const preparing = reply.preparing?.includes('battles') === true;
  const foughtPartial = leader.partialFrom != null && leader.partialFrom <= hour || !(coverage?.get(role.kind) ?? battleHistoryComplete(reply, role.kind, hour));
  return { key: leader.reference, hour, career: leader, value: { ...run.value, ...role }, person: reply.people[run.value.person], traitCatalogue: reply.traits, ...totals,
    foughtPartial, preparing, ...(preparing ? { fought: null, won: null, lost: null, unresolved: null, winRate: null } : { fought: totals.fought }) };
}
export function leadersAt(reply: UnitLeadersData, hour: number, countries: ReadonlySet<string> | null, kind: string): LeaderRow[] {
  const coverage = new Map([1, 2].map(k => [k, battleHistoryComplete(reply, k, hour)]));
  return reply.leaders.flatMap(l => { const row = leaderAt(reply, l, hour, coverage); return row && (!countries || row.value.owner != null && countries.has(row.value.owner))
    && (kind === 'all' || row.value.kind === (kind === 'navy' ? 2 : 1)) ? [row] : []; });
}

/** Owner runs start at a transfer or a visibility gap; skill changes keep that read. */
export function ownerReadHour(reply: UnitLeadersData, leader: LeaderCareer, hour: number): number | null {
  let first: number | null = null; let previous: LeaderRun | undefined;
  for (const run of leader.runs) {
    if (run.firstHour > hour) break;
    if (!previous || previous.lastHour + 1 !== run.firstHour || reply.roles[previous.value.role].owner !== reply.roles[run.value.role].owner) first = run.firstHour;
    previous = run;
  }
  return previous && previous.lastHour >= hour ? first : null;
}

/** Warm owner changes, then read header and hover identities at their exact hours. */
export function careerCountryFacts(reply: UnitLeadersData, leader: LeaderCareer, hour: number, hoveredHour: number | null): NameAt[] {
  const wanted = new Map<string, NameAt>();
  const add = (hour: number, owner: string | null) => { if (owner) wanted.set(nameKey(hour, owner), { hour, tag: owner }); };
  let previous: LeaderRun | undefined;
  for (const run of leader.runs) {
    const owner = reply.roles[run.value.role].owner;
    if (!previous || previous.lastHour + 1 !== run.firstHour || reply.roles[previous.value.role].owner !== owner) add(run.firstHour, owner);
    previous = run;
  }
  for (const at of hoveredHour == null ? [hour] : [hour, hoveredHour]) {
    const run = runAt(leader.runs, at);
    if (run) add(at, reply.roles[run.value.role].owner);
  }
  return [...wanted.values()];
}
export const leaderName = (row: Pick<LeaderRow, 'person'>) => row.person.name ?? 'Character name unavailable';
export const attributeNames = (role: Pick<LeaderRole, 'attributeNames'>) => role.attributeNames.map(name => name ?? 'Name not recorded');
export const recordedSince = (leader: LeaderCareer, recordingStart: number) => `Battles since ${formatDateTime(leader.runs[0]?.firstHour ?? recordingStart)}`;

export function careerEmptyEntry(reply: UnitLeadersData, leader: LeaderCareer, laneId: string, hour: number): string {
  const run = runAt(leader.runs, hour);
  if (!run) return 'Leader unavailable.';
  const kind = reply.roles[run.value.role].kind;
  const observed = runAt(reply.observed, hour);
  if (laneId === 'commands') return observed && (kind === 1 ? observed.landCommands : observed.navalCommands)
    ? 'No command.' : 'Command data unavailable.';
  if (laneId === 'battles') return battleHistoryComplete(reply, kind, hour)
    ? 'No recorded participation at this hour' : 'Battle data unavailable.';
  return 'No recorded entry at this hour';
}

export function leaderTooltip(row: LeaderRow, country: { name: string; flag?: string }): TooltipContent {
  const dates = new Map(row.career.traitGains.filter(g => g.hour <= row.hour).map(g => [g.token, g]));
  const names = attributeNames(row.value);
  return { title: leaderName(row), subtitle: `${country.name} · ${row.value.rank || `Rank ${row.value.category}`} · ${formatDateTime(row.hour)}`,
    rows: [{ label: 'Skill', value: row.value.skill == null ? 'Not recorded' : `${row.value.skill}${row.value.maximumSkill == null ? '' : ` / ${row.value.maximumSkill}`}` },
      { label: `${names[0]} · ${names[1]}`, value: `${row.value.components[0]} · ${row.value.components[1]}` },
      { label: `${names[2]} · ${names[3]}`, value: `${row.value.components[2]} · ${row.value.components[3]}` },
      { label: 'Battles won / fought', value: row.preparing ? 'Still being prepared' : `${row.won ?? 'Result not recorded'} / ${row.fought}`, divider: true },
      { label: 'Battles lost', value: row.preparing ? 'Still being prepared' : row.lost == null ? 'Result not recorded' : String(row.lost) },
      { label: 'Unresolved battles', value: row.preparing ? 'Still being prepared' : String(row.unresolved) },
      { label: 'Win rate', value: row.preparing ? 'Still being prepared' : row.winRate == null ? row.won == null ? 'Result not recorded' : 'No battles with known results.' : `${Math.round(row.winRate)}%` },
      ...(row.value.traits?.length === 0 ? [{ label: 'Traits', value: 'None recorded', divider: true }] : row.value.traits?.map((token, i) => {
        const gain = dates.get(token); const trait = row.traitCatalogue[token];
        return { label: contentTitle(trait.name, trait.token, 'Trait name unavailable'), icon: trait.icon ?? undefined, divider: i === 0,
          value: row.preparing ? 'Still being prepared' : gain ? `${gain.date === 'heldBefore' ? 'Held before' : gain.date === 'gained' ? 'Gained' : 'Held by'} ${formatDateTime(gain.hour)}` : 'Date not recorded' };
      }) ?? [{ label: 'Traits not recorded', divider: true }])],
  };
}

export function careerLanes(reply: UnitLeadersData, leader: LeaderCareer, firstHour: number, lastHour: number, readHour = lastHour): TimelineLane[] {
  const traits = new Map(reply.traits.map((t, i) => [i, t]));
  const skill: TimelineSpan[] = leader.runs.flatMap((run, i) => {
    if (run.value.skill == null || run.firstHour > lastHour || run.lastHour < firstHour) return [];
    const previous = leader.runs[i - 1];
    const consecutive = previous && previous.lastHour + 1 === run.firstHour && previous.value.skill != null;
    const gain = consecutive && run.value.skill > previous.value.skill!;
    const date = run.firstHour === reply.firstHour ? 'Held before' : gain ? 'Level up' : consecutive ? null : 'First seen';
    return [{ id: `skill:${run.firstHour}`, firstHour: Math.max(firstHour, run.firstHour), lastHour: Math.min(lastHour, run.lastHour), value: run.value.skill, dot: !!gain && run.firstHour >= firstHour,
      label: `Skill ${run.value.skill}${date ? ` · ${date} ${formatDateTime(run.firstHour)}` : ''}` }];
  });
  const held = runAt(leader.runs, firstHour)?.value.traits ?? [];
  const earlier = leader.traitGains.filter(g => g.hour < firstHour && held.includes(g.token)).filter((g, i, all) => !all.slice(i + 1).some(other => other.token === g.token));
  const dates = new Map<number, TimelineSpan>();
  for (const g of [...earlier.map(g => ({ ...g, hour: firstHour, date: 'heldBefore' as const })), ...leader.traitGains.filter(g => g.hour >= firstHour && g.hour <= lastHour)]) {
    const label = `${contentTitle(traits.get(g.token)?.name, g.token, 'Trait name unavailable')} · ${g.date === 'heldBefore' ? 'Held before' : g.date === 'gained' ? 'Gained' : 'First seen'} ${formatDateTime(g.hour)}`;
    const previous = dates.get(g.hour);
    dates.set(g.hour, { id: `trait:${g.hour}`, firstHour: g.hour, lastHour: g.hour, marker: true,
      label: previous ? `${previous.label}; ${label}` : label, icons: [...(previous?.icons ?? []), traits.get(g.token)?.icon ?? undefined] });
  }
  const gains = [...dates.values()];
  const commands = stackedSpans(leader.commands.filter(c => c.firstHour <= lastHour && c.lastHour >= firstHour).map(c => ({ id: `command:${c.formation}:${c.firstHour}`, firstHour: Math.max(firstHour, c.firstHour), lastHour: Math.min(lastHour, c.lastHour), label: c.nameReadHour != null && c.nameReadHour <= lastHour && c.name || `${c.kind === 'fleet' ? 'Fleet' : 'Formation'} ${c.formation}` })));
  // Separate neutral fought counts from the result series, so joined outcomes can
  // replace them with won above and lost below the shared monthly axis.
  const months = new Map<number, { first: number; last: number; count: number; unknown: number; won: number; lost: number }>();
  const unknown = new Set(leader.unknownBattleStarts); const startReads = leader.battleStartReadHours ?? [];
  for (const [i, first] of battleStarts(leader).entries()) {
    if (first < firstHour || first > lastHour || startReads[i] > readHour) continue;
    const date = gameDate(first); const index = date ? monthIndex(date) : first;
    const start = date ? firstHourOfMonth(index) : first; const end = date ? firstHourOfMonth(index + 1) - 1 : first;
    const month = months.get(index) ?? { first: Math.max(firstHour, start), last: Math.min(lastHour, end), count: 0, unknown: 0, won: 0, lost: 0 };
    if (unknown.has(i)) month.unknown++;
    const won = leader.battleWon?.[i]; const resultHour = leader.battleResultHours?.[i]; const resultRead = leader.battleResultReadHours?.[i];
    if (won != null && resultHour != null && resultHour <= lastHour && resultRead != null && resultRead <= readHour) { if (won) month.won++; else month.lost++; }
    month.count++; months.set(index, month);
  }
  const kind = reply.roles[leader.runs[0]?.value.role]?.kind;
  const battles = [...months].flatMap(([key, m]) => reply.observed.filter(o => (kind === 1 ? o.landBattles : o.navalBattles) && o.firstHour <= m.last && o.lastHour >= m.first).map(o => ({
    id: `battle:${key}:${o.firstHour}`, firstHour: Math.max(o.firstHour, m.first), lastHour: Math.min(o.lastHour, m.last), value: m.count,
    positiveCount: leader.battleWon ? m.won : undefined, negativeCount: leader.battleWon ? m.lost : undefined,
    label: `Fought ${m.count} this month${m.unknown ? ` · ${m.unknown} unknown combat start` : ''}${leader.battleWon ? ` · ${m.won} won · ${m.lost} lost · ${m.count - m.won - m.lost} unresolved` : ' · Battle results are not in this recording'}` })));
  const visibleBattles = clipToSpans(battles, leader.runs).reduce<TimelineSpan[]>((spans, span) => {
    const last = spans.at(-1);
    if (last && last.lastHour + 1 === span.firstHour && last.label === span.label) last.lastHour = span.lastHour;
    else spans.push({ ...span, id: `${span.id}:${span.firstHour}` });
    return spans;
  }, []);
  return [
    { id: 'skill', label: 'Skill', compact: true, stepMax: leader.runs.filter(r => r.firstHour <= lastHour && r.lastHour >= firstHour).reduce((max, r) => Math.max(max, reply.roles[r.value.role].maximumSkill ?? r.value.skill ?? 1), 1), spans: skill },
    { id: 'traits', label: 'Traits', compact: true, spans: gains },
    { id: 'commands', label: 'Commands', compact: true, spans: commands },
    { id: 'battles', label: 'Battles', compact: true, barMax: [...months.values()].reduce((max, month) => Math.max(max, month.count), 1), spans: visibleBattles },
  ];
}

export function pinnedCareer(source: WidgetSpec, reference: string, track: Pick<BoundTrack, 'id' | 'branch'>,
  hour: number, follow: boolean): WidgetSpec {
  const base = createWidget('general-career', track.id);
  const time: WidgetSpec['time'] = follow ? { mode: 'follow' } : { mode: 'pin', windows: {
    [track.id]: { firstOffset: 0, lastOffset: Math.floor(hour) - track.branch.firstHour },
  } };
  const options = resolveOptions([GROW_WITH_PLAYBACK_OPTION], source.options);
  return { ...base, campaign: source.campaign, trackIds: [...source.trackIds], countries: source.countries, time,
    contentScale: source.contentScale, options: { leader: reference, growWithPlayback: options.growWithPlayback } };
}

export function leaderRequestSelection(tags: readonly string[] | null | undefined): LeaderSelection | undefined {
  return tags === undefined ? undefined : tags === null ? { kind: 'all' } : { kind: 'countries', tags: [...tags] };
}
