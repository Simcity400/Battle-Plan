import { object } from '../../dashboard/validation.ts';
import type { FactionGoalRewardsReply } from '../../generated/dashboard';

const text = (value: unknown) => value === null || typeof value === 'string';

export function validateFactionGoalRewards(value: unknown): FactionGoalRewardsReply {
  if (!object(value) || typeof value.goal !== 'string' || !text(value.text) || !text(value.reason)
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The faction goal rewards reply is malformed.');
  return value as unknown as FactionGoalRewardsReply;
}
