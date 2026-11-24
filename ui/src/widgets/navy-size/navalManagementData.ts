import type { NavalManagementAmount, NavalManagementReply } from '../../generated/dashboard';

const text = (value: unknown): value is string => typeof value === 'string';
const optional = (value: unknown) => value === null || (text(value) && /^-?\d+$/u.test(value));
const amount = (value: unknown) => {
  const held = value as NavalManagementAmount | null;
  return held === null || (!!held && text(held.row) && text(held.column) && optional(held.raw) && optional(held.of) && text(held.unit));
};

export function validateNavalManagement(value: unknown): NavalManagementReply {
  const reply = value as NavalManagementReply;
  if (!reply || !Array.isArray(reply.warnings) || reply.warnings.some(line => !text(line))
    || !Array.isArray(reply.sections) || reply.sections.some(section => section == null || !text(section.key) || !text(section.title)
      || !Array.isArray(section.lines) || section.lines.some(line => !line || !text(line.text) || !amount(line.amount)))) {
    throw Error('The naval management reply is malformed.');
  }
  return reply;
}

export function fixedText(raw: string | number): string {
  const value = BigInt(typeof raw === 'number' ? Math.trunc(raw) : raw), negative = value < 0n, magnitude = negative ? -value : value;
  return `${negative ? '-' : ''}${magnitude / 100000n}.${(magnitude % 100000n).toString().padStart(5, '0')}`;
}

export function formatNavalAmount(held: NavalManagementAmount): string {
  const value = held.raw == null ? 'Unavailable' : fixedText(held.raw);
  return held.of !== null ? `${value} / ${fixedText(held.of)}` : `${value} ${held.unit}`;
}
