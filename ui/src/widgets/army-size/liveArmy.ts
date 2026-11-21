import type { CountryRaceEntry, LiveRaceEntries } from '../../charts/race/countryRace';
import { countryValueEntries, rankCountryValues } from '../../charts/race/countryValues.ts';
import type { CountryValuesModel } from '../../charts/race/countryValues';
import { isMember, rankedCandidates } from '../../dashboard/groups.ts';
import type { ResolvedCountries } from '../../dashboard/groups';
import { CELL, cellNumber, cellState, cellValue, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { FollowEntry } from '../../dashboard/follow/store';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { ArmyMeasure } from '../../generated/dashboard';
import { formatArmyRaw, formatArmyRawFast, formatArmyValue } from './armySizeData.ts';

export interface RaceClock {
  getSnapshot(): { position: number };
  subscribeFrame(listener: () => void): () => void;
  subscribe(listener: () => void): () => void;
}

export interface LiveArmy extends LiveRaceEntries {
  structure(): string;
  subscribe(changed: () => void): () => void;
  wake(): void;
}

export const rowsOf = (entries: readonly CountryRaceEntry[]) => entries.map(entry => entry.tag).sort().join(',');

export function followedEntries(answer: FollowedAnswer | undefined, asked: readonly string[] | null, resolved: Pick<ResolvedCountries, 'membership'>,
  hour: number, slots: number, measure: ArmyMeasure, scale: number): CountryRaceEntry[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const rows: { tag: string; raw: number | bigint }[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const at = row * answer.columns;
    if (cellState(answer, at) !== CELL.value) continue;
    const tag = rowName(answer, row) ?? asked?.[row];
    if (!tag || (resolved.membership && !isMember(resolved.membership.get(tag), hour))) continue;
    rows.push({ tag, raw: cellNumber(answer, at) ?? cellValue(answer, at) });
  }
  rows.sort((a, b) => (a.raw < b.raw ? 1 : a.raw > b.raw ? -1 : 0) || compareTags(a.tag, b.tag));
  const divisor = 10 ** scale;
  return rows.slice(0, Math.max(0, slots)).map(({ tag, raw }) => ({ tag, value: Number(raw) / divisor, label: formatArmyRawFast(raw, measure), stretch: null, hour }));
}

export function replyEntries(model: CountryValuesModel, resolved: ResolvedCountries, chosen: boolean, hour: number, slots: number,
  byType: boolean, measure: ArmyMeasure): CountryRaceEntry[] {
  const ranked = rankCountryValues(model, chosen ? model.tags : rankedCandidates(resolved, hour, model.tags), hour, slots);
  return countryValueEntries(model, ranked, byType, formatArmyValue).map((entry, index) => ({ ...entry, hour,
    label: ranked[index].at.rawTotal === undefined ? entry.label : formatArmyRaw(ranked[index].at.rawTotal!, measure) }));
}

export function liveArmy(clock: RaceClock, axisOffset: number, read: (hour: number) => readonly CountryRaceEntry[],
  version: () => number, key?: (hour: number) => string, byHour = true): LiveArmy {
  const wakes = new Set<() => void>();
  let made: { hour: number; version: number; entries: readonly CountryRaceEntry[] } | null = null;
  const hour = () => Math.floor(clock.getSnapshot().position) + axisOffset;
  const entries = () => {
    const at = hour(), now = version();
    if (!made || (byHour && made.hour !== at) || made.version !== now) {
      const next = read(at);
      const last = made?.entries ?? [];
      let before: Map<string, CountryRaceEntry> | null = null;
      let same = made !== null && last.length === next.length;
      const kept = next.map((entry, index) => {
        const held = last[index]?.tag === entry.tag ? last[index] : (before ??= new Map(last.map(entry => [entry.tag, entry]))).get(entry.tag);
        const keep = held !== undefined && held.label === entry.label && held.value === entry.value && held.stretch === entry.stretch && !entry.parts && !held.parts;
        same &&= keep && made!.entries[index] === held;
        return keep ? held : entry;
      });
      made = { hour: at, version: now, entries: same ? made!.entries : kept };
    }
    return made.entries;
  };
  return {
    entries, hour,
    structure: () => key ? `${key(hour())}|${rowsOf(entries())}` : rowsOf(entries()),
    bind: wake => { wakes.add(wake); return () => { wakes.delete(wake); }; },
    subscribe: changed => {
      const stops = [clock.subscribeFrame(changed), clock.subscribe(changed)];
      wakes.add(changed);
      return () => { for (const stop of stops) stop(); wakes.delete(changed); };
    },
    wake: () => { for (const wake of [...wakes]) wake(); },
  };
}

export const followedArmy = (clock: RaceClock, axisOffset: number, entry: FollowEntry, asked: readonly string[] | null,
  resolved: Pick<ResolvedCountries, 'membership'>, slots: number, measure: ArmyMeasure, scale: number) =>
  liveArmy(clock, axisOffset, hour => followedEntries(entry.answer, asked, resolved, hour, slots, measure, scale), () => entry.version, undefined, resolved.membership != null);
