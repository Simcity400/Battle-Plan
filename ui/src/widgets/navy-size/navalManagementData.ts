import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { NavalManagementAmount, NavalManagementReply } from '../../generated/dashboard';

const text = (value: unknown): value is string => typeof value === 'string';
const optional = (value: unknown) => value === null || text(value) && /^-?\d+$/u.test(value);
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

export const managementStructure = (reply: NavalManagementReply) => JSON.stringify([reply.sections.map(section => [section.key, section.title,
  section.lines.map(line => [line.text, line.amount && [line.amount.row, line.amount.column, line.amount.of, line.amount.unit]])]), reply.warnings]);

export const amountKey = (amount: NavalManagementAmount) => liveKey(amount.row, amount.column);

export function fixedText(raw: string | number): string {
  const value = BigInt(typeof raw === 'number' ? Math.trunc(raw) : raw), negative = value < 0n, magnitude = negative ? -value : value;
  return `${negative ? '-' : ''}${magnitude / 100000n}.${(magnitude % 100000n).toString().padStart(5, '0')}`;
}

export function amountText(values: LiveValues<number>, key: string, reply: NavalManagementReply | undefined): string {
  const held = reply?.sections.flatMap(section => section.lines).find(line => line.amount && amountKey(line.amount) === key)?.amount;
  if (!held) return '';
  const live = values.get(key);
  const raw = live ?? held.raw;
  const value = raw == null ? 'Unavailable' : fixedText(raw);
  return held.of !== null ? `${value} / ${fixedText(held.of)}` : `${value} ${held.unit}`;
}
