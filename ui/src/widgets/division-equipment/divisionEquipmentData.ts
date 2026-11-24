import type { DivisionEquipmentReply } from '../../generated/dashboard';

const isArray = Array.isArray;
export function validateDivisionEquipment(value: unknown): DivisionEquipmentReply {
  const reply = value as DivisionEquipmentReply;
  if (!reply || typeof reply.tag !== 'string' || (reply.missing !== null && typeof reply.missing !== 'string')
    || !isArray(reply.types) || !isArray(reply.templates) || !isArray(reply.warnings)
    || reply.types.some(row => (row.archetype !== null && typeof row.archetype !== 'string') || (row.heldRaw !== null && typeof row.heldRaw !== 'string') || !isArray(row.variants)))
    throw new Error('Invalid division equipment reply');
  return reply;
}
