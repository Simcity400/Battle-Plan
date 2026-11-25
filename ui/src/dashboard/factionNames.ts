import type { CountryGroupReply, FactionGroup } from '../generated/dashboard';
import { runAt } from '../charts/series.ts';

export const UNNAMED_FACTION = 'Unnamed faction';
export interface FactionName { name: string | null; reason: string | null }

export function factionNameAt(faction: Pick<FactionGroup, 'name' | 'nameReason' | 'names'>, hour?: number, override?: string): FactionName {
  if (override) return { name: override, reason: null };
  if (hour == null) return { name: faction.name, reason: faction.nameReason };
  if (!Number.isFinite(hour)) return { name: null, reason: 'Not recorded' };
  const run = runAt(faction.names, Math.floor(hour));
  return run ? { name: run.name, reason: run.reason } : { name: null, reason: 'Not recorded' };
}

export function factionChoiceLabel(faction: Pick<FactionGroup, 'name' | 'nameReason' | 'names'>, hour?: number, override?: string): string {
  return factionNameAt(faction, hour, override).name ?? faction.name ?? UNNAMED_FACTION;
}

export function latestFactionSummaries(replies: readonly CountryGroupReply[]): FactionGroup[] {
  const byId = new Map<string, FactionGroup>();
  const latestRead = (value: FactionGroup) => value.names.at(-1)?.lastHour ?? value.lastHour;
  for (const reply of replies) for (const faction of reply.factions) {
    const known = byId.get(faction.id);
    const latest = known && latestRead(known) >= latestRead(faction) ? known : faction;
    byId.set(faction.id, { ...latest, recordedName: known?.recordedName || faction.recordedName });
  }
  return [...byId.values()];
}
