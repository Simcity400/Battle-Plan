import type { CountryRaceEntry, LiveRaceEntries } from '../../charts/race/countryRace';

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
