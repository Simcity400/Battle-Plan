import { int, object } from '../../dashboard/validation.ts';
import type { ArmyMeasure } from '../../generated/dashboard';

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

export const measureName = (measure: ArmyMeasure) => MEASURES.find(item => item.value === measure)!.label;

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
