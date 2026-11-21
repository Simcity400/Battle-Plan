import type { CountryGroupReply } from '../generated/dashboard';
import type { NameAt } from './countryFacts';
import type { Rgb } from '../shell/appearance/color';

export interface FactionFounder { branchId: string; hour: number; tag: string }

export function factionFounders(replies: readonly CountryGroupReply[]): Map<string, FactionFounder> {
  const founders = new Map<string, FactionFounder>();
  for (const reply of replies) {
    for (const faction of reply.factions) {
      const first = faction.leaders.reduce<typeof faction.leaders[number] | null>((best, run) => !best || run.firstHour < best.firstHour ? run : best, null);
      const known = founders.get(faction.id);
      if (first && (!known || first.firstHour < known.hour)) founders.set(faction.id, { branchId: reply.branchId, hour: first.firstHour, tag: first.tag });
    }
  }
  return founders;
}

export function founderReads(founders: ReadonlyMap<string, FactionFounder>): Map<string, NameAt[]> {
  const reads = new Map<string, NameAt[]>();
  for (const founder of founders.values()) {
    const list = reads.get(founder.branchId);
    const item = { hour: founder.hour, tag: founder.tag };
    if (list) list.push(item); else reads.set(founder.branchId, [item]);
  }
  return reads;
}

export function leaderColours(founders: ReadonlyMap<string, FactionFounder>,
  styleAt: (founder: FactionFounder) => { color: [number, number, number] | null } | null | undefined): Map<string, Rgb | null> {
  return new Map([...founders].map(([id, founder]) => {
    const colour = styleAt(founder)?.color;
    return [id, colour ? { r: colour[0], g: colour[1], b: colour[2] } : null];
  }));
}
