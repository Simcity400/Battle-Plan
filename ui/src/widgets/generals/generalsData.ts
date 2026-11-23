import type { GatedTime, PinnedSpec } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { clipToSpans } from '../../charts/series.ts';
import { sortRows } from '../../charts/table/logic.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { stackedSpans } from '../../charts/timeline/layout.ts';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import { firstHourOfMonth, formatDateTime, gameDate, monthIndex } from '../../dashboard/calendar.ts';
import { createWidget } from '../../dashboard/model.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options.ts';
import { subjectToken } from '../../dashboard/chosen.ts';
import { GROW_WITH_PLAYBACK_OPTION } from '../general-career/definition.ts';
import { DEFAULT_SORT } from './definition.ts';
import type { TraitDate } from './types';
import type { LeaderRow as WireRow, CareerBattle, LeaderCareerReply, LeaderHourReply, LeaderKinds, LeaderRole, LeaderSelection, LeaderTrait, LeaderWithheld, UnitLeadersReply } from '../../generated/dashboard';

const KIND_ARMY = 1;
const KIND_NAVY = 2;
const DATES = ['Held before', 'Gained', 'Held by'] as const;

function shaped(input: unknown, keys: readonly string[], what: string): void {
  if (typeof input !== 'object' || input === null || !keys.every(key => key in input)) throw new Error(`The ${what} reply is malformed.`);
}
export function validateLeaders(input: unknown): UnitLeadersReply {
  shaped(input, ['leaders', 'traits', 'withheld', 'missing', 'battlesPartialFrom', 'warnings'], 'unit leaders');
  return input as UnitLeadersReply;
}
export function validateCareer(input: unknown): LeaderCareerReply {
  shaped(input, ['reference', 'runs', 'traitGains', 'traits', 'commands', 'battles', 'observed', 'warnings'], 'leader career');
  return input as LeaderCareerReply;
}
export function validateLeaderHour(input: unknown): LeaderHourReply {
  shaped(input, ['reference', 'missing', 'traitCatalogue', 'fought', 'progress', 'warnings'], 'leader');
  return input as LeaderHourReply;
}

export interface LeaderRow {
  key: string; hour: number; row: WireRow; name: string; role: LeaderRole; traits: readonly LeaderTrait[];
  fought: number; won: number | null; lost: number | null; unresolved: number; winRate: number | null; foughtPartial: boolean;
}

export function battleTotals(fought: number, won: number, lost: number, partial: boolean) {
  const decided = won + lost;
  return { fought, won, lost, unresolved: Math.max(0, fought - decided), winRate: decided ? won * 100 / decided : null, foughtPartial: partial };
}

export const leaderName = (row: { name: string | null }) => row.name ?? 'Character name unavailable';
export const attributeNames = (role: Pick<LeaderRole, 'attributeNames'>) => role.attributeNames.map(name => name ?? 'Name not recorded');

export function leaderRows(reply: UnitLeadersReply, hour: number, countries: ReadonlySet<string> | null, kind: LeaderKinds): LeaderRow[] {
  const partial = reply.battlesPartialFrom !== null && reply.battlesPartialFrom <= hour;
  return reply.leaders.flatMap(row => {
    if (kind !== 'all' && row.role.kind !== (kind === 'navy' ? KIND_NAVY : KIND_ARMY)) return [];
    if (countries && !(row.role.owner !== null && countries.has(row.role.owner))) return [];
    return [{ key: row.reference, hour, row, name: leaderName(row), role: row.role, traits: (row.traits ?? []).map(at => reply.traits[at]),
      ...battleTotals(row.fought, row.won, row.lost, partial) }];
  });
}

export function withheldLeaders(reply: UnitLeadersReply, countries: ReadonlySet<string> | null, kind: LeaderKinds): LeaderWithheld[] {
  return reply.withheld.filter(leader => (kind === 'all' || leader.kind === (kind === 'navy' ? KIND_NAVY : KIND_ARMY))
    && (leader.owner === null || countries === null || countries.has(leader.owner)));
}

export function withheldLeaderState(leaders: readonly LeaderWithheld[], country?: string) {
  const count = new Set(leaders.map(leader => leader.reference)).size;
  const owners = country ? `of ${country}’s` : 'of these countries’';
  return {
    message: `${count} ${owners} ${count === 1 ? 'leader' : 'leaders'} can’t be shown: ${[...new Set(leaders.map(leader => leader.reason))].join('; ')}`,
    detail: leaders.map(leader => `${leader.name ?? `Leader ${leader.reference}`}: ${leader.reason}`).join('\n'),
  };
}

export const leaderKey = (row: LeaderRow) => row.key;
export const leaderSkill = (row: LeaderRow) => row.row.skill;
const defaultSortValue = { skill: leaderSkill }[DEFAULT_SORT.column];
export function topLeader(rows: readonly LeaderRow[]): LeaderRow | null {
  const first = sortRows(rows, defaultSortValue, DEFAULT_SORT.direction, leaderKey)[0];
  return first && defaultSortValue(first) != null ? first : null;
}

