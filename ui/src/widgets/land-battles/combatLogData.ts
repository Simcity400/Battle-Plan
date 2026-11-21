import type { CombatLogReply } from '../../generated/dashboard';

const LISTS = ['groups', 'manpower', 'equipment', 'templates', 'battles', 'active', 'warnings'] as const;

export function validateCombatLog(data: unknown): CombatLogReply {
  const reply = data as CombatLogReply;
  if (!reply || !Number.isInteger(reply.hour) || !reply.availability || !LISTS.every(key => Array.isArray(reply[key]))) throw new Error('Invalid Combat Log reply');
  return reply;
}
