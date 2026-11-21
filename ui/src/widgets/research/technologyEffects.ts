import type { TechnologyEffects } from '../../generated/dashboard';
import { validGameRuns } from './gameRuns.ts';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

export function validateTechnologyEffects(value: unknown): TechnologyEffects {
  if (!object(value) || !Number.isSafeInteger(value.hour) || !validGameRuns(value.runs) || !object(value.refusals)
    || !Object.values(value.refusals).every(count => Number.isSafeInteger(count)) || !(value.unavailable === null || typeof value.unavailable === 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(line => typeof line === 'string')) throw new Error('Invalid technology effects');
  return value as unknown as TechnologyEffects;
}

export const technologyEffectsStructure = (reply: TechnologyEffects) => JSON.stringify([reply.runs, reply.refusals, reply.unavailable, reply.warnings]);
