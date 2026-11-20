import type { MineLine, MineReport, MinedRegion, MinefieldHistoryReply, MinefieldReportsReply, MinefieldsReply } from '../../generated/dashboard';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';

const text = (value: unknown): value is string => typeof value === 'string';
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(text);
const optional = (value: unknown) => value === null || text(value);
const decimal = (value: unknown) => text(value) && /^-?\d+$/u.test(value);
const count = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;

export function validateMinefields(value: unknown): MinefieldsReply {
  const reply = value as MinefieldsReply;
  if (!reply || typeof reply !== 'object' || !Number.isSafeInteger(reply.scale) || reply.scale <= 0 || !strings(reply.warnings)
    || !Array.isArray(reply.regions) || !reply.regions.every(region => !!region && count(region.region) && count(region.unknownOwners)
      && Array.isArray(region.owners) && region.owners.every(owner => !!owner && text(owner.tag)))
    || !Array.isArray(reply.unknown) || !reply.unknown.every(region => !!region && count(region.region) && count(region.unknownOwners))
    || !Array.isArray(reply.choices) || !reply.choices.every(choice => !!choice && text(choice.id) && text(choice.name)))
    throw new Error('The minefields reply is malformed.');
  return reply;
}

export function validateHistory(value: unknown): MinefieldHistoryReply {
  const reply = value as MinefieldHistoryReply;
  const run = (item: unknown) => Array.isArray(item) && item.length === 3 && Number.isSafeInteger(item[0]) && Number.isSafeInteger(item[1])
    && (item[0] as number) <= (item[1] as number) && (item[2] === null || decimal(item[2]));
  if (!reply || typeof reply !== 'object' || !count(reply.region) || !Number.isSafeInteger(reply.scale) || reply.scale <= 0 || !strings(reply.warnings)
    || !Array.isArray(reply.lines) || !reply.lines.every((line: MineLine) => !!line && (typeof line.country === 'number' || text(line.country)) && Array.isArray(line.runs) && line.runs.every(run)))
    throw new Error('The minefield history reply is malformed.');
  return reply;
}

export function validateReports(value: unknown): MinefieldReportsReply {
  const reply = value as MinefieldReportsReply;
  if (!reply || typeof reply !== 'object' || !count(reply.region) || !(reply.readHour === null || Number.isSafeInteger(reply.readHour)) || !strings(reply.warnings)
    || !Array.isArray(reply.reports) || !reply.reports.every((report: MineReport) => !!report && Number.isSafeInteger(report.kind) && Number.isSafeInteger(report.ordinal)
      && optional(report.victim) && optional(report.mineOwner) && optional(report.ship) && optional(report.equipment) && decimal(report.damage) && typeof report.sunk === 'boolean'))
    throw new Error('The mine reports reply is malformed.');
  return reply;
}

/** A region as a row: its name, its owners (largest first at the reply's hour), and how many owners' amounts are
 * unknown there. The amounts are live cells: (region, owner tag) each, and (region, `total`). */
export interface RegionRow { region: string; name: string; owners: { tag: string }[]; unknownOwners: number }

const regionName = (names: ReadonlyMap<string, string>, region: string) => names.get(region) ?? 'Region name unavailable';

/** The rows shown, largest total first at the reply's hour: every mined region, or with `chosen` regions only those,
 * each listed while an owner's amount there is unknown even with none known; `unlisted` counts the regions left out
 * whose mines are unknown. */
export function regionRows(reply: MinefieldsReply, chosen: readonly string[] = []): { rows: RegionRow[]; unlisted: number } {
  const names = new Map(reply.choices.map(choice => [choice.id, choice.name]));
  const wanted = chosen.length ? new Set(chosen) : null;
  const row = (region: MinedRegion): RegionRow => {
    const id = String(region.region);
    return { region: id, name: regionName(names, id), owners: region.owners.map(owner => ({ tag: owner.tag })), unknownOwners: region.unknownOwners };
  };
  const rows = reply.regions.filter(region => !wanted || wanted.has(String(region.region))).map(row);
  const unknown = reply.unknown.filter(region => !wanted || wanted.has(String(region.region)));
  if (wanted) rows.push(...unknown.map(region => row({ region: region.region, owners: [], unknownOwners: region.unknownOwners })));
  return { rows, unlisted: wanted ? 0 : unknown.length };
}

/** By name, or as the reply orders them: most mines first at its hour (a change of that order is a new reply). */
export function orderRegions(rows: readonly RegionRow[], order: 'mines' | 'name'): RegionRow[] {
  return order === 'name' ? [...rows].sort((a, b) => a.name.localeCompare(b.name) || a.region.localeCompare(b.region)) : [...rows];
}

/** Mines on the reply's scale, exactly. */
export function minesText(value: bigint, scale: number): string {
  const negative = value < 0n, magnitude = negative ? -value : value, unit = BigInt(scale);
  const fraction = (magnitude % unit).toString().padStart(String(scale).length - 1, '0').replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / unit).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}

const exact = (value: number | undefined) => value === undefined ? null : BigInt(Math.trunc(value));
/** An owner's mines in a region from the live cells; null where not known now. */
export const ownerMines = (values: LiveValues<number>, region: string, tag: string) => exact(values.get(liveKey(region, tag)));
/** A region's total from the live cells; null where an owner's amount is not known now. */
export const regionTotal = (values: LiveValues<number>, region: string) => exact(values.get(liveKey(region, 'total')));

/** A region's total on the scale: a dash where no owner's amount there is known, the reason where it is not read now,
 * never a zero. */
export function regionTotalText(row: RegionRow, values: LiveValues<number>, scale: number): string {
  if (!row.owners.length) return row.unknownOwners ? '—' : minesText(0n, scale);
  const total = regionTotal(values, row.region);
  return total === null ? values.missing(liveKey(row.region, 'total')) : minesText(total, scale);
}

/** Each owner segment's width in percent of the greatest region total now: what each frame paints. */
export function segmentWidths(rows: readonly RegionRow[], values: LiveValues<number>): Map<string, number[]> {
  const highest = rows.reduce((max, row) => { const total = regionTotal(values, row.region); return total !== null && total > max ? total : max; }, 0n);
  return new Map(rows.map(row => [row.region, row.owners.map(owner => {
    const amount = ownerMines(values, row.region, owner.tag);
    return highest > 0n && amount !== null && amount > 0n ? Number(amount * 10000n / highest) / 100 : 0;
  })]));
}

/** What a render shows: the rows in order, their owners and unknown counts. Amounts are painted. */
export function minefieldsStructure(reply: MinefieldsReply): string {
  const { rows } = regionRows(reply);
  return JSON.stringify([rows.map(row => [row.region, row.owners.map(owner => owner.tag), row.unknownOwners]), reply.unknown, reply.choices.length]);
}

/** An owner's history runs as chart runs on the reply's scale; hours with no value are left out. */
export function historyRuns(line: MineLine, scale: number): { firstHour: number; lastHour: number; value: number; breakBefore: boolean }[] {
  const out: { firstHour: number; lastHour: number; value: number; breakBefore: boolean }[] = [];
  let gap = true;
  for (const [firstHour, lastHour, value] of line.runs) {
    if (value === null) { gap = true; continue; }
    out.push({ firstHour, lastHour, value: Number(value) / scale, breakBefore: gap });
    gap = false;
  }
  return out;
}
