import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedWindow } from '../../dashboard/keyedHistory';
import type { TooltipContent } from '../../charts/tooltip';

/** The window holds each directed pair's summary fields (`OWNER|TARGET|field`), each country's
 * dated presence (`member|COUNTRY`) and its dated network count (`rank|COUNTRY`), so playback
 * picks the most active countries at its own hour. */
export interface IntelCoverageReply { window: KeyedWindow; warnings: readonly string[] }

export type Measure = 'covered' | 'strength' | 'strongest';
export type Basis = 'controlled' | 'core';
/** The recorded summary fields, by the names the backend gives them. */
export const FIELDS = ['coveredControlled', 'coveredCore', 'coveredWorth', 'controlledStrength', 'coreStrength', 'maxStrength'] as const;
export type Field = typeof FIELDS[number];

const FIXED = 100000n;

export function validateIntelCoverage(value: unknown): IntelCoverageReply {
  const reply = value as IntelCoverageReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.warnings) || !reply.warnings.every(item => typeof item === 'string'))
    throw new Error('The intel network coverage reply is malformed.');
  const window = validateKeyedWindow(reply.window);
  for (const series of window.series) {
    const [kind, part] = series.name.split('|');
    if ((kind === 'member' || kind === 'rank') && countryPart(part) === null) throw new Error('The intel network coverage reply is malformed.');
  }
  return { ...reply, window };
}

export interface ShownCountry { id: string; tag: string; slot: number }

/** The countries present at `hour`: those the explicit tags name (bare tag or TAG@slot), or
 * else the `shown` most active by their network count then, ties by identity. Null when the
 * ranking is not recorded at that hour and no tags choose the countries. */
export function countriesAt(reply: IntelCoverageReply, hour: number, shown: number, tags?: readonly string[]): ShownCountry[] | null {
  const present: string[] = [];
  const ranks = new Map<string, bigint>();
  for (const series of reply.window.series) {
    const [kind, part] = series.name.split('|');
    if (kind === 'member' && runAtHour(series.runs, hour)) present.push(part);
    else if (kind === 'rank') { const run = runAtHour(series.runs, hour); if (run) ranks.set(part, BigInt(run[2])); }
  }
  const country = (id: string): ShownCountry => ({ id, ...countryPart(id)! });
  if (tags) return present.filter(id => tags.some(tag => tag === id || tag === countryPart(id)!.tag)).sort().map(country);
  if (!ranks.size) return null;
  return present.filter(id => ranks.has(id))
    .sort((a, b) => { const left = ranks.get(a)!, right = ranks.get(b)!; return left > right ? -1 : left < right ? 1 : a < b ? -1 : a > b ? 1 : 0; })
    .slice(0, Math.max(0, shown)).map(country);
}

/** The field a measure shows on a basis. */
export function measureField(measure: Measure, basis: Basis): Field {
  if (measure === 'strongest') return 'maxStrength';
  if (measure === 'strength') return basis === 'core' ? 'coreStrength' : 'controlledStrength';
  return basis === 'core' ? 'coveredCore' : 'coveredControlled';
}

/** Every recorded summary of each directed pair at an hour, read whether or not the hour can
 * prove absence (`known`), with the countries present then. A pair with a summary of zeros is an
 * empty network; see `cellState` for what a pair without one means. */
export interface PairsAt { pairs: Map<string, Map<Field, bigint>>; known: boolean; members: ReadonlySet<string>; readHour: number | undefined }

export function pairsAt(reply: IntelCoverageReply, hour: number): PairsAt {
  const pairs = new Map<string, Map<Field, bigint>>();
  const members = new Set<string>();
  for (const series of reply.window.series) {
    const [owner, target, field] = series.name.split('|');
    if (owner === 'member') { if (runAtHour(series.runs, hour)) members.add(target); continue; }
    if (!(FIELDS as readonly string[]).includes(field)) continue;
    const run = runAtHour(series.runs, hour);
    if (!run) continue;
    const key = `${owner}|${target}`;
    const fields = pairs.get(key) ?? new Map<Field, bigint>();
    fields.set(field as Field, BigInt(run[2]));
    pairs.set(key, fields);
  }
  const known = keyedKnown(reply.window, hour);
  return { pairs, known, members, readHour: known ? keyedRead(reply.window, hour) : undefined };
}

/** One cell: a recorded summary; certified absence (no network) only where the hour proves
 * absence and both countries are present then; otherwise unknown. */
export type CellState = { kind: 'present'; fields: ReadonlyMap<Field, bigint> } | { kind: 'absent' } | { kind: 'unknown' };

export function cellState(at: PairsAt, owner: string, target: string): CellState {
  const fields = at.pairs.get(`${owner}|${target}`);
  if (fields) return { kind: 'present', fields };
  return at.known && at.members.has(owner) && at.members.has(target) ? { kind: 'absent' } : { kind: 'unknown' };
}

/** A field's value for display: strengths are the game's fixed-point values. */
export function fieldText(field: Field, value: bigint): string {
  if (field === 'coveredControlled' || field === 'coveredCore' || field === 'coveredWorth') return value.toLocaleString('en-US');
  const negative = value < 0n, magnitude = negative ? -value : value;
  const fraction = (magnitude % FIXED).toString().padStart(5, '0').slice(0, 2).replace(/0+$/u, '');
  return `${negative ? '−' : ''}${(magnitude / FIXED).toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`;
}

/** The chart operand of a value: its display magnitude, for colour only. */
export const fieldNumber = (field: Field, value: bigint) => field.endsWith('Strength') ? Number(value) / Number(FIXED) : Number(value);

/** The shown countries on the axes the matrix draws (both axes list the same countries). */
export function visibleCountries<C extends { id: string }>(countries: readonly C[], visible: ReadonlySet<string>): C[] {
  return countries.filter(country => visible.has(country.id));
}

export const LABELS: Record<Field, string> = {
  coveredControlled: 'Controlled states covered', coveredCore: 'Core states covered', coveredWorth: 'States worth covering covered',
  controlledStrength: 'Strength over controlled states', coreStrength: 'Strength over core states', maxStrength: 'Strongest state',
};

export function pairTooltip(owner: string, target: string, cell: CellState, readDate: string | null): TooltipContent {
  const fields = cell.kind === 'present' ? cell.fields : undefined;
  return {
    title: `${owner} → ${target}`,
    rows: cell.kind === 'unknown' ? [{ label: 'Network', value: 'Not recorded' }]
      : !fields ? [{ label: 'Network', value: 'No network' }]
      : [...FIELDS.filter(field => field !== 'coveredWorth').map(field => ({ label: LABELS[field], value: fields.has(field) ? fieldText(field, fields.get(field)!) : 'Not recorded' })),
        ...readDate ? [{ label: 'As of', value: readDate }] : []],
  };
}
