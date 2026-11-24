import { isOrderedSpans } from '../../charts/series.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { ArmyBreakdownReply, ArmyMeasure } from '../../generated/dashboard';

export const MEASURES = [
  { value: 'manpower', label: 'Manpower in the field' },
  { value: 'divisions', label: 'Number of divisions' },
  { value: 'strength', label: 'Combined strength (hit points)' },
  { value: 'battalions', label: 'Battalions' },
] as const satisfies readonly { value: ArmyMeasure; label: string }[];

export interface ArmyMetadataReply {
  readonly branchId: string;
  readonly available: boolean;
  readonly measures: readonly ArmyMeasure[];
  readonly warnings: readonly string[];
}

export function armyMeasure(value: unknown, measures: readonly ArmyMeasure[] | undefined): ArmyMeasure {
  if (typeof value === 'string' && measures?.includes(value as ArmyMeasure)) return value as ArmyMeasure;
  return MEASURES.find(item => measures?.includes(item.value))?.value ?? 'divisions';
}

export function measureChoices(reply: Pick<ArmyMetadataReply, 'measures'> | undefined): { value: string; label: string }[] {
  return MEASURES.filter(item => !!reply && reply.measures.includes(item.value)).map(item => ({ ...item }));
}

export const measureName = (measure: ArmyMeasure) => MEASURES.find(item => item.value === measure)!.label;

export function armyGroupLabel(type: { readonly key: string; readonly name?: string | null; readonly unresolved?: boolean }): string {
  const isUnknown = type.key.startsWith('unknown:');
  const trimmed = type.name?.trim() ?? '';
  if (!isUnknown && !type.unresolved && trimmed.length > 0 && trimmed !== 'Unit group name unavailable') {
    return trimmed;
  }
  const raw = type.key.replace(/^(group|unknown):/, '') || type.key;
  return isUnknown ? `${raw} (unknown unit group)` : `${raw} (name unavailable)`;
}

export const armyMetadataParams = (branchId: string) => ({
  branchId,
  countries: [] as string[],
  top: null,
  measure: 'divisions' as ArmyMeasure,
  countFor: 'owner' as const,
  byType: false,
  window: undefined,
});

export function validateArmyMetadata(value: unknown): ArmyMetadataReply {
  if (!object(value) || typeof value.branchId !== 'string' || typeof value.available !== 'boolean'
    || !(value.window === null || (int(value.window) && value.window >= 0))
    || !Array.isArray(value.measures) || !value.measures.every(measure => MEASURES.some(item => item.value === measure))
    || new Set(value.measures).size !== value.measures.length
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The army size reply is malformed.');
  }
  return value as unknown as ArmyMetadataReply;
}

export function validateArmyBreakdownReply(value: unknown): ArmyBreakdownReply {
  const ok = object(value)
    && int(value.hour)
    && (value.nextChange === null || (int(value.nextChange) && (value.nextChange as number) > (value.hour as number)))
    && typeof value.available === 'boolean'
    && Array.isArray(value.types)
    && value.types.every(item => object(item) && typeof item.key === 'string' && typeof item.name === 'string')
    && Array.isArray(value.runs)
    && value.runs.every(run => Array.isArray(run) && run.length >= 3 && int(run[0])
      && (run[1] === null || (typeof run[1] === 'number' && Number.isFinite(run[1])))
      && (run[2] === null || (int(run[2]) && (run[2] as number) >= (run[0] as number)))
      && ((run[2] as number | null) ?? (run[0] as number)) <= (value.hour as number))
    && Array.isArray(value.mix)
    && isOrderedSpans(value.mix)
    && value.mix.every(run => {
      const vals = (run as Record<string, unknown>).values;
      return run.lastHour <= (value.hour as number) && Array.isArray(vals) && vals.length === (value.types as unknown[]).length;
    })
    && Array.isArray(value.missing)
    && value.missing.every(span => object(span) && int(span.firstHour) && int(span.lastHour)
      && (span.firstHour as number) <= (span.lastHour as number)
      && (span.lastHour as number) <= (value.hour as number)
      && typeof span.reason === 'string')
    && Array.isArray(value.warnings)
    && value.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The army breakdown reply is malformed.');
  return value as unknown as ArmyBreakdownReply;
}

export function armyBreakdownProof(data: unknown, _requested: readonly [number, number], at: number): readonly [number, number] | null {
  const reply = data as ArmyBreakdownReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= at && at <= last ? [reply.hour, last] : null;
}
