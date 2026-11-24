import { object, int } from '../../dashboard/validation.ts';
import { isEngineHour } from '../../dashboard/calendar.ts';
import type { AirAce, AirAcesReply } from '../../generated/dashboard';

const text = (v: unknown) => v === null || typeof v === 'string';
const handle = (v: unknown): v is string => typeof v === 'string' && /^\d+$/.test(v) && BigInt(v) <= 0xffff_ffff_ffff_ffffn;
const hour = (v: unknown) => int(v) && isEngineHour(v);
const nullableHour = (v: unknown) => v === null || hour(v);
const warnings = (v: unknown) => Array.isArray(v) && v.every(item => typeof item === 'string');
const invalid = (): never => { throw new Error('Invalid ace roster reply'); };

function validAce(v: unknown): boolean {
  if (!object(v) || !handle(v.id) || !text(v.tag) || !text(v.name) || !text(v.callsign) || !(v.wing === null || handle(v.wing)) || !text(v.wingName)
    || typeof v.alive !== 'boolean' || !nullableHour(v.firstSeen) || !text(v.portrait) || !text(v.portraitMissing) || (v.portrait === null) === (v.portraitMissing === null)) return false;
  if (!(v.aircraft === null || Array.isArray(v.aircraft) && v.aircraft.every(plane => object(plane) && handle(plane.id) && text(plane.name) && text(plane.missing)
    && (plane.name === null) !== (plane.missing === null)))) return false;
  if (v.alive) return v.death === null;
  const d = v.death;
  return object(d) && nullableHour(d.lastAlive) && nullableHour(d.byHour)
    && ((d.lastAlive === null && d.byHour === null) || (int(d.lastAlive) && int(d.byHour) && d.lastAlive < d.byHour))
    && (d.killerAce === null || handle(d.killerAce)) && text(d.killerName) && text(d.killerTag);
}

export function validateAces(v: unknown): AirAcesReply {
  if (!object(v) || !nullableHour(v.readHour) || !text(v.missing) || !Array.isArray(v.aces) || !warnings(v.warnings)
    || (v.missing !== null && v.aces.length > 0)) return invalid();
  const ids = new Set<string>();
  for (const ace of v.aces) {
    if (!validAce(ace) || ids.has((ace as AirAce).id)) return invalid();
    ids.add((ace as AirAce).id);
  }
  return v as unknown as AirAcesReply;
}
