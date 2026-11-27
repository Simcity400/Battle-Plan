import type { HourSpan } from '../../generated/dashboard';
import type { HeldPart, KeyedState } from '../../dashboard/keyedSeriesData';
import type { MeasureParts, StabilityPart, StabilityPartName, StabilityPartNamesReply } from './types';
import { lineName, termLabel } from './stabilityData.ts';
import type { Measure } from './stabilityData';

export const REMAINDER_LABEL = 'Other';
export const SMALLER_LABEL = 'Smaller parts';

export function engineSum(raws: readonly string[]): bigint {
  return raws.reduce((total, raw) => BigInt.asIntN(64, total + BigInt(raw)), 0n);
}

export function partName(part: StabilityPart, measure: Measure): { name: string; recorded: boolean } {
  if (part.line) return lineName(part.line);
  if (part.term) return { name: termLabel(measure, part.term), recorded: false };
  if (part.id === 'clamp') return { name: 'Limited to the allowed range', recorded: false };
  return { name: REMAINDER_LABEL, recorded: false };
}

export function partsWithin(parts: MeasureParts, spans: readonly HourSpan[] | null): MeasureParts {
  if (!spans) return parts;
  const known = [...parts.observed].sort((a, b) => a.firstHour - b.firstHour);
  const shown = [...spans].sort((a, b) => a.firstHour - b.firstHour);
  const observed: HourSpan[] = [];
  let i = 0;
  let j = 0;
  while (i < known.length && j < shown.length) {
    const firstHour = Math.max(known[i].firstHour, shown[j].firstHour);
    const lastHour = Math.min(known[i].lastHour, shown[j].lastHour);
    if (firstHour <= lastHour) observed.push({ firstHour, lastHour });
    if (known[i].lastHour < shown[j].lastHour) i++; else j++;
  }
  return { ...parts, observed };
}

export interface PartLayer { id: string; name: string; color: string; members: readonly string[] }

export function partLayers(parts: MeasureParts, measure: Measure, palette: readonly string[], grey: string, exactWeights?: ReadonlyMap<string, bigint>): PartLayer[] {
  const observed = [...parts.observed].sort((a, b) => a.firstHour - b.firstHour);
  const before: number[] = [];
  let total = 0;
  for (const span of observed) { before.push(total); total += span.lastHour - span.firstHour + 1; }
  const shownThrough = (hour: number) => {
    let low = 0;
    let high = observed.length - 1;
    let found = -1;
    while (low <= high) { const middle = (low + high) >> 1; if (observed[middle].firstHour <= hour) { found = middle; low = middle + 1; } else high = middle - 1; }
    return found < 0 ? 0 : before[found] + Math.min(hour, observed[found].lastHour) - observed[found].firstHour + 1;
  };
  const weight = (part: StabilityPart) => part.runs.reduce((sum, run) =>
    sum + (BigInt(run.valueRaw) < 0n ? -BigInt(run.valueRaw) : BigInt(run.valueRaw)) * BigInt(shownThrough(run.lastHour) - shownThrough(run.firstHour - 1)), 0n);
  const weights = exactWeights ?? new Map(parts.parts.map(part => [part.id, weight(part)]));
  const ranked = parts.parts.filter(part => (weights.get(part.id) ?? 0n) > 0n)
    .sort((a, b) => weights.get(a.id)! === weights.get(b.id)! ? a.id.localeCompare(b.id) : weights.get(a.id)! > weights.get(b.id)! ? -1 : 1);
  const room = Math.min(ranked.length, palette.length);
  const layers: PartLayer[] = ranked.slice(0, room).map((part, index) => ({ id: part.id, name: partName(part, measure).name, color: palette[index], members: [part.id] }));
  const rest = ranked.slice(room);
  if (rest.length) layers.push({ id: '__smaller', name: rest.length === 1 ? partName(rest[0], measure).name : SMALLER_LABEL, color: grey, members: rest.map(part => part.id) });
  return layers;
}

export function measureOf(part: string): Measure {
  return BigInt(part) >> 32n === 1n ? 'warSupport' : 'stability';
}

const nameOf = (part: string, names: ReadonlyMap<string, StabilityPartName>) => names.get(part);
const partOf = (part: string, names: ReadonlyMap<string, StabilityPartName>): StabilityPart => {
  const name = nameOf(part, names);
  return { id: name?.id ?? `part:${part}`, term: name?.term ?? null, line: name?.line ?? null, runs: [] };
};
const isTotal = (part: string, names: ReadonlyMap<string, StabilityPartName>) => nameOf(part, names)?.kind === 'total';

export function partsAtStates(parts: readonly string[], states: readonly KeyedState[], hour: number,
  names: ReadonlyMap<string, StabilityPartName>): MeasureParts {
  const at = new Map(states.filter(state => state.hour === hour).map(state => [state.part, state]));
  const known = parts.some(part => isTotal(part, names) && at.get(part)?.run);
  if (!known) return { parts: [], observed: [] };
  const byId = new Map<string, StabilityPart>();
  for (const part of parts) {
    const run = at.get(part)?.run;
    if (!run || isTotal(part, names)) continue;
    const made = partOf(part, names);
    const held = byId.get(made.id);
    if (held) held.runs[0].valueRaw = String(BigInt.asIntN(64, BigInt(held.runs[0].valueRaw) + BigInt(run.valueRaw)));
    else byId.set(made.id, { ...made, runs: [{ firstHour: hour, lastHour: hour, valueRaw: run.valueRaw, startsSegment: false }] });
  }
  return { parts: [...byId.values()], observed: [{ firstHour: hour, lastHour: hour }] };
}

const text = (value: unknown) => value === null || typeof value === 'string';
export function validateStabilityPartNames(value: unknown): StabilityPartNamesReply {
  const reply = value as StabilityPartNamesReply;
  const ok = typeof reply === 'object' && reply !== null && typeof reply.tag === 'string' && Number.isSafeInteger(reply.hour)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string') && Array.isArray(reply.parts)
    && reply.parts.every(part => typeof part === 'object' && part !== null && typeof part.part === 'string' && /^\d+$/.test(part.part)
      && (part.measure === 'stability' || part.measure === 'warSupport') && typeof part.kind === 'string' && text(part.id) && text(part.term)
      && text(part.missing) && (part.line === null || typeof part.line === 'object'));
  if (!ok) throw new Error('The stability part names reply is malformed.');
  return reply;
}

export function namesAt(listed: readonly HeldPart[], end: number): [number, string[]][] {
  const last = new Map<string, number>();
  for (const part of listed) for (const span of part.spans) if (span.firstHour <= end) last.set(part.part, Math.max(last.get(part.part) ?? -Infinity, Math.min(span.lastHour, end)));
  const byHour = new Map<number, string[]>();
  for (const [part, hour] of last) byHour.set(hour, [...(byHour.get(hour) ?? []), part]);
  return [...byHour].sort(([a], [b]) => a - b);
}
