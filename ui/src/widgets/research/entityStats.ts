import type { EntityStats } from '../../generated/dashboard';
import { entityKind, validGameRuns } from './gameRuns.ts';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

export function validateEntityStats(value: unknown): EntityStats {
  if (!object(value) || !entityKind(value.kind) || typeof value.token !== 'string'
    || !Array.isArray(value.stats) || !value.stats.every(stat => object(stat) && validGameRuns(stat.label) && validGameRuns(stat.value) && (stat.source === null || typeof stat.source === 'string'))
    || !object(value.refusals) || !Object.values(value.refusals).every(count => Number.isSafeInteger(count))
    || !(value.unavailable === null || typeof value.unavailable === 'string')
    || !Array.isArray(value.warnings) || !value.warnings.every(line => typeof line === 'string')) throw new Error('Invalid entity stats');
  return value as unknown as EntityStats;
}

export const entityStatsStructure = (reply: EntityStats) => JSON.stringify([reply.kind, reply.token, reply.stats, reply.refusals, reply.unavailable, reply.warnings]);
