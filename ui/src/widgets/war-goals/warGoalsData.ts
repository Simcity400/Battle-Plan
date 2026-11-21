import { object, signed32, unsigned32, signed64, unsigned64, opaqueTag } from '../../dashboard/validation.ts';
import type { WargoalsReply } from '../../generated/dashboard';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';

const text = (v: unknown): v is string => typeof v === 'string';
const list = (v: unknown, check: (v: unknown) => boolean): v is unknown[] => Array.isArray(v) && v.every(check);
export function validateWargoalsReply(v: unknown): WargoalsReply {
  if (!object(v) || !unsigned64(v.branchId) || !opaqueTag(v.tag) || !list(v.warnings, text) || !list(v.stateNames, r => object(r) && unsigned32(r.id) && text(r.name) && typeof r.known === 'boolean')
    || !list(v.standings, r => object(r) && opaqueTag(r.actor) && opaqueTag(r.recipient) && (r.standing === null || r.standing === 'atWar' || r.standing === 'sameFaction'))
    || !list(v.goals, r => object(r) && unsigned64(r.handle) && text(r.name) && opaqueTag(r.actor) && opaqueTag(r.recipient) && typeof r.available === 'boolean'
      && (r.expires === null || signed32(r.expires)) && list(r.states, unsigned32) && list(r.liberate, opaqueTag) && list(r.puppets, opaqueTag))
    || !list(v.justifications, r => object(r) && unsigned64(r.id) && unsigned32(r.ordinal) && text(r.name) && opaqueTag(r.actor) && opaqueTag(r.recipient) && list(r.states, unsigned32)
      && signed64(r.total) && signed64(r.daily) && typeof r.payUpfront === 'boolean' && (r.start === null || signed32(r.start)) && signed32(r.completion)
      && (r.paused === null || typeof r.paused === 'boolean'))) throw new Error('Invalid war-goal inventory reply');
  return v as unknown as WargoalsReply;
}
export function fixed5(raw: string): string {
  const value = BigInt(raw); const sign = value < 0n ? '-' : ''; const positive = value < 0n ? -value : value;
  const tail = (positive % 100000n).toString().padStart(5, '0').replace(/0+$/, '');
  return sign + (positive / 100000n).toString() + (tail ? '.' + tail : '');
}

export function wargoalStructure(reply: WargoalsReply): string {
  return JSON.stringify([reply.branchId, reply.tag, reply.standings, reply.goals, reply.stateNames, reply.warnings,
    reply.justifications.map(({ completion: _completion, ...fixed }) => fixed)]);
}

export function progressText(points: number | undefined, total: string, missing: string): { share: number | null; shareText: string; text: string } {
  if (points === undefined) return { share: null, shareText: '', text: `${missing} / ${fixed5(total)}` };
  const done = BigInt(Math.trunc(points)), cost = BigInt(total);
  const share = cost > 0n ? Number(((done < 0n ? 0n : done > cost ? cost : done) * 1000n) / cost) / 10 : null;
  return { share, shareText: share != null ? `${share}%` : '', text: `${share != null ? ' · ' : ''}${fixed5(done.toString())} / ${fixed5(total)}` };
}

export function expiryText(expires: number, hour: number): string {
  const days = Math.ceil((expires - hour) / 24);
  return expires - hour > 0 ? `Expires ${dayLabel(expires)} · ${usNumber.format(days)} ${days === 1 ? 'day' : 'days'}` : `Expiry date passed ${dayLabel(expires)}`;
}
