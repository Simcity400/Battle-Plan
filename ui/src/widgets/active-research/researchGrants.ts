import type { GrantInventory } from '../../generated/dashboard.ts';

export function researchGrants(value: unknown): GrantInventory {
  const data = value as GrantInventory;
  const integer = (value: unknown) => Number.isInteger(value) && Number(value) >= -2147483648 && Number(value) <= 2147483647;
  const raw = (value: unknown) => typeof value === 'string' && /^-?(0|[1-9][0-9]{0,18})$/.test(value)
    && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n);
  const name = (value: unknown) => value === null || typeof value === 'string';
  const restrictions = (value: unknown) => Array.isArray(value) && value.every(row => row && typeof row.token === 'string'
    && name(row.name) && (row.databaseIndex === null || integer(row.databaseIndex)));
  if (!data || !(data.nextId === null || integer(data.nextId)) || !name(data.unavailable) || !Array.isArray(data.grants)
    || !data.grants.every(row => row && integer(row.id) && Number.isInteger(row.ordinal) && row.ordinal >= 0 && row.ordinal <= 0xffffffff
      && (row.kind === 'cost' || row.kind === 'speed') && typeof row.token === 'string' && name(row.name)
      && integer(row.uses) && (row.activeClaims === null || integer(row.activeClaims))
      && [row.speedBonusRaw, row.aheadReductionRaw, row.costReductionRaw].every(value => value === null || raw(value))
      && restrictions(row.technologies) && restrictions(row.categories))) throw new Error('Invalid research grant inventory.');
  return data;
}
