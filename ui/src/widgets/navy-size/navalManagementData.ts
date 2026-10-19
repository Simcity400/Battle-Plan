import type { NavalManagementReply } from '../../generated/dashboard';

export function validateNavalManagement(value: unknown): NavalManagementReply {
  const reply = value as NavalManagementReply;
  if (!reply || typeof reply.branchId !== 'string' || !Number.isSafeInteger(reply.hour) || reply.hour < -2147483648 || reply.hour > 2147483647
    || !Array.isArray(reply.warnings) || reply.warnings.some(line => typeof line !== 'string')
    || !Array.isArray(reply.sections) || reply.sections.some(section => section == null || typeof section.key !== 'string'
      || typeof section.title !== 'string' || !Array.isArray(section.lines) || section.lines.some(line => typeof line !== 'string'))) {
    throw Error('The naval management reply is malformed.');
  }
  return reply;
}