export const recordedSince = (firstSeen: number | null, recordingStart: number) => `Battles since ${formatDateTime(firstSeen ?? recordingStart)}`;

const atLeast = (partial: boolean, count: number | null) => count == null ? '—' : partial ? `at least ${count}` : String(count);

export function traitDateText(date: TraitDate | null): string {
  return date ? `${DATES[date[1]] ?? 'Held by'} ${formatDateTime(date[0])}` : 'Date not recorded';
}

export function leaderTooltip(row: LeaderRow, country: { name: string; flag?: string; nameFailure?: Error; nameDetail?: string }): TooltipContent {
  const names = attributeNames(row.role);
  const { components } = row.row;
  return { ...countryNameDetails(country), title: row.name, subtitle: `${country.name} · ${row.role.rank ?? `Rank ${row.role.category}`} · ${formatDateTime(row.hour)}`,
    rows: [{ label: 'Skill', value: row.row.skill == null ? 'Not recorded' : `${row.row.skill}${row.role.maximumSkill == null ? '' : ` / ${row.role.maximumSkill}`}` },
      { label: `${names[0]} · ${names[1]}`, value: `${components[0]} · ${components[1]}` },
      { label: `${names[2]} · ${names[3]}`, value: `${components[2]} · ${components[3]}` },
      { label: 'Battles won / fought', value: `${atLeast(row.foughtPartial, row.won)} / ${atLeast(row.foughtPartial, row.fought)}`, divider: true },
      { label: 'Battles lost', value: atLeast(row.foughtPartial, row.lost) },
      { label: 'Unresolved battles', value: atLeast(row.foughtPartial, row.unresolved) },
      ...row.foughtPartial ? [{ label: 'Battle counts are incomplete: some battles were not recorded.', tone: 'muted' as const }] : [],
      { label: 'Win rate', value: row.winRate == null ? 'No battles with known results.' : `${Math.round(row.winRate)}%` },
      ...(row.row.traits === null ? [{ label: 'Traits not recorded', divider: true }] : row.traits.length === 0 ? [{ label: 'Traits', value: 'None recorded', divider: true }]
        : row.traits.map((trait, i) => ({ label: contentTitle(trait.name, trait.token, 'Trait name unavailable'), icon: trait.icon ?? undefined, divider: i === 0,
          value: traitDateText(row.row.traitDates[i] ?? null) })))],
  };
}

export function careerTotals(career: LeaderCareerReply, hour: number) {
  let fought = 0; let won = 0; let lost = 0;
  for (const battle of career.battles) {
    if (Math.max(battle.start, battle.read) > hour) continue;
    fought++;
    if (battle.won !== null && battle.resultRead !== null && battle.resultRead <= hour && (battle.resultHour ?? battle.resultRead) <= hour) {
      if (battle.won) won++; else lost++;
    }
  }
  return battleTotals(fought, won, lost, career.battlesPartialFrom !== null && career.battlesPartialFrom <= hour);
}

export function runAt(career: LeaderCareerReply, hour: number) {
  return career.runs.find(run => run.firstHour <= hour && hour <= run.lastHour);
}

export function careerEmptyEntry(career: LeaderCareerReply, laneId: string, hour: number): string {
  const run = runAt(career, hour);
  if (!run) return 'Leader unavailable.';
  if (laneId === 'commands') return 'No command.';
  if (laneId === 'battles') return career.battlesPartialFrom !== null && career.battlesPartialFrom <= hour ? 'Battle data unavailable.' : 'No recorded participation at this hour';
  return 'No recorded entry at this hour';
}

function battleMonths(battles: readonly CareerBattle[], firstHour: number, lastHour: number, readHour: number) {
  const months = new Map<number, { first: number; last: number; count: number; unknown: number; won: number; lost: number }>();
  for (const battle of battles) {
    if (battle.start > lastHour || battle.read > readHour) continue;
    const date = gameDate(battle.start); const index = date ? monthIndex(date) : battle.start;
    const start = date ? firstHourOfMonth(index) : battle.start; const end = date ? firstHourOfMonth(index + 1) - 1 : battle.start;
    if (end < firstHour) continue;
    const month = months.get(index) ?? { first: Math.max(firstHour, start), last: Math.min(lastHour, end), count: 0, unknown: 0, won: 0, lost: 0 };
    if (battle.unknown) month.unknown++;
    if (battle.won !== null && battle.resultHour !== null && battle.resultHour <= lastHour && battle.resultRead !== null && battle.resultRead <= readHour) {
      if (battle.won) month.won++; else month.lost++;
    }
    month.count++; months.set(index, month);
  }
  return months;
}

