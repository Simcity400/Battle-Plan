import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { MatrixCell } from '../../charts/heat/matrix';
import type { TooltipContent } from '../../charts/tooltip';
import type { Country, IntelCoverageReply } from '../../generated/dashboard';


export type Measure = 'covered' | 'strength' | 'strongest';
export type Basis = 'controlled' | 'core';
/** The recorded summary fields the widget draws, by the names the product and the `intel-networks` series give them. */
export const FIELDS = ['coveredControlled', 'coveredCore', 'controlledStrength', 'coreStrength', 'maxStrength'] as const;
export type Field = typeof FIELDS[number];

const FIXED = 100000n;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const country = (value: unknown) => object(value) && Number.isSafeInteger(value.id) && (value.tag === null || typeof value.tag === 'string' && value.tag.length > 0);

export function validateIntelCoverage(value: unknown): IntelCoverageReply {
  const ok = object(value) && (value.missing === null || typeof value.missing === 'string') && (value.readHour === null || Number.isSafeInteger(value.readHour))
    && Array.isArray(value.countries) && value.countries.every(item => country(item) && (item as Country).tag !== null)
    && Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string')
    && Array.isArray(value.networks) && value.networks.every(item => object(item) && country(item.owner) && country(item.target));
  if (!ok) throw new Error('The intel network coverage reply is malformed.');
  return value as unknown as IntelCoverageReply;
}

/** What renders: the countries and which pairs hold a network, not their values nor the daily read hour. */
export const coverageStructure = (reply: IntelCoverageReply) => JSON.stringify([reply.missing, reply.countries, reply.networks, reply.warnings]);

/** The hour's networks by their countries' tags (`OWNER|TARGET`), each its live cells' row; a side no tag names is not
 * on the matrix. */
export interface PairsAt { rows: ReadonlyMap<string, string>; present: ReadonlySet<string>; ids: ReadonlyMap<string, number>; known: boolean }

export function pairsAt(reply: IntelCoverageReply): PairsAt {
  const rows = new Map<string, string>();
  for (const network of reply.networks) {
    if (network.owner.tag !== null && network.target.tag !== null) rows.set(`${network.owner.tag}|${network.target.tag}`, `${network.owner.id}:${network.target.id}`);
  }
  const tags = reply.countries.map(item => item.tag!);
  return { rows, present: new Set(tags), ids: new Map(reply.countries.map(item => [item.tag!, item.id])), known: reply.missing === null };
}

/** The countries on the matrix: those `matches` keeps (in tag order), else the `shown` most active by their count of
 * networks in or from them, ties by tag; with `touching`, after them every country with a network in or from one of
 * them, in tag order. */
export function countriesAt(reply: IntelCoverageReply, shown: number, matches?: (tag: string) => boolean, touching = false): string[] {
  const tags = reply.countries.map(item => item.tag!);
  const counts = new Map<string, number>(tags.map(tag => [tag, 0]));
  for (const network of reply.networks) for (const side of [network.owner.tag, network.target.tag]) {
    if (side !== null && counts.has(side)) counts.set(side, counts.get(side)! + 1);
  }
  const own = matches ? tags.filter(matches)
    : [...counts].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)).slice(0, Math.max(0, shown)).map(([tag]) => tag);
  if (!touching || !own.length) return own;
  const kept = new Set(own);
  const linked = new Set<string>();
  for (const { owner, target } of reply.networks) {
    if (owner.tag === null || target.tag === null) continue;
    if (kept.has(owner.tag)) linked.add(target.tag);
    if (kept.has(target.tag)) linked.add(owner.tag);
  }
  return [...own, ...tags.filter(tag => !kept.has(tag) && linked.has(tag))];
}

/** The field a measure shows on a basis. */
export function measureField(measure: Measure, basis: Basis): Field {
  if (measure === 'strongest') return 'maxStrength';
  if (measure === 'strength') return basis === 'core' ? 'coreStrength' : 'controlledStrength';
  return basis === 'core' ? 'coveredCore' : 'coveredControlled';
}

export const isStrength = (field: Field) => field.endsWith('Strength');

