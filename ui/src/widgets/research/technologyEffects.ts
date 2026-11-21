import type { TechnologyEffects } from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

export function validateTechnologyEffects(value: unknown): TechnologyEffects {
  if (!object(value) || !Number.isSafeInteger(value.hour) || typeof value.text !== 'string' || !object(value.refusals)
    || !Object.values(value.refusals).every(count => Number.isSafeInteger(count)) || !(value.unavailable === null || typeof value.unavailable === 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(line => typeof line === 'string')) throw new Error('Invalid technology effects');
  return value as unknown as TechnologyEffects;
}

export const technologyEffectsStructure = (reply: TechnologyEffects) => JSON.stringify([reply.text, reply.refusals, reply.unavailable, reply.warnings]);