export function careerLanes(career: LeaderCareerReply, firstHour: number, lastHour: number, readHour = lastHour): TimelineLane[] {
  const runs = career.runs;
  const skill: TimelineSpan[] = runs.flatMap((run, i) => {
    if (run.skill == null || run.firstHour > lastHour || run.lastHour < firstHour) return [];
    const previous = runs[i - 1];
    const consecutive = previous && previous.lastHour + 1 === run.firstHour && previous.skill != null;
    const gain = consecutive && run.skill > previous.skill!;
    const first = career.observed[0]?.[0];
    const date = run.firstHour === first ? 'Held before' : gain ? 'Level up' : consecutive ? null : 'First seen';
    return [{ id: `skill:${run.firstHour}`, firstHour: Math.max(firstHour, run.firstHour), lastHour: Math.min(lastHour, run.lastHour), value: run.skill,
      dot: !!gain && run.firstHour >= firstHour, label: `Skill ${run.skill}${date ? ` · ${date} ${formatDateTime(run.firstHour)}` : ''}` }];
  });
  const earlier = new Map<number, [number, number, number]>();
  for (const gain of career.traitGains) if (gain[0] < firstHour) earlier.set(gain[1], gain);
  const dates = new Map<number, TimelineSpan>();
  for (const [hour, at, date] of [...[...earlier.values()].map(([, at]) => [firstHour, at, 0] as [number, number, number]),
    ...career.traitGains.filter(([hour]) => hour >= firstHour && hour <= lastHour)]) {
    const trait = career.traits[at];
    const label = `${contentTitle(trait?.name, trait?.token ?? String(at), 'Trait name unavailable')} · ${date === 0 ? 'Held before' : date === 1 ? 'Gained' : 'First seen'} ${formatDateTime(hour)}`;
    const previous = dates.get(hour);
    dates.set(hour, { id: `trait:${hour}`, firstHour: hour, lastHour: hour, marker: true,
      label: previous ? `${previous.label}; ${label}` : label, icons: [...(previous?.icons ?? []), trait?.icon ?? undefined] });
  }
  const commands = stackedSpans(career.commands.filter(c => c.firstHour <= lastHour && c.lastHour >= firstHour).map(c => ({ id: `command:${c.formation}:${c.firstHour}`,
    firstHour: Math.max(firstHour, c.firstHour), lastHour: Math.min(lastHour, c.lastHour), label: c.name ?? `${c.kind === 'fleet' ? 'Fleet' : 'Formation'} ${c.formation}` })));
  const months = battleMonths(career.battles, firstHour, lastHour, readHour);
  const kind = runs[0]?.role.kind;
  const recorded = (from: number, to: number) => career.observed.filter(([a, b]) => a <= to && b >= from && !(kind !== undefined && career.battlesPartialFrom !== null && career.battlesPartialFrom <= a));
  const battles = [...months].flatMap(([key, m]) => recorded(m.first, m.last).map(([a, b]) => ({
    id: `battle:${key}:${a}`, firstHour: Math.max(a, m.first), lastHour: Math.min(b, m.last), value: m.count, positiveCount: m.won, negativeCount: m.lost,
    label: `Fought ${m.count} this month${m.unknown ? ` · ${m.unknown} unknown combat start` : ''} · ${m.won} won · ${m.lost} lost · ${m.count - m.won - m.lost} unresolved` })));
  const visibleBattles = clipToSpans(battles, runs).reduce<TimelineSpan[]>((spans, span) => {
    const last = spans.at(-1);
    if (last && last.lastHour + 1 === span.firstHour && last.label === span.label) last.lastHour = span.lastHour;
    else spans.push({ ...span, id: `${span.id}:${span.firstHour}` });
    return spans;
  }, []);
  return [
    { id: 'skill', label: 'Skill', compact: true, stepMax: runs.filter(r => r.firstHour <= lastHour && r.lastHour >= firstHour).reduce((max, r) => Math.max(max, r.role.maximumSkill ?? r.skill ?? 1), 1), spans: skill },
    { id: 'traits', label: 'Traits', compact: true, spans: [...dates.values()] },
    { id: 'commands', label: 'Commands', compact: true, spans: commands },
    { id: 'battles', label: 'Battles', compact: true, barMax: [...months.values()].reduce((max, month) => Math.max(max, month.count), 1), spans: visibleBattles },
  ];
}

export function pinnedCareer(source: WidgetSpec, reference: string, track: Pick<BoundTrack, 'id' | 'branch' | 'source'>, time: GatedTime): PinnedSpec {
  const base = createWidget('general-career', track.id);
  const options = resolveOptions([GROW_WITH_PLAYBACK_OPTION], source.options);
  return { ...base, campaign: source.campaign, trackIds: [...source.trackIds], countries: source.countries, time,
    contentScale: source.contentScale, options: { leader: subjectToken(track.source.metadata.campaignKey, reference), growWithPlayback: options.growWithPlayback } };
}

export function leaderRequestSelection(tags: readonly string[] | null): LeaderSelection {
  return tags === null ? { kind: 'all' } : { kind: 'countries', countries: tags.map(countryFromKey) };
}

export function leadersStructure(reply: UnitLeadersReply): string {
  return JSON.stringify([reply.leaders, reply.withheld, reply.missing, reply.battlesPartialFrom, reply.traits.length]);
}