/** A field's value for display: strengths are the game's fixed-point values. */
export function fieldText(field: Field, value: number): string {
  if (!isStrength(field)) return value.toLocaleString('en-US');
  const raw = BigInt(value);
  const negative = raw < 0n, magnitude = negative ? -raw : raw;
  const fraction = (magnitude % FIXED).toString().padStart(5, '0').slice(0, 2).replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / FIXED).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}

/** The chart operand of a value: its display magnitude, for colour only. */
export const fieldNumber = (field: Field, value: number) => isStrength(field) ? value / Number(FIXED) : value;

/** One cell: a network and its values as read (each undefined where not read, with why), no network (proven only where
 * the hour is known and both countries present), or unknown. */
export type CellState = { kind: 'present'; values: ReadonlyMap<Field, number | string> } | { kind: 'absent' } | { kind: 'unknown' };

export function cellState(at: PairsAt, values: LiveValues<number>, owner: string, target: string): CellState {
  const row = at.rows.get(`${owner}|${target}`);
  if (row !== undefined) {
    return { kind: 'present', values: new Map(FIELDS.map(field => {
      const value = values.get(liveKey(row, field));
      return [field, value ?? values.missing(liveKey(row, field))];
    })) };
  }
  return at.known && at.present.has(owner) && at.present.has(target) ? { kind: 'absent' } : { kind: 'unknown' };
}

/** The matrix's cells at the values read now, keyed `liveKey(owner, target)`: each network coloured by the chosen
 * field against the largest shown, none on the diagonal or where no network is proven, the unknown hatch otherwise. */
export function matrixCells(at: PairsAt, values: LiveValues<number>, field: Field, colour: (value: number, highest: number) => string): LiveValues<MatrixCell> {
  let highest = 0;
  for (const row of at.rows.values()) {
    const value = values.get(liveKey(row, field));
    if (value !== undefined) highest = Math.max(highest, fieldNumber(field, value));
  }
  return {
    get: key => {
      const [owner, target] = key.split('\u0000');
      if (owner === target) return null;
      const row = at.rows.get(`${owner}|${target}`);
      if (row === undefined) return at.known && at.present.has(owner) && at.present.has(target) ? null : undefined;
      const value = values.get(liveKey(row, field));
      return value === undefined ? undefined : { color: colour(fieldNumber(field, value), highest) };
    },
    missing: () => 'Not recorded',
  };
}

export const LABELS: Record<Field, string> = {
  coveredControlled: 'Controlled states covered', coveredCore: 'Core states covered',
  controlledStrength: 'Strength over controlled states', coreStrength: 'Strength over core states', maxStrength: 'Strongest state',
};

export function pairTooltip(owner: string, target: string, cell: CellState, readDate: string | null): TooltipContent {
  const values = cell.kind === 'present' ? cell.values : undefined;
  return {
    title: `${owner} → ${target}`,
    rows: cell.kind === 'unknown' ? [{ label: 'Network', value: 'Not recorded' }]
      : !values ? [{ label: 'Network', value: 'No network' }]
      : [...FIELDS.map(field => {
        const value = values.get(field);
        return { label: LABELS[field], value: typeof value === 'number' ? fieldText(field, value) : `Not recorded${value ? ` (${value})` : ''}` };
      }), ...readDate ? [{ label: 'As of', value: readDate }] : []],
  };
}

/** A pinned pair's saved owner and target: `TAG|id` each, the target's raw id being its line's part. */
export interface ChosenPair { owner: string; target: string; targetId: number; hour: number }
export const pairToken = (pair: ChosenPair) => `${pair.owner}|${pair.target}|${pair.targetId}|${pair.hour}`;
export function pairOf(owner: unknown, target: unknown): ChosenPair | null {
  if (typeof owner !== 'string' || typeof target !== 'string' || !owner || !target) return null;
  const [ownerTag, hour] = owner.split('|');
  const [targetTag, id] = target.split('|');
  const targetId = Number(id), at = Number(hour);
  return ownerTag && targetTag && Number.isSafeInteger(targetId) && Number.isSafeInteger(at) ? { owner: ownerTag, target: targetTag, targetId, hour: at } : null;
}
/** The pinned options of `pair`: the owner with the hour it was chosen at, the target with its raw id. */
export const pairOptions = (pair: ChosenPair) => ({ owner: `${pair.owner}|${pair.hour}`, target: `${pair.target}|${pair.targetId}` });
