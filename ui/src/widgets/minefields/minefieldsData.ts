import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedRun, KeyedWindow } from '../../dashboard/keyedHistory';

export interface Choice { id: string; name: string }
export interface MinefieldsReply { window: KeyedWindow; regions: readonly Choice[]; scale: number; warnings: readonly string[] }
export interface MineReport { kind: number; ordinal: number; victim: string | null; mineOwner: string | null; ship: string | null; equipment: string | null; damage: string; sunk: boolean; firstSeen: number | null }
export interface ReportsReply { hour: number; readHour: number | null; region: number; reports: readonly MineReport[]; warnings: readonly string[] }

const isChoice = (value: unknown): value is Choice => !!value && typeof value === 'object'
  && typeof (value as Choice).id === 'string' && typeof (value as Choice).name === 'string';
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const optional = (value: unknown) => value === null || typeof value === 'string';

export function validateMinefields(value: unknown): MinefieldsReply {
  const reply = value as MinefieldsReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.regions) || !reply.regions.every(isChoice)
    || !Number.isSafeInteger(reply.scale) || reply.scale <= 0 || !strings(reply.warnings))
    throw new Error('The minefields reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

export function validateReports(value: unknown): ReportsReply {
  const reply = value as ReportsReply;
  if (!reply || typeof reply !== 'object' || !Number.isSafeInteger(reply.hour) || !(reply.readHour === null || Number.isSafeInteger(reply.readHour))
    || !Number.isSafeInteger(reply.region) || !strings(reply.warnings) || !Array.isArray(reply.reports)
    || !reply.reports.every(report => report && Number.isSafeInteger(report.kind) && Number.isSafeInteger(report.ordinal) && optional(report.victim)
      && optional(report.mineOwner) && optional(report.ship) && optional(report.equipment) && typeof report.damage === 'string'
      && typeof report.sunk === 'boolean' && (report.firstSeen === null || Number.isSafeInteger(report.firstSeen))))
    throw new Error('The mine reports reply is malformed.');
  return reply;
}

/** Each owner's runs: its regional amounts, its witnessed extent and the hours its vector was
 * read but refused. */
export interface OwnerSeries { owner: string; tag: string; regions: Map<string, readonly KeyedRun[]>; extent: readonly KeyedRun[]; refused: readonly KeyedRun[] }

/** The owner part of the count of owners a read holds vectors for but cannot name. */
export const UNNAMED_OWNERS = '?';

/** Named owners' series; the unnamed owners' count is read separately with `unnamedRuns`. */
export function ownerSeries(window: KeyedWindow): Map<string, OwnerSeries> {
  const out = new Map<string, OwnerSeries>();
  for (const series of window.series) {
    const [owner, field] = series.name.split('|');
    if (!field || owner === UNNAMED_OWNERS) continue;
    const entry = out.get(owner) ?? { owner, tag: countryPart(owner)?.tag ?? owner, regions: new Map(), extent: [], refused: [] };
    if (field === 'extent') entry.extent = series.runs;
    else if (field === 'refused') entry.refused = series.runs;
    else entry.regions.set(field, series.runs);
    out.set(owner, entry);
  }
  return out;
}

/** How many owners' vectors a read holds but could not admit (named or not) at `hour`: while
 * any are, the inventory is incomplete and an empty table proves nothing. */
export function refusedOwnersAt(window: KeyedWindow, owners: ReadonlyMap<string, OwnerSeries>, hour: number): number {
  let count = 0;
  for (const series of owners.values()) if (runAtHour(series.refused, hour)) count++;
  const unnamed = window.series.find(series => series.name === `${UNNAMED_OWNERS}|refused`);
  const run = unnamed && runAtHour(unnamed.runs, hour);
  return count + (run ? Number(run[2]) : 0);
}

/** One owner's mines in a region at an hour: its amount, a proven zero below its witnessed
 * extent, or null (unknown) beyond it or without a witness. */
export function ownerAmount(series: OwnerSeries, region: string, hour: number): bigint | null {
  const run = runAtHour(series.regions.get(region) ?? [], hour);
  if (run) return BigInt(run[2]);
  const extent = runAtHour(series.extent, hour);
  return extent && Number(region) < Number(extent[2]) ? 0n : null;
}

export interface RegionMines { region: string; name: string; owners: { owner: string; tag: string; amount: bigint }[]; total: bigint; unknownOwners: number }

/** Sea regions holding mines at `hour`, each with its owners' amounts (largest first) and how
 * many owners' amounts are unknown there (refused owners included, named or not), and how many
 * owners' vectors were refused; null when not recorded then. */
export function minesAt(reply: MinefieldsReply, owners: ReadonlyMap<string, OwnerSeries>, hour: number): { regions: RegionMines[]; readHour: number | undefined; refused: number } | null {
  if (!keyedKnown(reply.window, hour)) return null;
  const refused = refusedOwnersAt(reply.window, owners, hour);
  const unnamed = refused - [...owners.values()].filter(series => runAtHour(series.refused, hour)).length;
  const names = new Map(reply.regions.map(region => [region.id, region.name]));
  const mined = new Set<string>();
  for (const series of owners.values()) for (const [region, runs] of series.regions) { const run = runAtHour(runs, hour); if (run && run[2] !== '0') mined.add(region); }
  const regions = [...mined].map(region => {
    const list: RegionMines['owners'] = [];
    let unknownOwners = unnamed;
    for (const series of owners.values()) {
      const amount = ownerAmount(series, region, hour);
      if (amount === null) unknownOwners++;
      else if (amount !== 0n) list.push({ owner: series.owner, tag: series.tag, amount });
    }
    list.sort((a, b) => a.amount > b.amount ? -1 : a.amount < b.amount ? 1 : a.owner.localeCompare(b.owner));
    return { region, name: names.get(region) ?? 'Region name unavailable', owners: list, total: list.reduce((sum, item) => sum + (item.amount > 0n ? item.amount : 0n), 0n), unknownOwners };
  });
  return { regions, readHour: keyedRead(reply.window, hour), refused };
}

export function orderRegions(regions: readonly RegionMines[], order: 'mines' | 'name'): RegionMines[] {
  return [...regions].sort((a, b) => order === 'name' ? a.name.localeCompare(b.name) || a.region.localeCompare(b.region)
    : a.total > b.total ? -1 : a.total < b.total ? 1 : a.name.localeCompare(b.name));
}

/** Mines on the reply's scale, exactly. */
export function minesText(value: bigint, scale: number): string {
  const negative = value < 0n, magnitude = negative ? -value : value, unit = BigInt(scale);
  const fraction = (magnitude % unit).toString().padStart(String(scale).length - 1, '0').replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / unit).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}
