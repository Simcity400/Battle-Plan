import type { PlaceNamesReply } from '../generated/dashboard';
import { object, unsigned32, signed32 } from './validation.ts';

const uint = unsigned32;
const hour = signed32;
const nullableText = (value: unknown) => value === null || typeof value === 'string';

export function validatePlaceNamesReply(v: unknown): PlaceNamesReply {
  if (!object(v) || !Array.isArray(v.places) || !v.places.every(p => object(p) && uint(p.provinceId) && hour(p.hour) && nullableText(p.name) && nullableText(p.missing))
    || new Set(v.places.map(p => placeKey(p.provinceId, p.hour))).size !== v.places.length
    || !Array.isArray(v.states) || !v.states.every(s => object(s) && uint(s.stateId) && hour(s.hour)
      && (s.name === null ? typeof s.missing === 'string' && s.missing.length > 0 : typeof s.name === 'string' && s.name.length > 0)
      && nullableText(s.missing))
    || new Set(v.states.map(s => placeKey(s.stateId, s.hour))).size !== v.states.length
    || !Array.isArray(v.warnings) || !v.warnings.every(w => typeof w === 'string')) throw new Error('The place names reply is malformed.');
  return v as unknown as PlaceNamesReply;
}
export const placeKey = (provinceId: number, hour: number) => JSON.stringify([provinceId, hour]);
